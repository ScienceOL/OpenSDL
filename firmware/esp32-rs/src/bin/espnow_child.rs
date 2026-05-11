//! ESP-NOW child — broadcasts telemetry + REG, listens for commands
//! addressed to its MAC, bridges those commands to the on-board RS-485
//! transceiver, forwards any RS-485 replies back via ESP-NOW, and
//! mirrors traffic on the on-board ST7796 LCD.
//!
//! Inbound ESP-NOW frame format: [dst_mac(6) | payload(...)]
//! Child filters by checking dst_mac matches its own MAC.
//!
//! RS-485 bridge:
//!   * LilyGO T-Connect Pro has a built-in TD501D485H-A transceiver wired to
//!     GPIO17 (TX) / GPIO18 (RX). No external MAX485 needed — connect the
//!     Runze pump's RS-485 A/B lines (+ GND) directly to the T-Connect's
//!     RS-485 header.
//!   * For loopback testing without a real device, jumper GPIO17 ↔ GPIO18
//!     (or connect the RS-485 A/B headers of two T-Connect boards).
//!   * 9600 baud, 8N1 (Runze SY-03B default).
//!   * RX frames are coalesced with a 50 ms idle timeout — same scheme as
//!     `DirectSerialTransport` on the Mac side — then broadcast to the
//!     gateway with the usual dst_mac = broadcast header.
//!
//! LilyGO T-Connect Pro ST7796 pins (SPI2, no MISO):
//!   SCLK=12  MOSI=11  CS=21  DC=41  BL=46  RST=none (tied to EN)

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::espnow::{EspNow, PeerInfo};
use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::{config::Config as SpiConfig, config::DriverConfig, Dma, SpiDeviceDriver};
use esp_idf_svc::hal::uart::{config::Config as UartConfig, UartDriver};
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{esp_mac_type_t_ESP_MAC_WIFI_STA, esp_read_mac};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};

use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};

use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7796;
use mipidsi::options::{ColorInversion, Orientation};
use mipidsi::Builder;

const BROADCAST: [u8; 6] = [0xFF; 6];
const CHANNEL: u8 = 1;
const HARDWARE_ID: &str = "syringe_pump_with_valve.runze.SY03B-T06";
const REG_INTERVAL_TICKS: u32 = 10; // REG every N telemetry ticks (1 tick = 1 s)

// UART bridge — LilyGO labels these pins RS485_TX_1 / RS485_RX_1.
const UART_BAUD: u32 = 9600; // ChinWe Runze — confirmed by ThinkPad TCP ground truth
const UART_FRAME_TIMEOUT_MS: u32 = 50;
const ESPNOW_MAX_PAYLOAD: usize = 244; // 250-byte cap minus 6-byte dst_mac prefix

// Panel — portrait, controller framebuffer is 320x480, visible window offset by 49 cols.
const W: i32 = 222;
const H: i32 = 480;

enum DisplayEvent {
    Tx { counter: u32, uptime_ms: u32 },
    Rx { len: usize, bytes: [u8; 16], bytes_len: usize, rx_count: u32 },
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    thread::sleep(Duration::from_secs(2));
    log::info!("[child] boot");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // -------- Display --------
    // Backlight: simple GPIO high is enough (PWM is possible via LEDC but unnecessary).
    let mut bl = PinDriver::output(peripherals.pins.gpio46)?;
    bl.set_high()?;

    let sclk = peripherals.pins.gpio12;
    let mosi = peripherals.pins.gpio11;
    let cs = peripherals.pins.gpio21;
    let dc = PinDriver::output(peripherals.pins.gpio41)?;

    let spi = SpiDeviceDriver::new_single(
        peripherals.spi2,
        sclk,
        mosi,
        Option::<AnyIOPin>::None,
        Some(cs),
        &DriverConfig::new().dma(Dma::Auto(4096)),
        &SpiConfig::new().baudrate(40.MHz().into()),
    )?;

    // mipidsi SPI interface buffer — larger = fewer transfers but more RAM.
    let buf: &'static mut [u8; 4096] = Box::leak(Box::new([0u8; 4096]));
    let di = SpiInterface::new(spi, dc, buf);

    let mut display = Builder::new(ST7796, di)
        .display_size(W as u16, H as u16)
        .display_offset(49, 0)
        .orientation(Orientation::new().flip_horizontal())
        .invert_colors(ColorInversion::Inverted)
        .init(&mut Ets)
        .map_err(|e| anyhow::anyhow!("display init: {:?}", e))?;

    display
        .clear(Rgb565::BLACK)
        .map_err(|e| anyhow::anyhow!("clear: {:?}", e))?;

    // -------- WiFi / ESP-NOW --------
    let mut wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start()?;

    unsafe {
        esp_idf_svc::sys::esp_wifi_set_channel(
            CHANNEL,
            esp_idf_svc::sys::wifi_second_chan_t_WIFI_SECOND_CHAN_NONE,
        );
    }

    let mut my_mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp!(esp_read_mac(my_mac.as_mut_ptr(), esp_mac_type_t_ESP_MAC_WIFI_STA))?;
    }
    log::info!("[child] my MAC = {:02X?}", my_mac);

    let espnow = Arc::new(EspNow::take()?);
    let mut peer = PeerInfo::default();
    peer.peer_addr = BROADCAST;
    peer.channel = CHANNEL;
    peer.encrypt = false;
    espnow.add_peer(peer)?;

    // -------- RS-485 bridge (via on-board TD501D485H-A transceiver) --------
    // GPIO17 → transceiver TXD (drives A/B lines)
    // GPIO18 ← transceiver RXD (receives A/B lines)
    // The transceiver handles DE/RE internally, so no direction pin is needed.
    // Push-pull default output is what the transceiver's logic side expects;
    // verified end-to-end via the `uart-count` diagnostic firmware on a
    // USB-RS485 sniffer.
    let uart = Arc::new(UartDriver::new(
        peripherals.uart1,
        peripherals.pins.gpio17,
        peripherals.pins.gpio18,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &UartConfig::new().baudrate(esp_idf_svc::hal::units::Hertz(UART_BAUD)),
    )?);

    log::info!(
        "[uart] RS-485 bridge ready on GPIO17(TX)/GPIO18(RX) @ {} baud",
        UART_BAUD
    );

    // -------- Display thread --------
    // All panel access happens here so the callback never touches SPI.
    let (disp_tx, disp_rx): (Sender<DisplayEvent>, Receiver<DisplayEvent>) = channel();
    let my_mac_for_disp = my_mac;
    thread::Builder::new()
        .stack_size(8 * 1024)
        .spawn(move || display_task(display, my_mac_for_disp, disp_rx))?;

    // Seed display with the static header right away.
    let _ = disp_tx.send(DisplayEvent::Tx { counter: 0, uptime_ms: 0 });

    // -------- ESP-NOW RX → UART TX side --------
    // Callback runs in WiFi task context, so it only drops the payload into a
    // channel; the handler thread owns the actual UART write + display update.
    let (rx_tx, rx_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
    let my_mac_for_cb = my_mac;
    espnow.register_recv_cb(move |_info: &esp_idf_svc::espnow::ReceiveInfo, data: &[u8]| {
        if data.len() >= 6 && data[..6] == my_mac_for_cb {
            let payload = data[6..].to_vec();
            let _ = rx_tx.send(payload);
        }
    })?;

    let disp_tx_for_rx = disp_tx.clone();
    let uart_for_rx = Arc::clone(&uart);
    thread::spawn(move || {
        let mut rx_count: u32 = 0;
        while let Ok(payload) = rx_rx.recv() {
            rx_count = rx_count.wrapping_add(1);
            log::info!(
                "[rx-for-me] {} bytes -> UART: {:02X?}",
                payload.len(),
                &payload[..payload.len().min(32)]
            );
            match uart_for_rx.write(&payload) {
                Ok(n) if n == payload.len() => {}
                Ok(n) => log::warn!("[uart tx] short write: {}/{}", n, payload.len()),
                Err(e) => log::warn!("[uart tx] failed: {:?}", e),
            }
            let mut bytes = [0u8; 16];
            let bytes_len = payload.len().min(16);
            bytes[..bytes_len].copy_from_slice(&payload[..bytes_len]);
            let _ = disp_tx_for_rx.send(DisplayEvent::Rx {
                len: payload.len(),
                bytes,
                bytes_len,
                rx_count,
            });
        }
    });

    // -------- UART RX → ESP-NOW TX side --------
    // Reads bytes with idle-timeout framing, broadcasts each frame via ESP-NOW
    // so the gateway picks it up as a normal RX line. Mother's
    // `EspNowChildTransport` receives under transport_id=hardware_id.
    let uart_for_reader = Arc::clone(&uart);
    let espnow_for_reader = Arc::clone(&espnow);
    thread::Builder::new()
        .stack_size(4 * 1024)
        .spawn(move || uart_reader_task(uart_for_reader, espnow_for_reader))?;

    // -------- TX loop --------
    // Send REG once immediately so the gateway/mother learns us fast, then every
    // REG_INTERVAL_TICKS as a heartbeat so late-booting mothers still pick us up.
    let _ = send_reg(&*espnow);

    let start = Instant::now();
    let mut counter: u32 = 0;
    loop {
        let uptime_ms = start.elapsed().as_millis() as u32;
        let mut payload = [0u8; 8];
        payload[0..4].copy_from_slice(&counter.to_le_bytes());
        payload[4..8].copy_from_slice(&uptime_ms.to_le_bytes());

        if counter > 0 && counter % REG_INTERVAL_TICKS == 0 {
            let _ = send_reg(&*espnow);
        }

        match espnow.send(BROADCAST, &payload) {
            Ok(()) => log::info!("[tx] counter={} uptime_ms={}", counter, uptime_ms),
            Err(e) => log::warn!("[tx] failed: {:?}", e),
        }
        let _ = disp_tx.send(DisplayEvent::Tx { counter, uptime_ms });
        counter = counter.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}

/// Read from the UART driver in a blocking loop and broadcast each completed
/// frame via ESP-NOW. Frames are coalesced with a UART_FRAME_TIMEOUT_MS idle
/// gap — same scheme the Mac-side DirectSerialTransport uses. Payloads above
/// ESPNOW_MAX_PAYLOAD (244 B) are split into successive broadcasts; the
/// gateway's line protocol doesn't carry a length field, so each broadcast
/// surfaces to the mother as its own `RX <mac> <hex>` line.
fn uart_reader_task(uart: Arc<UartDriver<'static>>, espnow: Arc<EspNow>) {
    let mut read_buf = [0u8; 256];
    let mut frame = Vec::<u8>::with_capacity(512);
    let idle_ticks = pdms_to_ticks(UART_FRAME_TIMEOUT_MS);

    loop {
        // Block up to 1 s for the first byte of a new frame. Using a timeout
        // (not portMAX_DELAY) keeps us responsive if we ever need to bail.
        match uart.read(&mut read_buf, pdms_to_ticks(1_000)) {
            Ok(0) => continue,
            Ok(n) => frame.extend_from_slice(&read_buf[..n]),
            Err(e) => {
                // ESP_ERR_TIMEOUT (263) is the normal "idle, no bytes yet" case.
                // Only surface other errors.
                if e.code() != esp_idf_svc::sys::ESP_ERR_TIMEOUT as i32 {
                    log::warn!("[uart rx] read error: {:?}", e);
                    thread::sleep(Duration::from_millis(100));
                }
                continue;
            }
        }

        // Coalesce: keep pulling bytes until the line has been quiet for
        // UART_FRAME_TIMEOUT_MS, then flush whatever we've buffered.
        loop {
            match uart.read(&mut read_buf, idle_ticks) {
                Ok(0) => break, // idle timeout
                Ok(n) => frame.extend_from_slice(&read_buf[..n]),
                Err(_) => break,
            }
        }

        if frame.is_empty() {
            continue;
        }

        log::info!(
            "[uart rx -> radio] {} bytes: {:02X?}",
            frame.len(),
            &frame[..frame.len().min(32)]
        );

        for chunk in frame.chunks(ESPNOW_MAX_PAYLOAD) {
            if let Err(e) = espnow.send(BROADCAST, chunk) {
                log::warn!("[uart rx -> radio] send failed: {:?}", e);
            }
        }
        frame.clear();
    }
}

/// Convert milliseconds to FreeRTOS ticks for `UartDriver::read`.
fn pdms_to_ticks(ms: u32) -> u32 {
    // pdMS_TO_TICKS(x) = (x * configTICK_RATE_HZ) / 1000 — usually 100 Hz.
    let hz = esp_idf_svc::sys::configTICK_RATE_HZ;
    (ms.saturating_mul(hz)) / 1000
}

/// Broadcast a registration frame so the mother can build its MAC -> hardware_id
/// table without any hard-coded config. Format: ASCII `REG <hardware_id>`.
/// Mother parses this in `EspNowGatewayClient` — see
/// `OpenSDL/crates/osdl-core/src/transport/espnow_gateway.rs`.
fn send_reg(espnow: &EspNow) -> Result<(), anyhow::Error> {
    let mut payload = Vec::with_capacity(4 + HARDWARE_ID.len());
    payload.extend_from_slice(b"REG ");
    payload.extend_from_slice(HARDWARE_ID.as_bytes());
    match espnow.send(BROADCAST, &payload) {
        Ok(()) => {
            log::info!("[reg] announced hardware_id={}", HARDWARE_ID);
            Ok(())
        }
        Err(e) => {
            log::warn!("[reg] failed: {:?}", e);
            Err(anyhow::anyhow!("{:?}", e))
        }
    }
}

fn display_task<D>(mut display: D, my_mac: [u8; 6], rx: Receiver<DisplayEvent>)
where
    D: DrawTarget<Color = Rgb565>,
    D::Error: core::fmt::Debug,
{
    let header_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let label_style = MonoTextStyle::new(&FONT_6X10, Rgb565::new(15, 40, 15)); // dim green
    let value_style = MonoTextStyle::new(&FONT_10X20, Rgb565::new(10, 60, 10)); // bright green
    let rx_value_style = MonoTextStyle::new(&FONT_10X20, Rgb565::new(30, 45, 10)); // amber
    let hex_style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    let divider = PrimitiveStyle::with_stroke(Rgb565::new(8, 16, 8), 1);

    // Static chrome — drawn once.
    let _ = display.clear(Rgb565::BLACK);
    let mac_s = format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        my_mac[0], my_mac[1], my_mac[2], my_mac[3], my_mac[4], my_mac[5]
    );
    let _ = Text::with_baseline("pump-01", Point::new(8, 6), header_style, Baseline::Top)
        .draw(&mut display);
    let _ = Text::with_baseline(&mac_s, Point::new(8, 30), label_style, Baseline::Top)
        .draw(&mut display);
    let _ = Text::with_baseline(
        &format!("CH {} / ESP-NOW broadcast", CHANNEL),
        Point::new(8, 44),
        label_style,
        Baseline::Top,
    )
    .draw(&mut display);

    let _ = Rectangle::new(Point::new(0, 60), Size::new(W as u32, 1))
        .into_styled(divider)
        .draw(&mut display);

    // Section labels
    let _ = Text::with_baseline("TX ->", Point::new(8, 70), label_style, Baseline::Top)
        .draw(&mut display);
    let _ = Text::with_baseline("RX <-", Point::new(8, 230), label_style, Baseline::Top)
        .draw(&mut display);
    let _ = Rectangle::new(Point::new(0, 220), Size::new(W as u32, 1))
        .into_styled(divider)
        .draw(&mut display);

    // Dynamic regions get repainted on each update.
    let tx_region = Rectangle::new(Point::new(0, 88), Size::new(W as u32, 120));
    let rx_region = Rectangle::new(Point::new(0, 248), Size::new(W as u32, 220));
    let bg = PrimitiveStyle::with_fill(Rgb565::BLACK);

    while let Ok(ev) = rx.recv() {
        match ev {
            DisplayEvent::Tx { counter, uptime_ms } => {
                let _ = tx_region.into_styled(bg).draw(&mut display);
                let _ = Text::with_baseline(
                    &format!("#{}", counter),
                    Point::new(8, 92),
                    value_style,
                    Baseline::Top,
                )
                .draw(&mut display);
                let secs = uptime_ms as f32 / 1000.0;
                let _ = Text::with_baseline(
                    &format!("up {:.1}s", secs),
                    Point::new(8, 120),
                    value_style,
                    Baseline::Top,
                )
                .draw(&mut display);
            }
            DisplayEvent::Rx {
                len,
                bytes,
                bytes_len,
                rx_count,
            } => {
                let _ = rx_region.into_styled(bg).draw(&mut display);
                let _ = Text::with_baseline(
                    &format!("#{} ({}B)", rx_count, len),
                    Point::new(8, 252),
                    rx_value_style,
                    Baseline::Top,
                )
                .draw(&mut display);

                // Render hex bytes, 8 per line.
                let mut line = String::with_capacity(24);
                let mut y = 286;
                for (i, b) in bytes[..bytes_len].iter().enumerate() {
                    if i > 0 && i % 8 == 0 {
                        let _ = Text::with_baseline(
                            &line,
                            Point::new(8, y),
                            hex_style,
                            Baseline::Top,
                        )
                        .draw(&mut display);
                        line.clear();
                        y += 14;
                    }
                    line.push_str(&format!("{:02X} ", b));
                }
                if !line.is_empty() {
                    let _ = Text::with_baseline(
                        &line,
                        Point::new(8, y),
                        hex_style,
                        Baseline::Top,
                    )
                    .draw(&mut display);
                }
            }
        }
    }
}
