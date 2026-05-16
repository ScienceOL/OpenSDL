//! Drive three Runze syringe pumps (addresses 1, 2, 3) sharing a single
//! RS-485 bus behind one ESP-NOW child. Scenario A from CLAUDE.md:
//!
//! ```text
//!   Mac (this example)
//!     │  3 × RunzeDriver   (address "1", "2", "3")
//!     ▼
//!   EspNowChildTransport  (single bus, single MAC)
//!     │
//!     ▼  ESP-NOW → child → UART1 → TD501D485H-A → RS-485 bus
//!     │
//!     ├── pump #1 (listens for "/1...")
//!     ├── pump #2 (listens for "/2...")
//!     └── pump #3 (listens for "/3...")
//! ```
//!
//! Run:
//!   cargo run -p osdl-cli --example runze_three_pumps_via_espnow \
//!       --features osdl-core/espnow
//!
//! Env overrides (see `runze_via_espnow.rs` for details):
//!   OSDL_GATEWAY_PORT=/dev/cu.usbserial-XXXX
//!   OSDL_CHILD_ID=syringe_pump_with_valve.runze.SY03B-T06
//!
//! Success criterion: each of the three pumps answers its `initialize`
//! query with a Runze reply frame. You'll see lines like:
//!   [pump 1] reply: FF 2F 30 40 03 0D 0A → status=Idle
//! for each of pumps 1, 2, 3. If a pump is missing from the bus, its
//! "initialize" window will simply time out with no reply — the other
//! pumps are unaffected.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::driver::builtins::runze::{self, RunzeConfig};
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

/// How long to wait for a reply after each command before moving on.
const REPLY_WINDOW: Duration = Duration::from_millis(600);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let child_id = env::var("OSDL_CHILD_ID")
        .unwrap_or_else(|_| "syringe_pump_with_valve.runze.SY03B-T06".to_string());

    // --- Three Runze configs, one per bus address ---
    // The Runze ASCII protocol carries the address in the first byte after
    // '/', so three pumps can share a single 485 bus. We invoke the
    // driver's pure encode/decode functions directly — no registry needed.
    let configs: Vec<(&str, RunzeConfig)> = ["1", "2", "3"]
        .iter()
        .map(|addr| {
            let cfg = RunzeConfig {
                address: (*addr).to_string(),
                ..RunzeConfig::default()
            };
            (*addr, cfg)
        })
        .collect();
    log::info!("configured 3 Runze codecs at addresses 1, 2, 3");

    // --- Bring up the gateway + per-child transport (one bus for all pumps) ---
    let (tx, mut rx) = mpsc::unbounded_channel::<TransportRx>();
    let client = Arc::new(EspNowGatewayClient::new(port.clone(), 115200, tx));
    client.start().await.map_err(|e| e.to_string())?;

    log::info!("waiting up to 15s for REG <{}> ...", child_id);
    let mac = client
        .wait_for_registration(&child_id, Duration::from_secs(15))
        .await?;
    log::info!(
        "discovered {} = {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        child_id, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    let child = EspNowChildTransport::new(mac, client.clone());
    child.start().await.map_err(|e| e.to_string())?;

    // Drain any telemetry/replies that queued up while REG was settling —
    // otherwise the first pump's reply-window would see stale 8-byte
    // counter frames from the child's heartbeat.
    drain(&mut rx, Duration::from_millis(50)).await;

    // --- Probe each pump with initialize + query_position ---
    let actions = &["initialize", "query_position"];
    let mut any_replies = false;

    for (addr, cfg) in &configs {
        log::info!("--- pump {} ---", addr);
        for action in actions {
            let cmd = DeviceCommand {
                command_id: format!("cmd-{}-{}", addr, action),
                device_id: format!("pump-{}", addr),
                action: action.to_string(),
                params: serde_json::json!({}),
            };
            let bytes = runze::encode(cfg, &cmd)
                .map_err(|e| format!("encode pump {} {}: {}", addr, action, e))?;
            log::info!(
                "[pump {}] send {}: {} | {}",
                addr,
                action,
                String::from_utf8_lossy(&bytes)
                    .replace('\r', "\\r")
                    .replace('\n', "\\n"),
                hex(&bytes)
            );
            child.send(&bytes).await?;

            // Collect replies in the reply window. On a shared bus any pump
            // *could* answer, but in practice only the addressed pump does;
            // we decode with this pump's driver so status/position are scoped
            // correctly to the command we just sent.
            let replies = collect(&mut rx, REPLY_WINDOW).await;
            if replies.is_empty() {
                log::warn!(
                    "[pump {}] no reply within {:?} — pump may be absent or at a different address",
                    addr, REPLY_WINDOW
                );
                continue;
            }
            for frame in replies {
                // Heuristic: telemetry frames from the child are exactly 8 bytes
                // (little-endian counter + uptime). Skip those so we only
                // report real pump replies.
                if frame.data.len() == 8 {
                    log::debug!("[pump {}] heartbeat: {}", addr, hex(&frame.data));
                    continue;
                }
                any_replies = true;
                match runze::decode(cfg, &frame.data) {
                    Some(props) => log::info!(
                        "[pump {}] reply: {}  → {}",
                        addr,
                        hex(&frame.data),
                        serde_json::to_string(&props).unwrap_or_default()
                    ),
                    None => log::info!(
                        "[pump {}] reply (undecoded): {}",
                        addr,
                        hex(&frame.data)
                    ),
                }
            }
        }
    }

    log::info!(
        "done — {} (any pump answered? {})",
        if any_replies {
            "bus exchange successful"
        } else {
            "no pump replied"
        },
        any_replies
    );
    client.stop().await.ok();
    Ok(())
}

/// Drain the RX channel for `window`, discarding whatever arrives.
async fn drain(rx: &mut mpsc::UnboundedReceiver<TransportRx>, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(_)) = tokio::time::timeout_at(deadline, rx.recv()).await {}
}

/// Collect every frame that arrives within `window`.
async fn collect(
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    window: Duration,
) -> Vec<TransportRx> {
    let deadline = tokio::time::Instant::now() + window;
    let mut out = Vec::new();
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        out.push(frame);
    }
    out
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
