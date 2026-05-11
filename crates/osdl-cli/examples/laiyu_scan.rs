//! Query Laiyu XYZ stepper motors (slave 1/2/3) via ESP-NOW ← → RS-485.
//!
//! Uses the laiyu_xyz driver's `get_status` — Modbus RTU "read holding
//! registers" at 115200 baud. Safe (no motion). Any reply confirms that
//! our RS-485 bus reaches a live Laiyu controller.
//!
//! Run (requires LilyGO firmware with UART_BAUD = 115200):
//!   cargo run -p osdl-cli --example laiyu_scan --features osdl-core/espnow

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

    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter
        .load_registry("registry/unilabos")
        .map_err(|e| format!("load_registry: {}", e))?;

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

    let devices = [
        ("stepper_motor.laiyu_xyz.X", "X"),
        ("stepper_motor.laiyu_xyz.Y", "Y"),
        ("stepper_motor.laiyu_xyz.Z", "Z"),
    ];

    let mut total_replies = 0usize;

    for (device_type, label) in devices.iter() {
        let cmd = DeviceCommand {
            command_id: format!("q-{}", label),
            device_id: "pump-01".into(),
            action: "get_status".into(),
            params: serde_json::json!({}),
        };
        let bytes = adapter
            .encode_command(device_type, &cmd)
            .map_err(|e| format!("encode {}: {}", label, e))?;
        log::info!("→ axis {}: {} B  hex={}", label, bytes.len(), hex(&bytes));
        child.send(&bytes).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if frame.data.len() == 8 {
                continue; // child telemetry heartbeat
            }
            total_replies += 1;
            log::info!("    ← {} B  hex={}", frame.data.len(), hex(&frame.data));
            match adapter.decode_response(device_type, &frame.data) {
                Some(props) => log::info!("    decoded: {:?}", props),
                None => log::info!("    (no decode — may be partial frame or noise)"),
            }
        }
    }

    if total_replies == 0 {
        log::warn!("no replies — pump is not a Laiyu XYZ (Modbus RTU @ 115200)?");
        log::warn!("try the SOPA pipette address too: pipette.sopa.YYQ");
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
