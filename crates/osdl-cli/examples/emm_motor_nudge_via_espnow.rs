//! Stage 1 identification: nudge each Emm V5.0 motor (device_id 4, 5) by a
//! small step so the user can eyeball which one is the stirrer (shaft
//! visibly spins a bit) and which is the valve (barely moves, valve
//! rotates slightly).
//!
//! Sequence for each motor:
//!   1. `enable(true)`       — energize driver (hold torque)
//!   2. short pause
//!   3. `run_position` 100 pulses, 30 RPM, direction=0 (relative) — ≈ 11°
//!   4. wait 3 s so observer can look
//!   5. `get_position`       — confirm encoder moved
//!
//! We do NOT send `stop` or `disable` — after the nudge the motor holds
//! position. That's fine: the next stage will pick up from here.
//!
//! Safety: 100 pulses at 30 RPM is ~0.03 rev → valve turns a tiny fraction
//! of a turn, well under the usual 45°–90° "open" threshold, so liquid
//! should not start flowing from this stage alone.
//!
//! Run:
//!   cargo run -p osdl-cli --example emm_motor_nudge_via_espnow \
//!       --features osdl-core/espnow

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::driver::builtins::emm::{self, EmmConfig};
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

const REPLY_WINDOW: Duration = Duration::from_millis(600);
const OBSERVE_PAUSE: Duration = Duration::from_secs(3);
const BETWEEN_MOTORS: Duration = Duration::from_secs(2);

/// Stage 1 move parameters — small, reversible, slow.
const NUDGE_PULSES: u64 = 100;
const NUDGE_SPEED_RPM: u64 = 30;
const NUDGE_DIRECTION: u64 = 0;

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
    log::info!(
        "STAGE 1 — nudging each motor by {} pulses @ {} RPM (dir={})",
        NUDGE_PULSES, NUDGE_SPEED_RPM, NUDGE_DIRECTION
    );

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

    for (idx, (id, cfg)) in motors.iter().enumerate() {
        if idx > 0 {
            log::info!(
                "--- pausing {:?} before motor {} ---",
                BETWEEN_MOTORS, id
            );
            tokio::time::sleep(BETWEEN_MOTORS).await;
            drain(&mut rx, Duration::from_millis(50)).await;
        }
        log::info!("========== MOTOR {} — watch the hardware ==========", id);

        send_and_log(&child, cfg, &mut rx, "enable", serde_json::json!({ "enable": true }), *id).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;

        let pos_before = send_and_log(
            &child,
            cfg,
            &mut rx,
            "get_position",
            serde_json::json!({}),
            *id,
        )
        .await?;

        send_and_log(
            &child,
            cfg,
            &mut rx,
            "run_position",
            serde_json::json!({
                "pulses": NUDGE_PULSES,
                "speed":  NUDGE_SPEED_RPM,
                "direction": NUDGE_DIRECTION,
                "acceleration": 10,
                "absolute": false,
            }),
            *id,
        )
        .await?;

        log::info!(
            "[motor {}] commanded move — watching for {:?} ...",
            id, OBSERVE_PAUSE
        );
        tokio::time::sleep(OBSERVE_PAUSE).await;
        drain(&mut rx, Duration::from_millis(50)).await;

        let pos_after = send_and_log(
            &child,
            cfg,
            &mut rx,
            "get_position",
            serde_json::json!({}),
            *id,
        )
        .await?;

        match (pos_before, pos_after) {
            (Some(a), Some(b)) => {
                let delta = b - a;
                log::info!(
                    "[motor {}] encoder Δ = {} (before={}, after={})",
                    id, delta, a, b
                );
            }
            _ => log::warn!(
                "[motor {}] could not compute encoder Δ (missing position reply)",
                id
            ),
        }
    }

    log::info!("STAGE 1 complete — tell me which motor was stirrer vs valve.");
    client.stop().await.ok();
    Ok(())
}

/// Send one command and log all replies that arrive within REPLY_WINDOW.
/// Returns the first `position` field we see decoded, if any — used to
/// compare encoder state before/after the nudge.
async fn send_and_log(
    child: &EspNowChildTransport,
    cfg: &EmmConfig,
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    action: &str,
    params: serde_json::Value,
    id: u8,
) -> Result<Option<i64>, Box<dyn std::error::Error>> {
    let cmd = DeviceCommand {
        command_id: format!("cmd-m{}-{}", id, action),
        device_id: format!("motor-{}", id),
        action: action.to_string(),
        params,
    };
    let bytes = emm::encode(cfg, &cmd)
        .map_err(|e| format!("encode motor {} {}: {}", id, action, e))?;
    log::info!("[motor {}] send {}: {}", id, action, hex(&bytes));
    child.send(&bytes).await?;

    let replies = collect(rx, REPLY_WINDOW).await;
    let mut position: Option<i64> = None;
    for frame in replies {
        // Skip 8-byte heartbeats that don't start with this motor's id.
        if frame.data.len() == 8 && frame.data[0] != id {
            log::debug!("[motor {}] heartbeat: {}", id, hex(&frame.data));
            continue;
        }
        match emm::decode(cfg, &frame.data) {
            Some(props) => {
                if position.is_none() {
                    if let Some(p) = props.get("position").and_then(|v| v.as_i64()) {
                        position = Some(p);
                    }
                }
                log::info!(
                    "[motor {}] reply: {} → {}",
                    id,
                    hex(&frame.data),
                    serde_json::to_string(&props).unwrap_or_default()
                );
            }
            None => log::info!(
                "[motor {}] reply (not for this device_id): {}",
                id,
                hex(&frame.data)
            ),
        }
    }
    Ok(position)
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
