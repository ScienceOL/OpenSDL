//! Drain the separatory funnel: open motor-5 (drain valve), hold open for
//! 10 seconds, close it. Deliberately conservative pulse count (800 ≈ 1/4
//! revolution) matching the rotations we already saw work in the
//! round-trip test. Does NOT touch motor-4 (stirrer).
//!
//! Run:
//!   cargo run -p osdl-cli --example drain_funnel_via_espnow \
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
const OPEN_HOLD: Duration = Duration::from_secs(15);

const DRAIN_VALVE_ID: u8 = 5;
const PULSES: u64 = 900;
const SPEED_RPM: u64 = 5;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let child_id = env::var("OSDL_CHILD_ID")
        .unwrap_or_else(|_| "syringe_pump_with_valve.runze.SY03B-T06".to_string());

    let cfg = EmmConfig { device_id: DRAIN_VALVE_ID };

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

    drain_ch(&mut rx, Duration::from_millis(50)).await;

    // Ensure driver is energized so the move actually happens.
    send_and_log(&child, &cfg, &mut rx, "enable", serde_json::json!({ "enable": true })).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    log::info!(">>> OPENING DRAIN VALVE ({} pulses @ {} RPM)", PULSES, SPEED_RPM);
    send_and_log(
        &child, &cfg, &mut rx, "run_position",
        serde_json::json!({
            "pulses": PULSES,
            "speed":  SPEED_RPM,
            "direction": 0u64,
            "acceleration": 10u64,
            "absolute": false,
        }),
    ).await?;

    log::info!("holding valve OPEN for {:?} ...", OPEN_HOLD);
    tokio::time::sleep(OPEN_HOLD).await;

    drain_ch(&mut rx, Duration::from_millis(50)).await;

    log::info!("<<< CLOSING DRAIN VALVE ({} pulses @ {} RPM)", PULSES, SPEED_RPM);
    send_and_log(
        &child, &cfg, &mut rx, "run_position",
        serde_json::json!({
            "pulses": PULSES,
            "speed":  SPEED_RPM,
            "direction": 1u64,
            "acceleration": 10u64,
            "absolute": false,
        }),
    ).await?;

    // Give the close motion a beat to complete before we drop the transport.
    tokio::time::sleep(Duration::from_secs(4)).await;
    drain_ch(&mut rx, Duration::from_millis(50)).await;

    log::info!("drain cycle complete");
    client.stop().await.ok();
    Ok(())
}

async fn send_and_log(
    child: &EspNowChildTransport,
    cfg: &EmmConfig,
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    action: &str,
    params: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let cmd = DeviceCommand {
        command_id: format!("cmd-m5-{}", action),
        device_id: "motor-5".into(),
        action: action.to_string(),
        params,
    };
    let bytes = emm::encode(cfg, &cmd).map_err(|e| format!("encode {}: {}", action, e))?;
    log::info!("[motor 5] send {}: {}", action, hex(&bytes));
    child.send(&bytes).await?;

    let replies = collect(rx, REPLY_WINDOW).await;
    for frame in replies {
        if frame.data.len() == 8 && frame.data[0] != DRAIN_VALVE_ID {
            continue;
        }
        match emm::decode(cfg, &frame.data) {
            Some(props) => log::info!(
                "[motor 5] reply: {} → {}",
                hex(&frame.data),
                serde_json::to_string(&props).unwrap_or_default()
            ),
            None => log::info!("[motor 5] reply (undecoded): {}", hex(&frame.data)),
        }
    }
    Ok(())
}

async fn drain_ch(rx: &mut mpsc::UnboundedReceiver<TransportRx>, window: Duration) {
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
