//! TCP socket transport — for devices reachable over the network.
//!
//! Covers Modbus TCP, SCPI over TCP, and any other TCP-based instrument protocol.
//! The mother node connects directly to the device's IP:port.
//!
//! TODO: implement when ready for network-connected instruments.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Transport that communicates over a TCP socket.
pub struct TcpTransport {
    host: String,
    port: u16,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    connected: std::sync::atomic::AtomicBool,
}

impl TcpTransport {
    pub fn new(
        host: String,
        port: u16,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Self {
        Self {
            host,
            port,
            rx_tx,
            connected: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Transport for TcpTransport {
    fn transport_type(&self) -> &str {
        "tcp"
    }

    fn description(&self) -> String {
        format!("TCP {}:{}", self.host, self.port)
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        // TODO: write bytes to TCP socket
        Err("TcpTransport not yet implemented".into())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn start(&self) -> Result<(), String> {
        // TODO: connect TCP, spawn read task that pushes to self.rx_tx
        log::info!("TCP: would connect to {}:{}", self.host, self.port);
        Err("TcpTransport not yet implemented".into())
    }
}
