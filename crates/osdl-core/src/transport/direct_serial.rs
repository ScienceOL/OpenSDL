//! Direct serial transport — USB/RS-232/RS-485 port on the mother node.
//!
//! For devices plugged directly into the mother node via USB-to-serial adapter.
//! No ESP32 child node needed. The mother node reads/writes the serial port directly.
//!
//! Used by Laiyu XYZ pipette station (/dev/ttyUSB0, 115200 baud, RS-485).
//!
//! Requires the `serial` feature: `cargo build --features serial`
//!
//! Frame coalescing: same idle-timeout approach as TCP transport.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

const DEFAULT_FRAME_TIMEOUT_MS: u64 = 50;

/// Transport that communicates directly through a local serial port.
///
/// The actual serial I/O requires the `serial` crate feature.
/// Without it, `start()` returns an error and `send()` fails.
pub struct DirectSerialTransport {
    port_path: String,
    baud_rate: u32,
    transport_id: String,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    connected: Arc<AtomicBool>,
    /// Type-erased writer: allows the struct to compile without tokio-serial.
    writer: Arc<Mutex<Option<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>>>,
    read_task: Mutex<Option<JoinHandle<()>>>,
    frame_timeout: Duration,
}

impl DirectSerialTransport {
    pub fn new(
        port_path: String,
        baud_rate: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Self {
        let transport_id = port_path.clone();
        Self {
            port_path,
            baud_rate,
            transport_id,
            rx_tx,
            connected: Arc::new(AtomicBool::new(false)),
            writer: Arc::new(Mutex::new(None)),
            read_task: Mutex::new(None),
            frame_timeout: Duration::from_millis(DEFAULT_FRAME_TIMEOUT_MS),
        }
    }

    pub fn with_frame_timeout(mut self, timeout: Duration) -> Self {
        self.frame_timeout = timeout;
        self
    }

    #[cfg(feature = "serial")]
    async fn open_and_start_read(&self) -> Result<(), String> {
        use tokio::io::AsyncReadExt;
        use tokio_serial::SerialPortBuilderExt;

        log::info!(
            "Serial: opening {} @ {} baud",
            self.port_path,
            self.baud_rate
        );

        let stream = tokio_serial::new(&self.port_path, self.baud_rate)
            .open_native_async()
            .map_err(|e| format!("Serial open {} failed: {}", self.port_path, e))?;

        let (mut reader, writer) = tokio::io::split(stream);

        *self.writer.lock().await = Some(Box::new(writer));
        self.connected.store(true, Ordering::Relaxed);

        let tx = self.rx_tx.clone();
        let transport_id = self.transport_id.clone();
        let connected = self.connected.clone();
        let frame_timeout = self.frame_timeout;

        let handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            let mut frame_buf = Vec::new();

            loop {
                let n = match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        log::error!("Serial read error on {}: {}", transport_id, e);
                        break;
                    }
                };
                frame_buf.extend_from_slice(&buf[..n]);

                // Coalesce: keep reading until idle for frame_timeout
                loop {
                    match tokio::time::timeout(frame_timeout, reader.read(&mut buf)).await {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => frame_buf.extend_from_slice(&buf[..n]),
                        Ok(Err(_)) => break,
                        Err(_) => break, // Timeout — frame complete
                    }
                }

                if !frame_buf.is_empty() {
                    let _ = tx.send(TransportRx {
                        transport_id: transport_id.clone(),
                        data: std::mem::take(&mut frame_buf),
                    });
                }
            }

            connected.store(false, Ordering::Relaxed);
            log::info!("Serial read loop ended for {}", transport_id);
        });

        *self.read_task.lock().await = Some(handle);
        log::info!("Serial: opened {} @ {} baud", self.port_path, self.baud_rate);
        Ok(())
    }

    #[cfg(not(feature = "serial"))]
    async fn open_and_start_read(&self) -> Result<(), String> {
        Err(format!(
            "Serial support not compiled. Enable the 'serial' feature to use {}",
            self.port_path
        ))
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

    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let mut guard = self.writer.lock().await;
        let writer = guard.as_mut().ok_or("Serial port not open")?;
        writer
            .write_all(bytes)
            .await
            .map_err(|e| format!("Serial write error: {}", e))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn start(&self) -> Result<(), String> {
        self.open_and_start_read().await
    }

    async fn stop(&self) -> Result<(), String> {
        *self.writer.lock().await = None;

        if let Some(handle) = self.read_task.lock().await.take() {
            handle.abort();
        }

        self.connected.store(false, Ordering::Relaxed);
        log::info!("Serial: closed {}", self.port_path);
        Ok(())
    }
}
