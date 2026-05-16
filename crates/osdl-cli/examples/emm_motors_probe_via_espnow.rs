//! Safe read-only probe for the two Emm V5.0 rotor motors (device_id 4, 5)
//! sharing the same RS-485 bus as the three Runze pumps, behind one ESP-NOW
//! child.
//!
//! This example sends **non-moving** commands only:
//!   1. `get_position`  — pure query, motor does not move.
//!   2. `enable(true)`  — energizes the driver so the shaft holds torque,
//!                        but the motor does not rotate.
//! It never sends `run_speed` / `run_position`. That keeps the test safe
//! even if the drain valve is not in a benign position.
//!
//! Run:
//!   cargo run -p osdl-cli --example emm_motors_probe_via_espnow \
//!       --features osdl-core/espnow
//!
//! Success criterion: both motors 4 and 5 return an 8-byte frame starting
//! with their own device_id in response to `get_position`. Anything shorter
//! or a timeout means the motor didn't answer — most commonly a baud-rate
//! mismatch (child firmware UART is 9600) or wiring/power issue.
//!
//! Caveat on baud rate: `espnow_child.rs` hard-codes UART1 at 9600, which
//! matches the Runze pumps we just verified. Emm V5.0 modules can also be
//! ordered/configured at 115200 — if motors 4/5 don't answer but the pumps
//! do, that's the first thing to check.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::driver::builtins::emm::{self, EmmConfig};
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

const REPLY_WINDOW: Duration = Duration::from_millis(600);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let child_id = env::var("OSDL_CHILD_ID")
        .unwrap_or_else(|_| "syringe_pump_with_valve.runze.SY03B-T06".to_string());

    let motors: Vec<(u8, EmmConfig)> = [4u8, 5u8]
        .iter()
        .map(|id| (*id, EmmConfig { device_id: *id }))
        .collect();
    log::info!("configured 2 Emm codecs at device_id 4, 5 (read-only probe)");

    // --- Gateway + child transport (shared bus with the pumps) ---
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

    drain(&mut rx, Duration::from_millis(50)).await;

    let actions: &[(&str, serde_json::Value)] = &[
        ("get_position", serde_json::json!({})),
        ("enable", serde_json::json!({ "enable": true })),
    ];

    let mut any_replies = false;

    for (id, cfg) in &motors {
        log::info!("--- motor {} ---", id);
        for (action, params) in actions {
            let cmd = DeviceCommand {
                command_id: format!("cmd-m{}-{}", id, action),
                device_id: format!("motor-{}", id),
                action: (*action).to_string(),
                params: params.clone(),
            };
            let bytes = emm::encode(cfg, &cmd)
                .map_err(|e| format!("encode motor {} {}: {}", id, action, e))?;
            log::info!("[motor {}] send {}: {}", id, action, hex(&bytes));
            child.send(&bytes).await?;

            let replies = collect(&mut rx, REPLY_WINDOW).await;
            if replies.is_empty() {
                log::warn!(
                    "[motor {}] no reply within {:?} for {}",
                    id, REPLY_WINDOW, action
                );
                continue;
            }
            for frame in replies {
                // 8-byte child heartbeat (counter + uptime) — skip.
                if frame.data.len() == 8 && frame.data[0] <= 0x7F {
                    // Heuristic: Emm get_position reply is also 8 bytes and
                    // starts with the device_id (4 or 5). If first byte is 4
                    // or 5 we fall through to the decoder; otherwise assume
                    // it's the child's heartbeat and drop it.
                    if frame.data[0] != *id {
                        log::debug!("[motor {}] heartbeat: {}", id, hex(&frame.data));
                        continue;
                    }
                }
                any_replies = true;
                match emm::decode(cfg, &frame.data) {
                    Some(props) => log::info!(
                        "[motor {}] reply: {} → {}",
                        id,
                        hex(&frame.data),
                        serde_json::to_string(&props).unwrap_or_default()
                    ),
                    None => log::info!(
                        "[motor {}] reply (undecoded, not for this device_id): {}",
                        id,
                        hex(&frame.data)
                    ),
                }
            }
        }
    }

    log::info!(
        "done — {}",
        if any_replies {
            "at least one motor answered"
        } else {
            "NO motor replied (check baud rate + wiring)"
        }
    );
    client.stop().await.ok();
    Ok(())
}

async fn drain(rx: &mut mpsc::UnboundedReceiver<TransportRx>, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(_)) = tokio::time::timeout_at(deadline, rx.recv()).await {}
}

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
