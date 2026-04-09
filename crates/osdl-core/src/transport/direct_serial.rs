//! Direct serial transport — USB/RS-232/RS-485 port on the mother node.
//!
//! For devices plugged directly into the mother node via USB-to-serial adapter.
//! No ESP32 child node needed. The mother node reads/writes the serial port directly.
//!
//! TODO: implement using `tokio-serial` crate when ready for real hardware.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Transport that communicates directly through a local serial port.
pub struct DirectSerialTransport {
    port_path: String,
    baud_rate: u32,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    connected: std::sync::atomic::AtomicBool,
}

impl DirectSerialTransport {
    pub fn new(
        port_path: String,
        baud_rate: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Self {
        Self {
            port_path,
            baud_rate,
            rx_tx,
            connected: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Transport for DirectSerialTransport {
    fn transport_type(&self) -> &str {
        "direct_serial"
    }

    fn description(&self) -> String {
        format!("{} @ {} baud", self.port_path, self.baud_rate)
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        // TODO: write bytes to serial port
        Err("DirectSerialTransport not yet implemented".into())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn start(&self) -> Result<(), String> {
        // TODO: open serial port, spawn read task that pushes to self.rx_tx
        log::info!(
            "DirectSerial: would open {} @ {} baud",
            self.port_path,
            self.baud_rate
        );
        Err("DirectSerialTransport not yet implemented".into())
    }
}
