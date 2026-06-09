//! ESP-NOW node for the LilyGO T-CAN485 board (silkscreen "XY_32_CAN+RS485
//! V1.1"). Differs from `node.rs` (bare ESP32 + external MAX485) in three
//! load-bearing ways, all forced by the on-board MAX13487E + boost converter:
//!
//! - **UART1 on GPIO22 (TX) / GPIO21 (RX)** at 115200 8N1.
//! - **GPIO16 BOOST_ENABLE must be HIGH** to power the on-board boost
//!   converter that feeds the RS485+CAN transceivers. Without this, the
//!   transceiver has no Vcc and the line stays floating.
//! - **GPIO19 SHUTDOWN must be HIGH** to bring MAX13487E out of shutdown.
//! - **GPIO17 (MAX13487E /RE) is held HIGH for the entire run**, matching
//!   LilyGO's reference example. MAX13487E uses AutoDirection internally,
//!   so the host doesn't toggle DE during writes — the chip flips direction
//!   based on bus activity. Holding /RE high is what the reference firmware
//!   does and we stay byte-for-byte compatible with it.
//!
//! All three pins above are pulled high once at boot and never touched again,
//! so this bin has no DE/RE turn-around code (compare `node.rs`'s
//! `write_rs485()` which inverts a direction pin around each write).
//!
//! Wire protocol with the dongle is identical to every other OSDL node, so
//! the mother (`EspNowDongleClient`) needs no changes.
//!
//! Identity is decided host-side: this firmware announces a MAC-only `REG`
//! (no hardware_id baked in), and the mother resolves the announcing MAC to
//! a station via `OsdlConfig.mac_assignments`. The same binary therefore
//! serves any station — flash-and-forget. See the new station's entry in
//! `docs/recipes/configs/*.yaml` for the MAC → hardware_id mapping.
//!
//! Reference: https://github.com/Xinyuan-LilyGO/T-CAN485/blob/main/examples/RS485/RS485.ino

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::espnow::{EspNow, PeerInfo};
use esp_idf_svc::hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::{config::Config as UartConfig, UartDriver};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{esp_mac_type_t_ESP_MAC_WIFI_STA, esp_read_mac};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};
use osdl_firmware_protocol::{reg as reg_codec, BROADCAST, CHANNEL, ESPNOW_MAX_PAYLOAD};

const REG_INTERVAL_TICKS: u32 = 10;
const UART_BAUD: u32 = 115200;
const UART_FRAME_TIMEOUT_MS: u32 = 50;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    thread::sleep(Duration::from_secs(2));
    log::info!("[node-tcan485] boot");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // -------- Board-level enables: do these BEFORE bringing up UART, so the
    // transceiver actually has Vcc when we start clocking out start bits.
    let mut boost_en = PinDriver::output(peripherals.pins.gpio16)?;
    boost_en.set_high()?;
    log::info!("[t-can485] BOOST_ENABLE = GPIO16 HIGH (transceiver Vcc up)");

    let mut rs485_shdn = PinDriver::output(peripherals.pins.gpio19)?;
    rs485_shdn.set_high()?;
    log::info!("[t-can485] RS485_SHUTDOWN = GPIO19 HIGH (MAX13487E enabled)");

    let mut rs485_re = PinDriver::output(peripherals.pins.gpio17)?;
    rs485_re.set_high()?;
    log::info!("[t-can485] RS485_RE = GPIO17 HIGH (AutoDirection mode, held)");

    // Keep the pins alive for the lifetime of `main` — dropping a `PinDriver`
    // releases the GPIO and the level reverts.
    std::mem::forget(boost_en);
    std::mem::forget(rs485_shdn);
    std::mem::forget(rs485_re);

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
    log::info!(
        "[node-tcan485] my MAC = {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        my_mac[0], my_mac[1], my_mac[2], my_mac[3], my_mac[4], my_mac[5]
    );

    let espnow = Arc::new(EspNow::take()?);
    let mut peer = PeerInfo::default();
    peer.peer_addr = BROADCAST;
    peer.channel = CHANNEL;
    peer.encrypt = false;
    espnow.add_peer(peer)?;

    // -------- UART1 RS-485 bridge --------
    let uart = Arc::new(UartDriver::new(
        peripherals.uart1,
        peripherals.pins.gpio22, // TX → MAX13487E DI
        peripherals.pins.gpio21, // RX ← MAX13487E RO
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &UartConfig::new().baudrate(Hertz(UART_BAUD)),
    )?);
    log::info!(
        "[uart] RS-485 bridge ready on UART1 GPIO22(TX)/GPIO21(RX) @ {} baud",
        UART_BAUD
    );

    // -------- ESP-NOW RX → UART TX --------
    let (rx_tx, rx_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
    let my_mac_for_cb = my_mac;
    espnow.register_recv_cb(move |_info: &esp_idf_svc::espnow::ReceiveInfo, data: &[u8]| {
        if data.len() >= 6 && data[..6] == my_mac_for_cb {
            let payload = data[6..].to_vec();
            let _ = rx_tx.send(payload);
        }
    })?;

    let uart_for_rx = Arc::clone(&uart);
    thread::spawn(move || {
        while let Ok(payload) = rx_rx.recv() {
            log::info!(
                "[rx-for-me] {} bytes -> UART: {:02X?}",
                payload.len(),
                &payload[..payload.len().min(32)]
            );
            // No DE/RE toggling — MAX13487E AutoDirection handles it.
            match uart_for_rx.write(&payload) {
                Ok(n) if n != payload.len() => {
                    log::warn!("[uart tx] short write {}/{}", n, payload.len());
                }
                Err(e) => log::warn!("[uart tx] write failed: {:?}", e),
                _ => {}
            }
        }
    });

    // -------- UART RX → ESP-NOW TX --------
    let uart_for_reader = Arc::clone(&uart);
    let espnow_for_reader = Arc::clone(&espnow);
    thread::Builder::new()
        .stack_size(4 * 1024)
        .spawn(move || uart_reader_task(uart_for_reader, espnow_for_reader))?;

    // -------- TX loop: REG + 1Hz heartbeat --------
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
        counter = counter.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}

fn uart_reader_task(uart: Arc<UartDriver<'static>>, espnow: Arc<EspNow>) {
    let mut read_buf = [0u8; 256];
    let mut frame = Vec::<u8>::with_capacity(512);
    let idle_ticks = pdms_to_ticks(UART_FRAME_TIMEOUT_MS);

    loop {
        match uart.read(&mut read_buf, pdms_to_ticks(1_000)) {
            Ok(0) => continue,
            Ok(n) => frame.extend_from_slice(&read_buf[..n]),
            Err(e) => {
                if e.code() != esp_idf_svc::sys::ESP_ERR_TIMEOUT as i32 {
                    log::warn!("[uart rx] read error: {:?}", e);
                    thread::sleep(Duration::from_millis(100));
                }
                continue;
            }
        }

        loop {
            match uart.read(&mut read_buf, idle_ticks) {
                Ok(0) => break,
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

fn pdms_to_ticks(ms: u32) -> u32 {
    let hz = esp_idf_svc::sys::configTICK_RATE_HZ;
    (ms.saturating_mul(hz)) / 1000
}

fn send_reg(espnow: &EspNow) -> Result<(), anyhow::Error> {
    let payload = reg_codec::build_mac_only();
    match espnow.send(BROADCAST, &payload) {
        Ok(()) => {
            log::info!("[reg] announced (mac-only) — mother resolves via mac_assignments");
            Ok(())
        }
        Err(e) => {
            log::warn!("[reg] failed: {:?}", e);
            Err(anyhow::anyhow!("{:?}", e))
        }
    }
}
