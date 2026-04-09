//! Transport abstraction for device communication.
//!
//! A Transport handles the physical delivery of bytes to/from a device.
//! The ProtocolAdapter handles WHAT the bytes mean; Transport handles HOW
//! they get there.
//!
//! ```text
//!   ProtocolAdapter: set_temperature(80) → [FE B1 01 50]
//!                                              │
//!   Transport:                                  ▼
//!     MqttSerial  → MQTT → ESP32 → RS-485 → device
//!     DirectSerial → /dev/ttyUSB0 → device
//!     Tcp         → TCP socket → device
//!     Http        → REST API → device
//! ```

pub mod mqtt_serial;
pub mod direct_serial;
pub mod tcp;

use async_trait::async_trait;

/// A transport channel for sending/receiving bytes to/from a device.
///
/// Each Device is associated with one Transport instance. The engine calls
/// `send()` to deliver encoded command bytes, and the transport pushes
/// received bytes back via the `rx_tx` channel provided at creation.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Transport type identifier: "mqtt_serial", "direct_serial", "tcp", "http"
    fn transport_type(&self) -> &str;

    /// Human-readable description (e.g., "MQTT via pump-01", "/dev/ttyUSB0", "192.168.1.50:502")
    fn description(&self) -> String;

    /// Send bytes to the device.
    async fn send(&self, bytes: &[u8]) -> Result<(), String>;

    /// Whether this transport is currently connected/available.
    fn is_connected(&self) -> bool;

    /// Start the transport (e.g., open serial port, connect TCP socket).
    /// For MQTT serial, this is a no-op since MQTT is already connected.
    async fn start(&self) -> Result<(), String> {
        Ok(())
    }

    /// Stop the transport and release resources.
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Bytes received from a device via its transport.
#[derive(Debug, Clone)]
pub struct TransportRx {
    pub transport_id: String,
    pub data: Vec<u8>,
}
