//! ESP-NOW gateway — forwards traffic between Mac (via UART0 line protocol)
//! and child ESP32 nodes (via ESP-NOW broadcast).
//!
//! Uses broadcast (FF:FF:FF:FF:FF:FF) in both directions so we don't have to
//! maintain a peer list. Child nodes filter inbound by their own MAC.
//!
//! Line protocol over UART0:
//!   gateway → Mac:  `RX <src_mac_hex> <hex_bytes>\n`
//!                   `ER <reason>\n`
//!   Mac → gateway:  `TX <dst_mac_hex> <hex_bytes>\n`
//!                     (dst_mac is embedded in payload so child can filter)

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::espnow::{EspNow, PeerInfo};
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::{config::Config as UartConfig, UartDriver};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{esp_mac_type_t_ESP_MAC_WIFI_STA, esp_read_mac};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};

const BROADCAST: [u8; 6] = [0xFF; 6];
const CHANNEL: u8 = 1;

enum GwEvent {
    Rx { src: [u8; 6], data: Vec<u8> },
    TxRequest { dst: [u8; 6], data: Vec<u8> },
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("[gateway] boot");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // UART0 = CH343 "COM" port. ESP-IDF's default `stdin` goes through newlib +
    // VFS and silently drops most lines on this sdkconfig (line-discipline
    // issues — most `TX ...\n` writes from Mac never surface to `read_line`).
    // Take UART0 directly so we read raw bytes ourselves. `log::info!` output
    // still works because the logger writes TX bytes via register writes that
    // bypass the driver's RX path.
    let uart0 = UartDriver::new(
        peripherals.uart0,
        peripherals.pins.gpio43,
        peripherals.pins.gpio44,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &UartConfig::new().baudrate(Hertz(115_200)),
    )?;
    let uart0 = Arc::new(uart0);

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
    log::info!("[gateway] my MAC = {}", mac_hex(&my_mac));

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

    // UART0 reader thread — byte-level accumulate until `\n`, then parse.
    // Replaces the old `io::stdin().read_line()` approach which lost most lines.
    let tx_for_uart = tx.clone();
    let uart0_for_reader = Arc::clone(&uart0);
    thread::Builder::new()
        .stack_size(4 * 1024)
        .spawn(move || uart0_reader_task(uart0_for_reader, tx_for_uart))?;

    log::info!("[gateway] ready (broadcast mode, channel {})", CHANNEL);

    loop {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(GwEvent::Rx { src, data }) => {
                emit_line(&format!("RX {} {}", mac_hex(&src), bytes_hex(&data)));
            }
            Ok(GwEvent::TxRequest { dst, data }) => {
                // Wrap: [dst_mac(6) | payload(...)]
                let mut frame = Vec::with_capacity(6 + data.len());
                frame.extend_from_slice(&dst);
                frame.extend_from_slice(&data);

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
    // Use log::info! which goes to the same UART0 as everything else.
    // Mac-side parser can match against a known prefix like `espnow_gateway: RX `.
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

/// Block-read bytes from UART0 (CH343 link to Mac), accumulate until `\n`,
/// then dispatch parsed `TX ...` lines onto the event channel. Lines over
/// 1 KiB are dropped with an `ER overflow` reply. Treats `\r` as whitespace
/// so CRLF works too.
fn uart0_reader_task(uart: Arc<UartDriver<'static>>, tx: Sender<GwEvent>) {
    const MAX_LINE: usize = 1024;
    let mut buf = [0u8; 128];
    let mut line = Vec::<u8>::with_capacity(128);

    loop {
        // portMAX_DELAY — block until something arrives.
        match uart.read(&mut buf, u32::MAX) {
            Ok(0) => continue,
            Ok(n) => {
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
            Err(e) => {
                log::warn!("[uart0 rx] read error: {:?}", e);
                thread::sleep(Duration::from_millis(100));
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
