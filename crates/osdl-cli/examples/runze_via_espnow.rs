//! End-to-end message-layer proof: Mac-side `OsdlEngine` stack encodes a
//! Runze `DeviceCommand`, routes it through `EspNowChildTransport`, over
//! the gateway, across ESP-NOW, to the child — which displays the bytes on
//! its ST7796 screen.
//!
//! Scope: **message layer only**. The child does not yet bridge to a real
//! RS-485 device, so there is no upstream response to parse. Success
//! criterion is seeing `2F 31 5A 52 0D 0A` (`/1ZR\r\n`) in the child's RX
//! area on the LCD — that proves the Runze driver's encoded ASCII reached
//! the physical child board via the full OpenSDL stack.
//!
//! Flow:
//! ```text
//!   DeviceCommand { action: "initialize" }
//!     │
//!     ▼
//!   ProtocolAdapter::encode_command("syringe_pump_with_valve.runze.SY03B-T06", cmd)
//!     │  (RunzeDriver: /1ZR\r\n = 2F 31 5A 52 0D 0A)
//!     ▼
//!   EspNowChildTransport::send(&bytes)
//!     │
//!     ▼
//!   EspNowGatewayClient::send_to_mac(MAC, bytes)
//!     │  "TX 30EDA0B65B38 2F315A520D0A\n" on USB-CDC
//!     ▼
//!   Gateway ESP32 → ESP-NOW radio (broadcast w/ dst MAC header)
//!     ▼
//!   Child ESP32 callback filters by MAC → forwards payload to display
//!
//! Run (gateway + child already flashed + powered):
//!   cargo run -p osdl-cli --example runze_via_espnow --features osdl-core/espnow
//!
//! Override defaults:
//!   OSDL_GATEWAY_PORT=/dev/cu.usbserial-XXXX \
//!   OSDL_CHILD_ID=pump-01 \
//!   OSDL_DEVICE_TYPE=syringe_pump_with_valve.runze.SY03B-T06 \
//!   cargo run -p osdl-cli --example runze_via_espnow --features osdl-core/espnow

use std::env;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::adapter::{unilabos::UniLabOsAdapter, ProtocolAdapter};
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient};
use osdl_core::transport::{Transport, TransportRx};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let child_id = env::var("OSDL_CHILD_ID").unwrap_or_else(|_| "pump-01".to_string());
    let device_type = env::var("OSDL_DEVICE_TYPE")
        .unwrap_or_else(|_| "syringe_pump_with_valve.runze.SY03B-T06".to_string());
    let registry_path = env::var("OSDL_REGISTRY_PATH")
        .unwrap_or_else(|_| "registry/unilabos".to_string());

    // --- Build the ProtocolAdapter (Runze driver is selected via device_type in registry) ---
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter
        .load_registry(&registry_path)
        .map_err(|e| format!("load_registry({}) failed: {}", registry_path, e))?;
    log::info!("adapter ready, device_type={}", device_type);

    // --- Bring up the gateway + per-child transport ---
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

    let child = EspNowChildTransport::new(child_id.clone(), mac, client.clone());
    child.start().await.map_err(|e| e.to_string())?;

    // --- Drive a few commands through the full stack ---
    // Each DeviceCommand is what a CLI / HTTP / MQTT caller would submit in prod.
    let commands = vec![
        DeviceCommand {
            command_id: "cmd-1".into(),
            device_id: child_id.clone(),
            action: "initialize".into(),
            params: serde_json::json!({}),
        },
        DeviceCommand {
            command_id: "cmd-2".into(),
            device_id: child_id.clone(),
            action: "query_position".into(),
            params: serde_json::json!({}),
        },
        DeviceCommand {
            command_id: "cmd-3".into(),
            device_id: child_id.clone(),
            action: "set_position".into(),
            params: serde_json::json!({ "position": 2.5 }),
        },
    ];

    for cmd in &commands {
        let bytes = adapter
            .encode_command(&device_type, cmd)
            .map_err(|e| format!("encode {} failed: {}", cmd.action, e))?;
        log::info!(
            "cmd {} ({}): {}  |  bytes: {}",
            cmd.command_id,
            cmd.action,
            String::from_utf8_lossy(&bytes)
                .replace('\r', "\\r")
                .replace('\n', "\\n"),
            hex(&bytes)
        );
        child.send(&bytes).await?;
        // Let the child's display paint before we send the next one.
        tokio::time::sleep(Duration::from_millis(800)).await;
    }

    // --- Drain any telemetry that arrived during the run (non-blocking) ---
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut rx_count = 0usize;
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        rx_count += 1;
        log::info!(
            "[probe rx #{}] transport_id={} {}B  {}",
            rx_count,
            frame.transport_id,
            frame.data.len(),
            hex(&frame.data)
        );
    }

    log::info!(
        "done — sent {} Runze commands, received {} frames during run",
        commands.len(),
        rx_count
    );
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
