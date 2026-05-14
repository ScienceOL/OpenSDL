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
//! `transport_id` is derived from its MAC (`"espnow:<MAC_HEX>"`). Using the MAC
//! — which is guaranteed unique per board — avoids collisions when two
//! children run the same firmware / announce the same `hardware_id`. The
//! client also tracks an auxiliary `MAC ↔ hardware_id` table so callers that
//! care (e.g. `wait_for_registration`) can still look up by hardware_id.
//!
//! Requires the `espnow` feature: `cargo build --features espnow`.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex, Notify, RwLock};
use tokio::task::JoinHandle;

pub type Mac = [u8; 6];

/// Marker prefix in ESP-NOW payloads used by children to announce their
/// hardware_id. Keep in sync with firmware (`espnow_child.rs::send_reg`).
const REG_PREFIX: &[u8] = b"REG ";

/// Emitted by the gateway read loop every time a REG frame arrives. Consumers
/// (e.g. `OsdlEngine`) subscribe to build per-child transports and devices.
#[derive(Debug, Clone)]
pub struct RegEvent {
    pub hardware_id: String,
    pub mac: Mac,
    /// True the first time this MAC is seen on this gateway; false if the
    /// child is re-announcing (e.g. after a reboot).
    pub is_new: bool,
}

/// Shared owner of the USB-CDC connection to a gateway board. Holds the
/// `MAC ↔ hardware_id` table used to route inbound frames and exposes a
/// `send_to_mac` entrypoint for per-child transports to call. Children
/// announce themselves via `REG <hardware_id>` broadcasts which the client
/// parses to keep the table up to date (no static config required).
pub struct EspNowGatewayClient {
    port_path: String,
    baud_rate: u32,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    routes: RwLock<Routes>,
    /// Fires whenever a new REG arrives, so `wait_for_registration` can wake up.
    reg_notify: Notify,
    /// Broadcasts every REG frame so multiple subscribers (engine, tests, …)
    /// can react to child registration independently.
    reg_tx: broadcast::Sender<RegEvent>,
    connected: Arc<AtomicBool>,
    writer: Arc<Mutex<Option<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>>,
    read_task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct Routes {
    mac_to_id: HashMap<Mac, String>,
    id_to_mac: HashMap<String, Mac>,
}

impl Routes {
    fn upsert(&mut self, mac: Mac, hardware_id: String) {
        if let Some(old_id) = self.mac_to_id.insert(mac, hardware_id.clone()) {
            if old_id != hardware_id {
                self.id_to_mac.remove(&old_id);
            }
        }
        self.id_to_mac.insert(hardware_id, mac);
    }
}

impl EspNowGatewayClient {
    pub fn new(
        port_path: String,
        baud_rate: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Self {
        let (reg_tx, _) = broadcast::channel(64);
        Self {
            port_path,
            baud_rate,
            rx_tx,
            routes: RwLock::new(Routes::default()),
            reg_notify: Notify::new(),
            reg_tx,
            connected: Arc::new(AtomicBool::new(false)),
            writer: Arc::new(Mutex::new(None)),
            read_task: Mutex::new(None),
        }
    }

    /// Subscribe to REG events from this gateway. Each new subscription gets
    /// future events only (broadcast semantics); call `register_device` or
    /// iterate `known_registrations()` if you need the current state.
    pub fn subscribe_reg(&self) -> broadcast::Receiver<RegEvent> {
        self.reg_tx.subscribe()
    }

    /// Snapshot of the current MAC ↔ hardware_id table. Useful when a late
    /// subscriber needs to replay what has already registered.
    pub async fn known_registrations(&self) -> Vec<(String, Mac)> {
        self.routes
            .read()
            .await
            .id_to_mac
            .iter()
            .map(|(id, mac)| (id.clone(), *mac))
            .collect()
    }

    /// Register a hardware_id ↔ MAC binding manually. Normally unnecessary —
    /// children announce themselves via REG frames — but useful for tests or
    /// to pre-seed the table before a device has booted.
    pub async fn register_device(&self, hardware_id: String, mac: Mac) {
        let is_new = {
            let mut routes = self.routes.write().await;
            let was_unknown = !routes.mac_to_id.contains_key(&mac);
            routes.upsert(mac, hardware_id.clone());
            was_unknown
        };
        self.reg_notify.notify_waiters();
        let _ = self.reg_tx.send(RegEvent {
            hardware_id,
            mac,
            is_new,
        });
    }

    /// Look up the MAC for a given hardware_id, if known.
    pub async fn mac_for(&self, hardware_id: &str) -> Option<Mac> {
        self.routes.read().await.id_to_mac.get(hardware_id).copied()
    }

    /// Block until a child announces (or has already announced) `hardware_id`
    /// via REG. Returns its MAC. Useful for callers that want to construct an
    /// `EspNowChildTransport` without knowing the MAC in advance.
    pub async fn wait_for_registration(
        &self,
        hardware_id: &str,
        timeout: Duration,
    ) -> Result<Mac, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(mac) = self.mac_for(hardware_id).await {
                return Ok(mac);
            }
            let notified = self.reg_notify.notified();
            // Re-check after arming the notifier to avoid the lost-wakeup race.
            if let Some(mac) = self.mac_for(hardware_id).await {
                return Ok(mac);
            }
            match tokio::time::timeout_at(deadline, notified).await {
                Ok(()) => continue,
                Err(_) => {
                    return Err(format!(
                        "timed out waiting for REG <{}> on gateway {}",
                        hardware_id, self.port_path
                    ))
                }
            }
        }
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
                let Some((mac, payload)) = parse_rx_line(&line) else {
                    continue;
                };

                // REG frame? Update the routing table and notify waiters.
                if let Some(hardware_id) = parse_reg_payload(&payload) {
                    let is_new = {
                        let mut routes = this.routes.write().await;
                        let was_unknown = !routes.mac_to_id.contains_key(&mac);
                        routes.upsert(mac, hardware_id.clone());
                        was_unknown
                    };
                    if is_new {
                        log::info!(
                            "gateway registered {} = {}",
                            hardware_id,
                            mac_hex(&mac)
                        );
                    } else {
                        log::debug!(
                            "gateway re-REG {} = {}",
                            hardware_id,
                            mac_hex(&mac)
                        );
                    }
                    this.reg_notify.notify_waiters();
                    let _ = this.reg_tx.send(RegEvent {
                        hardware_id,
                        mac,
                        is_new,
                    });
                    continue;
                }

                // Route by MAC — every distinct board gets its own transport_id,
                // so two children running identical firmware don't collide.
                // Frames from MACs we've never seen a REG for are dropped: the
                // engine has no Device keyed on them yet and the convention is
                // REG-first anyway.
                let known = this.routes.read().await.mac_to_id.contains_key(&mac);
                if known {
                    let _ = tx.send(TransportRx {
                        transport_id: transport_id_for(&mac),
                        data: payload,
                    });
                } else {
                    log::debug!(
                        "gateway RX from unregistered MAC {} ({} bytes) — dropping",
                        mac_hex(&mac),
                        payload.len()
                    );
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

/// Canonical engine-facing transport id for a child, keyed on its MAC so
/// that two boards running identical firmware don't collide. Shared between
/// the gateway read loop (which stamps inbound frames) and the engine (which
/// looks them up).
pub fn transport_id_for(mac: &Mac) -> String {
    format!("espnow:{}", mac_hex(mac))
}

/// Per-child transport. Delegates all I/O to a shared `EspNowGatewayClient`.
/// Transport id is derived from the child's MAC (see [`transport_id_for`]).
pub struct EspNowChildTransport {
    mac: Mac,
    client: Arc<EspNowGatewayClient>,
}

impl EspNowChildTransport {
    pub fn new(mac: Mac, client: Arc<EspNowGatewayClient>) -> Self {
        Self { mac, client }
    }

    pub fn mac(&self) -> Mac {
        self.mac
    }

    pub fn transport_id(&self) -> String {
        transport_id_for(&self.mac)
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

/// Extract the hardware_id from a REG payload, or None if the bytes don't
/// look like a REG announcement. Format: ASCII `REG <hardware_id>`.
pub(crate) fn parse_reg_payload(payload: &[u8]) -> Option<String> {
    let tail = payload.strip_prefix(REG_PREFIX)?;
    // Must be valid UTF-8, non-empty, no embedded whitespace.
    let id = std::str::from_utf8(tail).ok()?.trim();
    if id.is_empty() || id.contains(char::is_whitespace) {
        return None;
    }
    // Light sanity: reject control chars so a stray binary frame that happens
    // to start with "REG " can't pollute the table.
    if id.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(id.to_string())
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

    #[test]
    fn parses_reg_payload() {
        let payload = b"REG pump-01";
        assert_eq!(parse_reg_payload(payload).as_deref(), Some("pump-01"));
    }

    #[test]
    fn reg_rejects_non_reg_payload() {
        // 8-byte counter+uptime payload shouldn't be mistaken for REG.
        let payload = [0x20, 0x13, 0x00, 0x00, 0x3C, 0x74, 0x4B, 0x00];
        assert_eq!(parse_reg_payload(&payload), None);
    }

    #[test]
    fn reg_rejects_empty_id() {
        assert_eq!(parse_reg_payload(b"REG "), None);
        assert_eq!(parse_reg_payload(b"REG   "), None);
    }

    #[test]
    fn reg_rejects_whitespace_in_id() {
        assert_eq!(parse_reg_payload(b"REG pump 01"), None);
    }

    #[test]
    fn reg_rejects_binary_garbage_after_prefix() {
        assert_eq!(parse_reg_payload(b"REG \x00\x01\x02"), None);
    }

    #[test]
    fn routes_upsert_updates_both_directions() {
        let mut r = Routes::default();
        let mac_a: Mac = [1, 2, 3, 4, 5, 6];
        r.upsert(mac_a, "pump-01".into());
        assert_eq!(r.mac_to_id.get(&mac_a).map(String::as_str), Some("pump-01"));
        assert_eq!(r.id_to_mac.get("pump-01").copied(), Some(mac_a));

        // Re-registering same MAC with a renamed hardware_id should evict the
        // old id_to_mac entry so reverse lookup stays consistent.
        r.upsert(mac_a, "pump-01b".into());
        assert_eq!(r.id_to_mac.get("pump-01"), None);
        assert_eq!(r.id_to_mac.get("pump-01b").copied(), Some(mac_a));
    }
}
