//! ESP-NOW child for boards built around a standard ESP32-D0WD with an
//! external MAX485 transceiver. Sibling crate to `firmware/esp32-rs`; the
//! wire protocol with the gateway is identical so the Mac side
//! (`EspNowGatewayClient`) needs no changes.
//!
//! RS-485 sits on UART2 (TX=GPIO17 / RX=GPIO16, 115200 8N1). MAX485 DE/RE
//! on GPIO22 is driven HIGH for transmit and LOW for receive; we call
//! `uart_wait_tx_done` after every write so the last bit leaves the shift
//! register before we drop back to RX. Half-duplex turn-around timing is
//! the most common foot-gun on this kind of board.
//!
//! Inbound ESP-NOW frame format: `[dst_mac(6) | payload(...)]`. Child
//! filters by checking `dst_mac == my_mac`; everything else is dropped.
//! HARDWARE_ID announces the bus (not an individual device); the mother's
//! `BusConfig.match_hardware_id` fans this single REG out into the X/Y/Z
//! + YYQ devices that share the bus.

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
use esp_idf_svc::sys::{
    esp_mac_type_t_ESP_MAC_WIFI_STA, esp_read_mac, uart_port_t, uart_wait_tx_done,
};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};

const BROADCAST: [u8; 6] = [0xFF; 6];
const CHANNEL: u8 = 1;

/// Mother-side `BusConfig.match_hardware_id` should equal this. The bus
/// manifest fans this single REG out into the X/Y/Z + YYQ devices declared
/// in `registry/unilabos/laiyu_xyz_pipette.yaml`.
const HARDWARE_ID: &str = "bus.laiyu_xyz.station1";

const REG_INTERVAL_TICKS: u32 = 10; // REG every N seconds
const UART_BAUD: u32 = 115200;
const UART_FRAME_TIMEOUT_MS: u32 = 50;
const ESPNOW_MAX_PAYLOAD: usize = 244; // 250-byte cap minus 6-byte dst_mac prefix

/// Half-duplex turn-around guard. After `uart_wait_tx_done` returns, give
/// the line a small margin before dropping DE so a jittery transceiver
/// doesn't clip the stop bit. 1 character at 115200 = ~87 µs; we allow ~5×.
const TURNAROUND_US: u32 = 500;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    thread::sleep(Duration::from_secs(2));
    log::info!("[child-max485] boot");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

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
        "[child-max485] my MAC = {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        my_mac[0], my_mac[1], my_mac[2], my_mac[3], my_mac[4], my_mac[5]
    );

    let espnow = Arc::new(EspNow::take()?);
    let mut peer = PeerInfo::default();
    peer.peer_addr = BROADCAST;
    peer.channel = CHANNEL;
    peer.encrypt = false;
    espnow.add_peer(peer)?;

    // -------- MAX485 DE/RE on GPIO22 --------
    // Only the ESP-NOW → UART writer thread ever touches DE/RE; the UART
    // reader stays in RX mode. So we move the PinDriver into that one
    // thread (no Arc / no Mutex) and let the type system enforce single
    // ownership.
    let mut de_re = PinDriver::output(peripherals.pins.gpio22)?;
    de_re.set_low()?; // start in receive mode
    log::info!("[max485] DE/RE = GPIO22, idle low (receive mode)");

    // -------- UART2 RS-485 bridge --------
    // GPIO17 TX → MAX485 DI, GPIO16 RX ← MAX485 RO. Matches `Serial2` on
    // standard ESP32 dev boards and the existing passthrough firmware.
    let uart = Arc::new(UartDriver::new(
        peripherals.uart2,
        peripherals.pins.gpio17,
        peripherals.pins.gpio16,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &UartConfig::new().baudrate(Hertz(UART_BAUD)),
    )?);
    log::info!(
        "[uart] RS-485 bridge ready on GPIO17(TX)/GPIO16(RX) @ {} baud",
        UART_BAUD
    );

    // -------- ESP-NOW RX → UART TX --------
    // Callback runs in WiFi task context, so it only drops the payload into a
    // channel; the handler thread owns the actual UART write + DE/RE toggle.
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
            if let Err(e) = write_rs485(&uart_for_rx, &mut de_re, &payload) {
                log::warn!("[uart tx] {}", e);
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

/// Drive DE/RE high → write all bytes → wait until the line really empties
/// → drive DE/RE low.
///
/// `uart_wait_tx_done` alone has been observed (against esp-idf-svc 0.52.x
/// on stock ESP32) to return before the driver's TX ring buffer is fully
/// drained when more than ~5 bytes are queued. The visible symptom on a
/// MAX485 was: 15-byte payload arrives as the first 5 bytes plus one
/// corrupted byte, with the rest silenced as DE drops mid-character.
///
/// To make turn-around independent of that quirk we additionally compute
/// the worst-case time to shift all bytes out and sleep for it. At 8N1
/// each byte is 10 bit times, so `n * 10 / baud` seconds. That bound is
/// the same number the Arduino passthrough firmware (which works on this
/// board) effectively waits for via `HardwareSerial::flush()`.
fn write_rs485(
    uart: &UartDriver<'static>,
    de_re: &mut PinDriver<'_, esp_idf_svc::hal::gpio::Output>,
    bytes: &[u8],
) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    de_re.set_high().map_err(|e| format!("DE high: {:?}", e))?;
    // Tiny settle so the bus stops floating before the start bit.
    esp_idf_svc::hal::delay::Ets::delay_us(10);

    let written = uart.write(bytes).map_err(|e| format!("uart write: {:?}", e))?;
    if written != bytes.len() {
        log::warn!("[uart tx] short write {}/{}", written, bytes.len());
    }

    // First the "official" wait (driver ring buffer + FIFO + shift reg).
    let port: uart_port_t = 2;
    let err = unsafe { uart_wait_tx_done(port, u32::MAX) };
    if err != 0 {
        log::warn!("[uart tx] wait_tx_done err={}", err);
    }

    // Then a baud-based hard floor: 10 bit times per byte at 8N1, plus
    // the existing settle margin. This is the part that turned out to be
    // load-bearing — wait_tx_done was returning early on multi-byte writes.
    // 1e6 us/s * 10 bit/byte / baud = us per byte.
    let bit_time_us_per_byte = 10_000_000u32 / UART_BAUD;
    let payload_us = bit_time_us_per_byte.saturating_mul(bytes.len() as u32);
    esp_idf_svc::hal::delay::Ets::delay_us(payload_us + TURNAROUND_US);

    de_re.set_low().map_err(|e| format!("DE low: {:?}", e))?;
    Ok(())
}

/// Read from UART2 in a blocking loop and broadcast each completed frame
/// via ESP-NOW. Same idle-timeout coalescing as the LilyGO child. DE/RE
/// stays low (RX) the whole time except briefly during writes — the writer
/// re-acquires the mutex every TX. We do NOT need to touch DE/RE here.
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

        // Coalesce until the line goes quiet for UART_FRAME_TIMEOUT_MS.
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

