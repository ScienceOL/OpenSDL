//! Stage 1b — slow, large, reversible round-trip for each Emm V5.0 motor
//! so the movement is both visible and audible.
//!
//! Rationale: the small 100-pulse @ 30 RPM nudge completed in ~60 ms and the
//! user didn't see any movement. This version enlarges the move to 800
//! pulses (~1/4 turn) and slows it to 5 RPM so each move takes ~3 s, which
//! should be trivially visible on the stirrer and trivially audible on a
//! closed-loop stepper even at small displacements.
//!
//! Per motor, sequence:
//!   1. enable(true)
//!   2. get_position (pos_start)
//!   3. run_position 800 pulses forward (dir=0) at 5 RPM
//!   4. 4 s pause — USER WATCHES / LISTENS
//!   5. get_position (pos_mid)
//!   6. run_position 800 pulses reverse (dir=1) at 5 RPM
//!   7. 4 s pause
//!   8. get_position (pos_end) — should be ~= pos_start
//!
//! Net encoder Δ should be near 0. If Δ != 0 and the axis didn't move
//! mechanically, the encoder is reporting commanded-not-actual — which
//! points at firmware config rather than a wiring issue.
//!
//! Run:
//!   cargo run -p osdl-cli --example emm_motor_roundtrip_via_espnow \
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
const OBSERVE_PAUSE: Duration = Duration::from_secs(4);
const BETWEEN_MOTORS: Duration = Duration::from_secs(3);

/// 800 pulses ≈ 1/4 rev at 3200 ppr default. Big enough to see, still a
/// safe <90° rotation so a valve can't swing past "fully open".
const MOVE_PULSES: u64 = 800;
/// 5 RPM is slow enough that 800 pulses takes ~3 s — very visible.
const MOVE_SPEED_RPM: u64 = 5;

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
        "STAGE 1b — round-trip {} pulses @ {} RPM per motor (dir 0 then dir 1)",
        MOVE_PULSES, MOVE_SPEED_RPM
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
        log::info!("========== MOTOR {} — WATCH and LISTEN ==========", id);

        send_and_log(&child, cfg, &mut rx, "enable", serde_json::json!({ "enable": true }), *id).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;

        let pos_start = send_and_log(
            &child, cfg, &mut rx, "get_position", serde_json::json!({}), *id,
        ).await?;

        log::info!("[motor {}] ► FORWARD move: {} pulses @ {} RPM", id, MOVE_PULSES, MOVE_SPEED_RPM);
        send_and_log(
            &child, cfg, &mut rx, "run_position",
            serde_json::json!({
                "pulses": MOVE_PULSES,
                "speed":  MOVE_SPEED_RPM,
                "direction": 0u64,
                "acceleration": 10u64,
                "absolute": false,
            }),
            *id,
        ).await?;
        log::info!("[motor {}] watching for {:?} ...", id, OBSERVE_PAUSE);
        tokio::time::sleep(OBSERVE_PAUSE).await;
        drain(&mut rx, Duration::from_millis(50)).await;

        let pos_mid = send_and_log(
            &child, cfg, &mut rx, "get_position", serde_json::json!({}), *id,
        ).await?;

        log::info!("[motor {}] ◄ REVERSE move: {} pulses @ {} RPM", id, MOVE_PULSES, MOVE_SPEED_RPM);
        send_and_log(
            &child, cfg, &mut rx, "run_position",
            serde_json::json!({
                "pulses": MOVE_PULSES,
                "speed":  MOVE_SPEED_RPM,
                "direction": 1u64,
                "acceleration": 10u64,
                "absolute": false,
            }),
            *id,
        ).await?;
        log::info!("[motor {}] watching for {:?} ...", id, OBSERVE_PAUSE);
        tokio::time::sleep(OBSERVE_PAUSE).await;
        drain(&mut rx, Duration::from_millis(50)).await;

        let pos_end = send_and_log(
            &child, cfg, &mut rx, "get_position", serde_json::json!({}), *id,
        ).await?;

        match (pos_start, pos_mid, pos_end) {
            (Some(a), Some(b), Some(c)) => {
                log::info!(
                    "[motor {}] positions: start={} mid={} end={} | fwd Δ={} | net Δ={}",
                    id, a, b, c, b - a, c - a
                );
            }
            _ => log::warn!(
                "[motor {}] could not read all three positions",
                id
            ),
        }
    }

    log::info!("STAGE 1b complete — did you see/hear EITHER motor move?");
    client.stop().await.ok();
    Ok(())
}

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
        if frame.data.len() == 8 && frame.data[0] != id {
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
                    id, hex(&frame.data),
                    serde_json::to_string(&props).unwrap_or_default()
                );
            }
            None => log::info!(
                "[motor {}] reply (not for this device_id): {}",
                id, hex(&frame.data)
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
