//! Scan ChinWe RS-485 bus for Runze pumps using the correct `\r` line ending.
//!
//! The ChinWe station runs three Runze SY-03B pumps at addresses 1/2/3 on an
//! internal RS-485 bus whose master is normally a WiFi+TCP bridge module.
//! We've replaced that master with the LilyGO ESP-NOW child, so the bus
//! protocol is the same — Runze ASCII — but line endings are `\r` (not `\r\n`).
//!
//! Sends `/<addr>?0\r` to addresses 1..=3 and prints any reply.
//!
//! Run:
//!   cargo run -p osdl-cli --example chinwe_scan --features osdl-core/espnow

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let child_id = env::var("OSDL_CHILD_ID").unwrap_or_else(|_| "pump-01".to_string());

    let (tx, mut rx) = mpsc::unbounded_channel::<TransportRx>();
    let client = Arc::new(EspNowGatewayClient::new(port.clone(), 115200, tx));
    client.start().await.map_err(|e| e.to_string())?;

    log::info!("waiting up to 15s for REG <{}> ...", child_id);
    let mac = client
        .wait_for_registration(&child_id, Duration::from_secs(15))
        .await?;
    let child = EspNowChildTransport::new(child_id.clone(), mac, client.clone());
    child.start().await.map_err(|e| e.to_string())?;

    // Drain buffered telemetry
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    while (tokio::time::timeout_at(drain_deadline, rx.recv()).await).is_ok() {}

    log::info!("--- scanning Runze addresses 1..=3 with /<addr>?0\\r (ChinWe line_ending) ---");

    let mut total_replies = 0usize;

    // Also send a few distinctive test patterns to check if the driver is
    // producing *any* discriminable signal on the bus — if we can't even
    // self-hear our own TX through the transceiver's RX path, the driver
    // is dead electrically.
    let test_patterns: Vec<Vec<u8>> = vec![
        b"/1Q\r".to_vec(),
        b"/2Q\r".to_vec(),
        b"/3Q\r".to_vec(),
        b"UUUU".to_vec(),       // alternating bit pattern — best self-echo candidate
        b"AAAAAAAA".to_vec(),   // repeating char — stress auto-direction turnaround
        vec![0x55; 16],         // ten 0x55 bytes — longest continuous transition density
    ];

    for bytes in &test_patterns {
        log::info!("→ pattern ({} B): {}", bytes.len(), escape(bytes));
        child.send(bytes).await?;

        // Listen for 2s — anything appearing here is either a pump reply OR
        // our own TX echoing back through the transceiver's RX path.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if frame.data.len() == 8 {
                continue;
            }
            total_replies += 1;
            log::info!("    ← {} B: \"{}\"", frame.data.len(), escape(&frame.data));
            log::info!("       hex={}", hex(&frame.data));
        }
    }

    if total_replies == 0 {
        log::warn!("no replies — try swapping A/B or check wiring integrity");
    }

    client.stop().await.ok();
    Ok(())
}

fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02X}", b));
    }
    s
}

fn escape(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        match b {
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7E => s.push(b as char),
            _ => s.push_str(&format!("\\x{:02X}", b)),
        }
    }
    s
}
