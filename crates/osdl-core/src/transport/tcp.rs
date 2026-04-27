//! TCP socket transport — for devices reachable over the network.
//!
//! Used by ChinWe separator (192.168.31.201:8899) which multiplexes
//! ASCII (Runze pumps), binary (Emm motors), and Modbus RTU (XKC sensor)
//! over a single TCP connection.
//!
//! Frame coalescing: the read loop buffers incoming bytes and flushes them
//! as a complete frame after an idle timeout (default 50ms). This handles
//! all on-wire protocols without protocol-specific framing logic.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

const DEFAULT_FRAME_TIMEOUT_MS: u64 = 50;

/// Transport that communicates over a TCP socket.
pub struct TcpTransport {
    host: String,
    port: u16,
    transport_id: String,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    connected: Arc<AtomicBool>,
    writer: Arc<Mutex<Option<OwnedWriteHalf>>>,
    read_task: Mutex<Option<JoinHandle<()>>>,
    frame_timeout: Duration,
}

impl TcpTransport {
    pub fn new(host: String, port: u16, rx_tx: mpsc::UnboundedSender<TransportRx>) -> Self {
        let transport_id = format!("{}:{}", host, port);
        Self {
            host,
            port,
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
}

#[async_trait]
impl Transport for TcpTransport {
    fn transport_type(&self) -> &str {
        "tcp"
    }

    fn description(&self) -> String {
        format!("TCP {}:{}", self.host, self.port)
    }

    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let mut guard = self.writer.lock().await;
        let writer = guard.as_mut().ok_or("TCP not connected")?;
        writer
            .write_all(bytes)
            .await
            .map_err(|e| format!("TCP write error: {}", e))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn start(&self) -> Result<(), String> {
        let addr = format!("{}:{}", self.host, self.port);
        log::info!("TCP: connecting to {}", addr);

        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| format!("TCP connect to {} failed: {}", addr, e))?;

        stream.set_nodelay(true).ok();

        let (reader, writer) = stream.into_split();

        *self.writer.lock().await = Some(writer);
        self.connected.store(true, Ordering::Relaxed);

        let tx = self.rx_tx.clone();
        let transport_id = self.transport_id.clone();
        let connected = self.connected.clone();
        let frame_timeout = self.frame_timeout;

        let handle = tokio::spawn(async move {
            read_loop(reader, tx, transport_id, connected, frame_timeout).await;
        });

        *self.read_task.lock().await = Some(handle);
        log::info!("TCP: connected to {}", addr);
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        // Drop the writer half to close the write side
        *self.writer.lock().await = None;

        // Abort the read task
        if let Some(handle) = self.read_task.lock().await.take() {
            handle.abort();
        }

        self.connected.store(false, Ordering::Relaxed);
        log::info!("TCP: disconnected from {}:{}", self.host, self.port);
        Ok(())
    }
}

/// Read loop: buffer incoming bytes, flush as a frame after idle timeout.
async fn read_loop(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    tx: mpsc::UnboundedSender<TransportRx>,
    transport_id: String,
    connected: Arc<AtomicBool>,
    frame_timeout: Duration,
) {
    let mut buf = vec![0u8; 1024];
    let mut frame_buf = Vec::new();

    loop {
        // Wait for the first byte(s) of a new frame
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                log::error!("TCP read error on {}: {}", transport_id, e);
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
    log::info!("TCP read loop ended for {}", transport_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_tcp_connect_send_receive() {
        // Start a mock TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let transport = TcpTransport::new(
            addr.ip().to_string(),
            addr.port(),
            tx,
        );

        // Accept connection in background
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Read what client sends
            let mut buf = vec![0u8; 256];
            let n = stream.read(&mut buf).await.unwrap();
            let received = buf[..n].to_vec();

            // Send a response back
            tokio::time::sleep(Duration::from_millis(10)).await;
            stream.write_all(&[0x01, 0x03, 0x02, 0x00, 0x64]).await.unwrap();
            stream.flush().await.unwrap();

            received
        });

        // Connect
        transport.start().await.unwrap();
        assert!(transport.is_connected());

        // Send data
        transport.send(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x01]).await.unwrap();

        // Wait for the response to come through the channel
        let rx_msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rx_msg.data, vec![0x01, 0x03, 0x02, 0x00, 0x64]);

        // Verify server received our bytes
        let sent = server.await.unwrap();
        assert_eq!(sent, vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x01]);

        // Stop
        transport.stop().await.unwrap();
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn test_tcp_connect_failure() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let transport = TcpTransport::new("127.0.0.1".into(), 1, tx);
        let result = transport.start().await;
        assert!(result.is_err());
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn test_tcp_send_before_connect() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let transport = TcpTransport::new("127.0.0.1".into(), 9999, tx);
        let result = transport.send(&[0x01]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not connected"));
    }
}
