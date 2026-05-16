//! Live validation for the bus-manifest feature: boots a real `OsdlEngine`
//! with a hard-coded `buses:` entry that lists all 5 devices on the ChinWe
//! station's shared RS-485 bus, waits for the ESP-NOW child to register,
//! and prints the resulting `Device` set so we can confirm the engine
//! really built 5 independently-addressable records — mirroring what the
//! Xyzen Runner will do once the user drops the same manifest into
//! `~/.xyzen/config.yaml`.
//!
//! No commands are sent. The point is purely to verify registration.
//!
//! Run:
//!   cargo run -p osdl-cli --example bus_manifest_live --features osdl-core/espnow

use std::time::Duration;

use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;
use osdl_core::config::{
    AdapterConfig, BusConfig, BusDeviceConfig, EspNowGatewayConfig, OsdlConfig,
};
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::{OsdlEngine, OsdlEvent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port = std::env::var("OSDL_GATEWAY_PORT")
        .unwrap_or_else(|_| "/dev/cu.usbserial-A5069RR4".to_string());
    let registry = std::env::var("OSDL_REGISTRY_PATH")
        .unwrap_or_else(|_| "registry/unilabos".to_string());

    let buses = vec![BusConfig {
        match_hardware_id: "syringe_pump_with_valve.runze.SY03B-T06".into(),
        devices: vec![
            BusDeviceConfig {
                local_id: "pump-1".into(),
                device_type: "syringe_pump.chinwe.pump1".into(),
                role: Some("syringe_pump".into()),
                description: None,
            },
            BusDeviceConfig {
                local_id: "pump-2".into(),
                device_type: "syringe_pump.chinwe.pump2".into(),
                role: Some("syringe_pump".into()),
                description: None,
            },
            BusDeviceConfig {
                local_id: "pump-3".into(),
                device_type: "syringe_pump.chinwe.pump3".into(),
                role: Some("syringe_pump".into()),
                description: None,
            },
            BusDeviceConfig {
                local_id: "motor-4".into(),
                device_type: "stepper_motor.chinwe.emm4".into(),
                role: Some("stirrer".into()),
                description: Some(
                    "Stirrer motor above separatory funnel. \
                     Use run_speed for continuous stirring."
                        .into(),
                ),
            },
            BusDeviceConfig {
                local_id: "motor-5".into(),
                device_type: "stepper_motor.chinwe.emm5".into(),
                role: Some("drain_valve".into()),
                description: Some(
                    "Drain valve on separatory funnel. \
                     run_position ~800 pulses dir=0 opens; dir=1 closes."
                        .into(),
                ),
            },
        ],
    }];

    let config = OsdlConfig {
        mqtt: None,
        adapters: vec![AdapterConfig {
            adapter_type: "unilabos".into(),
            registry_path: Some(registry.clone()),
        }],
        espnow_gateways: vec![EspNowGatewayConfig {
            port: port.clone(),
            baud_rate: 115200,
        }],
        buses,
    };

    let adapters: Vec<Box<dyn ProtocolAdapter>> =
        vec![Box::new(UniLabOsAdapter::new(DriverRegistry::with_builtins()))];

    let mut engine = OsdlEngine::new(config, adapters);
    let event_rx_slot = engine.take_event_rx();
    let stop = engine.stop_handle();

    let engine_task = tokio::spawn(async move { engine.run().await });

    // Drain DeviceOnline events for up to 20 s, collecting each Device.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut online = Vec::new();
    let mut rx = event_rx_slot.lock().await.take().expect("event rx");
    while tokio::time::Instant::now() < deadline && online.len() < 5 {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(OsdlEvent::DeviceOnline(d))) => {
                log::info!(
                    "  + {} — {} ({}) role={:?}",
                    d.id, d.device_type, d.description, d.role,
                );
                online.push(d);
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }

    log::info!(
        "REGISTRATION COMPLETE: {} devices online",
        online.len()
    );
    for d in &online {
        log::info!(
            "    {:<36}  role={:<14}  transport_id={}",
            d.id,
            d.role.as_deref().unwrap_or("-"),
            d.transport_id,
        );
    }

    let _ = stop.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_task).await;

    if online.len() == 5 {
        Ok(())
    } else {
        Err(format!(
            "expected 5 devices, got {} — child REG may not have arrived, \
             or a device_type in the bus manifest doesn't match the registry",
            online.len()
        )
        .into())
    }
}
