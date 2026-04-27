//! Compound device operations built on top of the engine.
//!
//! The orchestrator coordinates multi-step sequences across devices that
//! share a bus. It uses `engine.send_command()` for individual operations
//! and polls device properties for completion.
//!
//! Examples:
//! - Safe XYZ move: Z up → wait → XY move → wait → Z down → wait
//! - Aspirate: move to well → lower Z → aspirate → raise Z
//!
//! Does NOT modify the engine core — purely a composition layer.

use crate::engine::OsdlEngine;
use crate::protocol::DeviceCommand;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_POLL_INTERVAL_MS: u64 = 100;

pub struct Orchestrator {
    engine: Arc<OsdlEngine>,
    /// Interval between status polls.
    poll_interval: Duration,
}

impl Orchestrator {
    pub fn new(engine: Arc<OsdlEngine>) -> Self {
        Self {
            engine,
            poll_interval: Duration::from_millis(DEFAULT_POLL_INTERVAL_MS),
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    fn make_cmd(&self, device_id: &str, action: &str, params: serde_json::Value) -> DeviceCommand {
        DeviceCommand {
            command_id: format!(
                "orch-{}-{}",
                action,
                Instant::now().elapsed().as_nanos()
            ),
            device_id: device_id.to_string(),
            action: action.to_string(),
            params,
        }
    }

    // === Laiyu XYZ stepper motors ===

    /// Poll a Laiyu XYZ motor until it reports "standby".
    pub async fn wait_laiyu_standby(
        &self,
        device_id: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let start = Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(format!("timeout waiting for {} standby", device_id));
            }

            // Send get_status query
            let cmd = self.make_cmd(device_id, "get_status", serde_json::json!({}));
            let _ = self.engine.send_command(cmd).await;

            tokio::time::sleep(self.poll_interval).await;

            if let Some(device) = self.engine.get_device(device_id).await {
                if device.properties.get("status").and_then(|v| v.as_str()) == Some("standby") {
                    return Ok(());
                }
            }
        }
    }

    /// Move a single Laiyu XYZ axis: write target → start motion → wait standby.
    pub async fn move_laiyu_axis(
        &self,
        device_id: &str,
        position: i32,
        speed: u16,
        accel: u16,
        timeout: Duration,
    ) -> Result<(), String> {
        // Write target position + speed + acceleration registers
        let move_cmd = self.make_cmd(
            device_id,
            "move_to_position",
            serde_json::json!({
                "position": position,
                "speed": speed,
                "acceleration": accel,
            }),
        );
        self.engine.send_command(move_cmd).await?;

        // Small delay for register write to complete
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Trigger motion
        let start_cmd = self.make_cmd(device_id, "start_motion", serde_json::json!({}));
        self.engine.send_command(start_cmd).await?;

        // Wait for motor to finish
        self.wait_laiyu_standby(device_id, timeout).await
    }

    /// Safe compound XYZ move: Z up to safe height → XY move → Z down to target.
    ///
    /// Prevents collisions by always raising Z first. On a half-duplex RS-485 bus,
    /// axes are moved sequentially (X then Y), not in parallel.
    pub async fn move_xyz_safe(
        &self,
        x_device: &str,
        y_device: &str,
        z_device: &str,
        target_x: i32,
        target_y: i32,
        target_z: i32,
        safe_z: i32,
        speed: u16,
        accel: u16,
        timeout: Duration,
    ) -> Result<(), String> {
        // 1. Z up to safe height
        log::info!("XYZ safe move: Z up to {}", safe_z);
        self.move_laiyu_axis(z_device, safe_z, speed, accel, timeout)
            .await?;

        // 2. X move
        log::info!("XYZ safe move: X to {}", target_x);
        self.move_laiyu_axis(x_device, target_x, speed, accel, timeout)
            .await?;

        // 3. Y move
        log::info!("XYZ safe move: Y to {}", target_y);
        self.move_laiyu_axis(y_device, target_y, speed, accel, timeout)
            .await?;

        // 4. Z down to target
        log::info!("XYZ safe move: Z down to {}", target_z);
        self.move_laiyu_axis(z_device, target_z, speed, accel, timeout)
            .await?;

        log::info!("XYZ safe move complete");
        Ok(())
    }

    // === Emm V5.0 stepper motors ===

    /// Poll an Emm motor until its position stabilizes (3 consecutive identical readings).
    pub async fn wait_emm_stable(
        &self,
        device_id: &str,
        timeout: Duration,
    ) -> Result<i64, String> {
        let start = Instant::now();
        let mut last_position: Option<i64> = None;
        let mut stable_count: u32 = 0;

        loop {
            if start.elapsed() > timeout {
                return Err(format!(
                    "timeout waiting for {} position to stabilize",
                    device_id
                ));
            }

            let cmd = self.make_cmd(device_id, "get_position", serde_json::json!({}));
            let _ = self.engine.send_command(cmd).await;

            tokio::time::sleep(self.poll_interval).await;

            if let Some(device) = self.engine.get_device(device_id).await {
                if let Some(pos) = device.properties.get("position").and_then(|v| v.as_i64()) {
                    if last_position == Some(pos) {
                        stable_count += 1;
                        if stable_count >= 3 {
                            return Ok(pos);
                        }
                    } else {
                        stable_count = 0;
                    }
                    last_position = Some(pos);
                }
            }
        }
    }

    /// Run an Emm motor to a position and wait for it to stop.
    pub async fn run_emm_position(
        &self,
        device_id: &str,
        pulses: u32,
        speed: u16,
        direction: u8,
        accel: u8,
        absolute: bool,
        timeout: Duration,
    ) -> Result<i64, String> {
        let cmd = self.make_cmd(
            device_id,
            "run_position",
            serde_json::json!({
                "pulses": pulses,
                "speed": speed,
                "direction": direction,
                "acceleration": accel,
                "absolute": absolute,
            }),
        );
        self.engine.send_command(cmd).await?;

        self.wait_emm_stable(device_id, timeout).await
    }

    // === Generic helpers ===

    /// Send a command and return immediately (no waiting).
    pub async fn fire(&self, device_id: &str, action: &str, params: serde_json::Value) -> Result<(), String> {
        let cmd = self.make_cmd(device_id, action, params);
        self.engine.send_command(cmd).await?;
        Ok(())
    }

    /// Send a command, wait a fixed duration, then read a property from the device.
    ///
    /// Useful for query commands (e.g., read_level, query_status) where we just
    /// need to wait for the response to arrive and update properties.
    pub async fn query(
        &self,
        device_id: &str,
        action: &str,
        params: serde_json::Value,
        property: &str,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let cmd = self.make_cmd(device_id, action, params);
        self.engine.send_command(cmd).await?;

        let start = Instant::now();
        loop {
            tokio::time::sleep(self.poll_interval).await;

            if let Some(device) = self.engine.get_device(device_id).await {
                if let Some(val) = device.properties.get(property) {
                    return Ok(val.clone());
                }
            }

            if start.elapsed() > timeout {
                return Err(format!(
                    "timeout waiting for property '{}' on {}",
                    property, device_id
                ));
            }
        }
    }
}
