//! Stir motor-4 for 10 seconds at 60 RPM, then stop. One-shot script for
//! a demo recording. Scheduled for deletion when the per-action examples
//! get consolidated into a single `osdl-probe` CLI.
//!
//! Run:
//!   cargo run -p osdl-cli --example stir_10s --features osdl-core/espnow

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::driver::builtins::emm::{self, EmmConfig};
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

const REPLY_WINDOW: Duration = Duration::from_millis(600);
const STIR_DURATION: Duration = Duration::from_secs(10);

const STIRRER_ID: u8 = 4;
const RPM: u64 = 60;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let child_id = env::var("OSDL_CHILD_ID")
        .unwrap_or_else(|_| "syringe_pump_with_valve.runze.SY03B-T06".to_string());

    let cfg = EmmConfig { device_id: STIRRER_ID };

    let (tx, mut rx) = mpsc::unbounded_channel::<TransportRx>();
    let client = Arc::new(EspNowGatewayClient::new(port.clone(), 115200, tx));
    client.start().await.map_err(|e| e.to_string())?;

    log::info!("waiting up to 15s for REG <{}> ...", child_id);
    let mac = client
        .wait_for_registration(&child_id, Duration::from_secs(15))
        .await?;
    let child = EspNowChildTransport::new(mac, client.clone());
    child.start().await.map_err(|e| e.to_string())?;

    drain_ch(&mut rx, Duration::from_millis(50)).await;

    send_and_log(&child, &cfg, &mut rx, "enable", serde_json::json!({ "enable": true })).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    log::info!(">>> STIRRING at {} RPM for {:?}", RPM, STIR_DURATION);
    send_and_log(
        &child, &cfg, &mut rx, "run_speed",
        serde_json::json!({
            "speed": RPM,
            "direction": 0u64,
            "acceleration": 10u64,
        }),
    ).await?;

    tokio::time::sleep(STIR_DURATION).await;

    log::info!("<<< STOP");
    send_and_log(&child, &cfg, &mut rx, "stop", serde_json::json!({})).await?;

    tokio::time::sleep(Duration::from_secs(1)).await;
    drain_ch(&mut rx, Duration::from_millis(50)).await;
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
        command_id: format!("cmd-m4-{}", action),
        device_id: "motor-4".into(),
        action: action.to_string(),
        params,
    };
    let bytes = emm::encode(cfg, &cmd).map_err(|e| format!("encode {}: {}", action, e))?;
    log::info!("[motor 4] send {}: {}", action, hex(&bytes));
    child.send(&bytes).await?;

    for frame in collect(rx, REPLY_WINDOW).await {
        if frame.data.len() == 8 && frame.data[0] != STIRRER_ID {
            continue;
        }
        match emm::decode(cfg, &frame.data) {
            Some(props) => log::info!(
                "[motor 4] reply: {} → {}",
                hex(&frame.data),
                serde_json::to_string(&props).unwrap_or_default()
            ),
            None => log::info!("[motor 4] reply (undecoded): {}", hex(&frame.data)),
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
