//! Minimal probe: prove `EspNowGatewayClient` + `EspNowChildTransport` work
//! end-to-end against a real gateway board.
//!
//! What it does:
//!   1. Opens the gateway serial port (default `/dev/cu.usbserial-A5069RR4`).
//!   2. Registers a single child: hardware_id="pump-01", MAC=30:ED:A0:B6:5B:38.
//!   3. Listens for inbound telemetry from the child for 5 seconds, printing each frame.
//!   4. Sends one downstream test frame (`DE AD BE EF CA FE`) to the child.
//!   5. Listens 2 more seconds for any ack / follow-up.
//!
//! Run with:
//!   cargo run -p osdl-cli --example espnow_probe --features osdl-core/espnow
//!
//! Override port / MAC via env vars:
//!   OSDL_GATEWAY_PORT=/dev/cu.usbserial-XXXX \
//!   OSDL_CHILD_MAC=30EDA0B65B38 \
//!   OSDL_CHILD_ID=pump-01 \
//!   cargo run -p osdl-cli --example espnow_probe --features osdl-core/espnow

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::transport::espnow_gateway::{
    parse_mac, EspNowChildTransport, EspNowGatewayClient, Mac,
};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let mac_hex = env::var("OSDL_CHILD_MAC").unwrap_or_else(|_| "30EDA0B65B38".to_string());
    let child_id = env::var("OSDL_CHILD_ID").unwrap_or_else(|_| "pump-01".to_string());

    let mac: Mac = parse_mac(&mac_hex).ok_or_else(|| {
        format!(
            "OSDL_CHILD_MAC must be 12 hex chars (got {:?})",
            mac_hex
        )
    })?;

    let (tx, mut rx) = mpsc::unbounded_channel::<TransportRx>();
    let client = Arc::new(EspNowGatewayClient::new(port.clone(), 115200, tx));
    client.start().await.map_err(|e| e.to_string())?;

    let child = EspNowChildTransport::new(child_id.clone(), mac, client.clone());
    child.start().await.map_err(|e| e.to_string())?;

    log::info!(
        "probe ready — listening for frames from {} (MAC {}) for 5s ...",
        child_id,
        mac_hex
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
