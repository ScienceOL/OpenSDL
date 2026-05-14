//! Minimal probe: prove `EspNowGatewayClient` + `EspNowChildTransport` work
//! end-to-end against a real gateway board.
//!
//! Discovery is now REG-based: the probe waits for the child to announce
//! itself via `REG <hardware_id>` on the radio, then uses the MAC it
//! learned to set up a per-child transport. No hard-coded MAC needed.
//!
//! What it does:
//!   1. Opens the gateway serial port (default `/dev/cu.usbserial-A5069RR4`).
//!   2. Waits up to 15s for a REG frame matching `OSDL_CHILD_ID` (default "pump-01").
//!   3. Listens for inbound telemetry from that child for 5 seconds.
//!   4. Sends one downstream test frame (`DE AD BE EF CA FE`).
//!   5. Listens 2 more seconds for follow-up frames.
//!
//! Run with:
//!   cargo run -p osdl-cli --example espnow_probe --features osdl-core/espnow
//!
//! Override via env vars:
//!   OSDL_GATEWAY_PORT=/dev/cu.usbserial-XXXX \
//!   OSDL_CHILD_ID=pump-01 \
//!   cargo run -p osdl-cli --example espnow_probe --features osdl-core/espnow

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
    log::info!(
        "discovered {} = {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        child_id,
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let child = EspNowChildTransport::new(mac, client.clone());
    child.start().await.map_err(|e| e.to_string())?;

    log::info!(
        "probe ready — listening for frames from {} for 5s ...",
        child_id
    );

    // Phase 1: passive listen
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut rx_count = 0usize;
    while let Ok(Some(frame)) =
        tokio::time::timeout_at(deadline, rx.recv()).await
    {
        rx_count += 1;
        log::info!(
            "[probe rx #{}] transport_id={} {}B  {}",
            rx_count,
            frame.transport_id,
            frame.data.len(),
            hex(&frame.data),
        );
    }
    log::info!("listen phase done — {} frames received", rx_count);

    if rx_count == 0 {
        log::warn!(
            "no frames received in 5s. Is the child board powered + on channel 1? \
             Is the gateway flashed with `espnow_gateway`?"
        );
    }

    // Phase 2: downstream send
    let payload = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE];
    log::info!(
        "sending downstream {}B to {} via gateway: {}",
        payload.len(),
        child_id,
        hex(&payload)
    );
    child.send(&payload).await?;

    // Phase 3: listen again for follow-up frames
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while let Ok(Some(frame)) =
        tokio::time::timeout_at(deadline, rx.recv()).await
    {
        rx_count += 1;
        log::info!(
            "[probe rx #{}] transport_id={} {}B  {}",
            rx_count,
            frame.transport_id,
            frame.data.len(),
            hex(&frame.data),
        );
    }

    log::info!("probe done. total frames received = {}", rx_count);
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
