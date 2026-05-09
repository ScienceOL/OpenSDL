//! ESP-NOW gateway transport — one USB-CDC gateway multiplexing N ESP-NOW children.
//!
//! Architecture (Option B — "per-child transport over a shared gateway client"):
//! ```text
//!   engine device "pump-01"  ─┐
//!   engine device "pump-02"  ─┤ Arc<EspNowGatewayClient>  ──USB──  Gateway ESP32
//!   engine device "balance-1" ─┘    (owns serial I/O)                  ↕ ESP-NOW
//!                                                                    children
//! ```
//!
//! The shared `EspNowGatewayClient` owns the serial port and parses the line
//! protocol emitted by `espnow_gateway.rs` on the YD board:
//!   `I (123) espnow_gateway: RX <mac_hex> <hex_bytes>\n`
//! Outbound frames become:
//!   `TX <mac_hex> <hex_bytes>\n`
//!
//! Each child is exposed to the engine as its own `EspNowChildTransport` whose
//! `transport_id` equals the engine-facing hardware id (e.g. `"pump-01"`). The
//! client dispatches inbound frames to the matching child's `transport_id`
//! based on a `MAC → hardware_id` lookup table built at setup time.
//!
//! Requires the `espnow` feature: `cargo build --features espnow`.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;

pub type Mac = [u8; 6];

/// Shared owner of the USB-CDC connection to a gateway board. Holds the
/// `MAC → hardware_id` table used to route inbound frames and exposes a
/// `send_to_mac` entrypoint for per-child transports to call.
pub struct EspNowGatewayClient {
    port_path: String,
    baud_rate: u32,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    /// MAC -> hardware_id. Used to tag inbound frames with the right transport_id.
    routes: RwLock<HashMap<Mac, String>>,
    connected: Arc<AtomicBool>,
    writer: Arc<Mutex<Option<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>>,
    read_task: Mutex<Option<JoinHandle<()>>>,
}

impl EspNowGatewayClient {
    pub fn new(
        port_path: String,
        baud_rate: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Self {
        Self {
            port_path,
            baud_rate,
            rx_tx,
            routes: RwLock::new(HashMap::new()),
            connected: Arc::new(AtomicBool::new(false)),
            writer: Arc::new(Mutex::new(None)),
            read_task: Mutex::new(None),
        }
    }

    /// Register a hardware_id ↔ MAC binding so inbound frames from that MAC are
    /// dispatched to the engine with `transport_id = hardware_id`.
    pub async fn register_device(&self, hardware_id: String, mac: Mac) {
        self.routes.write().await.insert(mac, hardware_id);
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Send a payload to one child, identified by MAC. Formats as
    /// `TX <mac_hex> <hex_bytes>\n` on the gateway's UART.
    pub async fn send_to_mac(&self, mac: Mac, bytes: &[u8]) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let line = format!("TX {} {}\n", mac_hex(&mac), bytes_hex(bytes));
        let mut guard = self.writer.lock().await;
        let writer = guard.as_mut().ok_or("Gateway serial port not open")?;
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("Gateway serial write error: {}", e))
    }

    #[cfg(feature = "espnow")]
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio_serial::SerialPortBuilderExt;

        log::info!(
            "ESP-NOW gateway: opening {} @ {} baud",
            self.port_path,
            self.baud_rate
        );

        let stream = tokio_serial::new(&self.port_path, self.baud_rate)
            .open_native_async()
            .map_err(|e| format!("Gateway open {} failed: {}", self.port_path, e))?;

        let (reader, writer) = tokio::io::split(stream);
        *self.writer.lock().await = Some(Box::new(writer));
        self.connected.store(true, Ordering::Relaxed);

        let tx = self.rx_tx.clone();
        let port_path = self.port_path.clone();
        let connected = self.connected.clone();
        let this = Arc::clone(self);

        let handle = tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some((mac, payload)) = parse_rx_line(&line) {
                    let maybe_id = this.routes.read().await.get(&mac).cloned();
                    match maybe_id {
                        Some(hardware_id) => {
                            let _ = tx.send(TransportRx {
                                transport_id: hardware_id,
                                data: payload,
                            });
                        }
                        None => {
                            log::debug!(
                                "gateway RX from unregistered MAC {} ({} bytes) — dropping",
                                mac_hex(&mac),
                                payload.len()
                            );
                        }
                    }
                }
            }
            connected.store(false, Ordering::Relaxed);
            log::info!("ESP-NOW gateway read loop ended for {}", port_path);
        });

        *self.read_task.lock().await = Some(handle);
        log::info!(
            "ESP-NOW gateway: opened {} @ {} baud",
            self.port_path,
            self.baud_rate
        );
        Ok(())
    }

    #[cfg(not(feature = "espnow"))]
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        Err(format!(
            "ESP-NOW support not compiled. Enable the 'espnow' feature to use {}",
            self.port_path
        ))
    }

    pub async fn stop(&self) -> Result<(), String> {
        *self.writer.lock().await = None;
        if let Some(handle) = self.read_task.lock().await.take() {
            handle.abort();
        }
        self.connected.store(false, Ordering::Relaxed);
        log::info!("ESP-NOW gateway: closed {}", self.port_path);
        Ok(())
    }
}

/// Per-child transport. Delegates all I/O to a shared `EspNowGatewayClient`.
/// The engine treats it as a regular transport keyed by `hardware_id`.
pub struct EspNowChildTransport {
    hardware_id: String,
    mac: Mac,
    client: Arc<EspNowGatewayClient>,
}

impl EspNowChildTransport {
    pub fn new(hardware_id: String, mac: Mac, client: Arc<EspNowGatewayClient>) -> Self {
        Self { hardware_id, mac, client }
    }
}

#[async_trait]
impl Transport for EspNowChildTransport {
    fn transport_type(&self) -> &str {
        "espnow"
    }

    fn description(&self) -> String {
        format!("ESP-NOW {} via gateway", mac_hex(&self.mac))
    }

    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        self.client.send_to_mac(self.mac, bytes).await
    }

    fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    async fn start(&self) -> Result<(), String> {
        // Ensure the MAC → hardware_id route is registered. The shared client
        // itself is started by the caller once; multiple children calling start
        // on the transport is a no-op beyond route registration.
        self.client
            .register_device(self.hardware_id.clone(), self.mac)
            .await;
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
}

// ---------- Line parsing ----------

/// Parse `<stuff> RX <mac_hex12> <hex_bytes>` out of a gateway UART log line.
/// The gateway emits frames through ESP-IDF's logger, which prefixes them with
/// `I (ts) espnow_gateway:` and a timestamp — we skip that prefix and match on
/// `RX ` appearing anywhere in the line.
pub(crate) fn parse_rx_line(line: &str) -> Option<(Mac, Vec<u8>)> {
    let idx = line.find("RX ")?;
    let rest = &line[idx + 3..];
    let mut it = rest.split_whitespace();
    let mac_s = it.next()?;
    let hex_s = it.next()?;
    let mac = parse_mac(mac_s)?;
    let data = parse_hex_bytes(hex_s)?;
    Some((mac, data))
}

pub fn parse_mac(s: &str) -> Option<Mac> {
    if s.len() != 12 {
        return None;
    }
    let mut mac = [0u8; 6];
    for i in 0..6 {
        mac[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(mac)
}

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        out.push(u8::from_str_radix(&s[i..i + 2], 16).ok()?);
    }
    Some(out)
}

fn mac_hex(mac: &Mac) -> String {
    format!(
        "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn bytes_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_gateway_log_line() {
        let line = "I (782) espnow_gateway: RX 30EDA0B65B38 26000000E5950000";
        let (mac, data) = parse_rx_line(line).unwrap();
        assert_eq!(mac, [0x30, 0xED, 0xA0, 0xB6, 0x5B, 0x38]);
        assert_eq!(data, vec![0x26, 0x00, 0x00, 0x00, 0xE5, 0x95, 0x00, 0x00]);
    }

    #[test]
    fn parses_bare_rx_without_log_prefix() {
        let line = "RX 30EDA0B65B38 DEADBEEF";
        let (mac, data) = parse_rx_line(line).unwrap();
        assert_eq!(mac, [0x30, 0xED, 0xA0, 0xB6, 0x5B, 0x38]);
        assert_eq!(data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn ignores_non_rx_lines() {
        assert!(parse_rx_line("I (123) espnow_gateway: [tx->radio] to=... len=6").is_none());
        assert!(parse_rx_line("some random noise").is_none());
    }

    #[test]
    fn rejects_bad_mac_length() {
        assert!(parse_rx_line("RX ABCD DEADBEEF").is_none());
    }

    #[test]
    fn rejects_odd_hex_length() {
        assert!(parse_rx_line("RX 30EDA0B65B38 DEADBEE").is_none());
    }
}
