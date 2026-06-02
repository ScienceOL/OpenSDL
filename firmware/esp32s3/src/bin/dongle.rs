//! ESP-NOW dongle — forwards traffic between Mac (via USB-Serial-JTAG)
//! and node ESP32 boards (via ESP-NOW broadcast).
//!
//! Uses broadcast (FF:FF:FF:FF:FF:FF) in both directions so we don't have to
//! maintain a peer list. Nodes filter inbound by their own MAC.
//!
//! Line protocol over USB-Serial-JTAG (the ESP32-S3 native-USB CDC):
//!   dongle → Mac:  `RX <src_mac_hex> <hex_bytes>\n`
//!                  `ER <reason>\n`
//!   Mac → dongle:  `TX <dst_mac_hex> <hex_bytes>\n`
//!                    (dst_mac is embedded in payload so the node can filter)
//!
//! The host link runs over the chip's built-in USB-Serial-JTAG, which works
//! identically on the YD-ESP32-S3 USB-C port and the Pocket-Dongle-S3 (the
//! latter has no external USB-UART chip). Output goes through the default
//! ESP-IDF console; input goes through the dedicated USJ driver.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::espnow::{EspNow, PeerInfo};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{
    esp_mac_type_t_ESP_MAC_WIFI_STA, esp_read_mac, usb_serial_jtag_driver_config_t,
    usb_serial_jtag_driver_install, usb_serial_jtag_read_bytes,
};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};
use osdl_firmware_protocol::{espnow as espnow_codec, BROADCAST, CHANNEL};

// USB-Serial-JTAG ring buffers. The default config gives 256 B which can drop
// lines under bursty Mac-side writers (e.g., probe scripts). 2 KiB rx is
// plenty for our single-line protocol; tx_buffer_size only affects calls into
// `usb_serial_jtag_write_bytes` (we don't use that — log goes through the
// default console path), but the field must be > 0 or driver install fails.
const USJ_RX_BUF: u32 = 2048;
const USJ_TX_BUF: u32 = 256;

enum GwEvent {
    Rx { src: [u8; 6], data: Vec<u8> },
    TxRequest { dst: [u8; 6], data: Vec<u8> },
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("[dongle] boot");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // Install the USB-Serial-JTAG driver so we can do interrupt-driven reads
    // from the host. Output (logs, RX/ER lines emitted via `log::info!`) keeps
    // going through the default ESP-IDF console hooks, which are wired to the
    // same USJ peripheral and continue to work after install.
    unsafe {
        let mut cfg = usb_serial_jtag_driver_config_t {
            tx_buffer_size: USJ_TX_BUF,
            rx_buffer_size: USJ_RX_BUF,
        };
        esp_idf_svc::sys::esp!(usb_serial_jtag_driver_install(&mut cfg as *mut _))?;
    }

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
    log::info!("[dongle] my MAC = {}", mac_hex(&my_mac));

    let espnow = Arc::new(EspNow::take()?);

    // One-time broadcast peer setup so send() to FF:FF:FF:FF:FF:FF works.
    let mut peer = PeerInfo::default();
    peer.peer_addr = BROADCAST;
    peer.channel = CHANNEL;
    peer.encrypt = false;
    espnow.add_peer(peer)?;

    let (tx, rx): (Sender<GwEvent>, Receiver<GwEvent>) = channel();

    let tx_for_rx_cb = tx.clone();
    espnow.register_recv_cb(move |info: &esp_idf_svc::espnow::ReceiveInfo, data: &[u8]| {
        log::info!("[cb] from {:02X?} len={}", info.src_addr, data.len()); // DIAG
        let mut src = [0u8; 6];
        src.copy_from_slice(&info.src_addr[..6]);
        let _ = tx_for_rx_cb.send(GwEvent::Rx { src, data: data.to_vec() });
    })?;

    // Host reader thread — pull bytes out of the USJ driver's RX ring buffer,
    // accumulate until `\n`, then parse and dispatch. Replaces the old UART0
    // path used on YD boards via the CH343 chip; this works identically on
    // both YD's USB-C port and the Pocket-Dongle-S3.
    let tx_for_host = tx.clone();
    thread::Builder::new()
        .stack_size(4 * 1024)
        .spawn(move || host_reader_task(tx_for_host))?;

    log::info!("[dongle] ready (broadcast mode, channel {})", CHANNEL);

    loop {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(GwEvent::Rx { src, data }) => {
                emit_line(&format!("RX {} {}", mac_hex(&src), bytes_hex(&data)));
            }
            Ok(GwEvent::TxRequest { dst, data }) => {
                let frame = espnow_codec::build_frame(&dst, &data);
                match espnow.send(BROADCAST, &frame) {
                    Ok(()) => log::info!("[tx->radio] to={} len={}", mac_hex(&dst), data.len()),
                    Err(e) => emit_line(&format!("ER send {}: {:?}", mac_hex(&dst), e)),
                }
            }
            Err(_) => {} // timeout heartbeat — stay quiet
        }
    }
}

fn mac_hex(mac: &[u8; 6]) -> String {
    format!("{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5])
}

fn bytes_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data { s.push_str(&format!("{:02X}", b)); }
    s
}

fn emit_line(s: &str) {
    // Use log::info! which goes through the default ESP-IDF console hook
    // (USB-Serial-JTAG). The Mac-side parser (`EspNowDongleClient::parse_rx_line`)
    // matches `RX ` anywhere in the line, so the ESP-IDF logger's
    // `I (ts) <module>:` prefix doesn't matter — module name follows the bin
    // name (`dongle`) but parsing is prefix-agnostic on purpose.
    log::info!("{}", s);
}

fn parse_tx_line(line: &str) -> Result<([u8; 6], Vec<u8>), String> {
    let mut it = line.split_whitespace();
    let tag = it.next().ok_or("empty")?;
    if tag != "TX" { return Err(format!("unknown tag: {}", tag)); }
    let mac_s = it.next().ok_or("missing mac")?;
    let data_s = it.next().ok_or("missing data")?;
    let mac = parse_mac(mac_s)?;
    let data = parse_hex_bytes(data_s)?;
    if data.len() > 244 { return Err(format!("payload {}B > 244", data.len())); } // 250 - 6 MAC
    Ok((mac, data))
}

fn parse_mac(s: &str) -> Result<[u8; 6], String> {
    if s.len() != 12 { return Err(format!("mac hex must be 12 chars, got {}", s.len())); }
    let mut mac = [0u8; 6];
    for i in 0..6 {
        mac[i] = u8::from_str_radix(&s[i*2..i*2+2], 16)
            .map_err(|_| format!("bad mac hex at {}", i*2))?;
    }
    Ok(mac)
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 { return Err("hex length must be even".into()); }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        out.push(u8::from_str_radix(&s[i..i+2], 16)
            .map_err(|_| format!("bad hex at {}", i))?);
    }
    Ok(out)
}

/// Block-read bytes from the USB-Serial-JTAG driver, accumulate until `\n`,
/// then dispatch parsed `TX ...` lines onto the event channel. Lines over
/// 1 KiB are dropped with an `ER overflow` reply. Treats `\r` as whitespace
/// so CRLF works too.
fn host_reader_task(tx: Sender<GwEvent>) {
    const MAX_LINE: usize = 1024;
    let mut buf = [0u8; 128];
    let mut line = Vec::<u8>::with_capacity(128);

    loop {
        // portMAX_DELAY — block until something arrives. The driver returns
        // the count of bytes copied into `buf`; 0 means timeout (won't happen
        // with portMAX_DELAY, but be defensive).
        let n = unsafe {
            usb_serial_jtag_read_bytes(
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
                u32::MAX, // portMAX_DELAY
            )
        };

        if n <= 0 {
            // Negative = error (rare); 0 = no data. Sleep a tick so we don't
            // hot-spin on a misbehaving driver.
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let n = n as usize;
        for &b in &buf[..n] {
            if b == b'\n' {
                let trimmed = trim_cr(&line);
                if !trimmed.is_empty() {
                    match std::str::from_utf8(trimmed) {
                        Ok(s) => match parse_tx_line(s) {
                            Ok((dst, data)) => {
                                let _ = tx.send(GwEvent::TxRequest { dst, data });
                            }
                            Err(e) => emit_line(&format!("ER parse: {}", e)),
                        },
                        Err(_) => emit_line("ER parse: non-utf8"),
                    }
                }
                line.clear();
            } else if line.len() >= MAX_LINE {
                emit_line("ER overflow");
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
}

fn trim_cr(b: &[u8]) -> &[u8] {
    let mut end = b.len();
    while end > 0 && (b[end - 1] == b'\r' || b[end - 1] == b' ' || b[end - 1] == b'\t') {
        end -= 1;
    }
    let mut start = 0;
    while start < end && (b[start] == b' ' || b[start] == b'\t') {
        start += 1;
    }
    &b[start..end]
}
