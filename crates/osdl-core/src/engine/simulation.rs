use super::*;
fn simulation_device(
    config: &crate::config::SimulationConfig,
    device_config: &crate::config::SimulationDeviceConfig,
) -> Device {
    let transport_id = simulation_transport_id(&config.world_id, &device_config.id);
    let backend_engine = config.engine.clone();
    let device_id = simulation_device_id(&config.world_id, &device_config.id);
    let mut properties = device_config.properties.clone();
    properties.insert("simulation".into(), serde_json::Value::Bool(true));
    properties.insert(
        "simulation_engine".into(),
        serde_json::Value::String(backend_engine),
    );
    properties.insert(
        "simulation_world".into(),
        serde_json::Value::String(config.world_id.clone()),
    );
    if let Some(asset_ref) = &device_config.asset_ref {
        properties.insert(
            "simulation_asset".into(),
            serde_json::json!({
                "namespace": asset_ref.namespace,
                "name": asset_ref.name,
                "version": asset_ref.version,
            }),
        );
    }
    properties.insert("position".into(), serde_json::json!(device_config.position));

    let description = if device_config.description.is_empty() {
        format!("Virtual {}", device_config.device_type)
    } else {
        device_config.description.clone()
    };
    let mut actions = device_config.actions.clone();
    if device_config.device_type != "simulation.static"
        && actions.iter().all(|action| action.name != "reset")
    {
        actions.push(ActionSchema {
            name: "reset".into(),
            description: "Restore this virtual device to its configured initial state".into(),
            params: serde_json::json!({"type":"object","properties":{}}),
        });
    }
    Device {
        id: device_id,
        transport_id,
        device_type: device_config.device_type.clone(),
        adapter: crate::adapter::simulation::PLATFORM.into(),
        description,
        online: true,
        properties,
        properties_timestamp_ms: now_millis(),
        actions,
        role: device_config.role.clone(),
    }
}

impl EngineHandle {
    pub async fn apply_simulation_scene(
        &self,
        revision: u64,
        config: Option<&crate::config::SimulationConfig>,
        poses: &[crate::simulation::world::DevicePose],
    ) -> Result<(), String> {
        let guard = self.simulation_world.write().await;
        let world = guard.as_ref().ok_or("simulation unavailable")?;
        let mut devices = self.devices.write().await;
        let mut notifications = Vec::new();
        let mut added_ids = std::collections::HashSet::new();
        if let Some(config) = config {
            let (removed, added) = world
                .reconcile(config, self.simulation_backend_factory.as_ref())
                .await?;
            let mut transports = self.transports.write().await;
            for id in removed {
                let device_id = world.device_id(&id);
                devices.remove(&device_id);
                transports.remove(&simulation_transport_id(&config.world_id, &id));
                notifications.push(OsdlEvent::DeviceOffline { device_id });
            }
            for cfg in config.devices.iter().filter(|c| added.contains(&c.id)) {
                let device = simulation_device(config, cfg);
                let transport = SimulationTransport::for_device(world.clone(), cfg.id.clone());
                transports.insert(device.transport_id.clone(), Arc::new(transport));
                devices.insert(device.id.clone(), device);
            }
            added_ids = added;
        }
        for status in world.apply_layout(revision, poses, &added_ids).await {
            if let Some(device) = devices.get_mut(&status.device_id) {
                device.properties = status.properties;
                device.properties_timestamp_ms = status.timestamp;
            }
        }
        for id in added_ids {
            notifications.push(OsdlEvent::DeviceOnline(
                devices[&world.device_id(&id)].clone(),
            ));
        }
        drop(devices);
        self.broadcast_status().await;
        self.emit(OsdlEvent::SimulationSceneChanged { revision });
        for event in notifications {
            self.emit(event);
        }
        Ok(())
    }
}
impl OsdlEngine {
    /// Register the configured local simulation world before entering the
    /// event loop. Virtual devices use the same transport and adapter path as
    /// physical devices, which keeps the Agent/UI contract identical.
    pub(super) async fn start_simulation(&self) -> Result<(), String> {
        let Some(config) = self.handle.config.simulation.clone() else {
            return Ok(());
        };
        config.validate()?;

        let world = crate::simulation::world::WorldRuntime::new(
            &config,
            self.handle.transport_rx_tx.clone(),
            self.handle.simulation_backend_factory.as_ref(),
        )?;
        for device_config in &config.devices {
            let device = simulation_device(&config, device_config);
            let transport =
                SimulationTransport::for_device(world.clone(), device_config.id.clone());
            self.handle
                .register_transport(device.transport_id.clone(), Arc::new(transport))
                .await;
            self.handle.register_device(device).await?;
        }
        world.start().await;
        *self.handle.simulation_world.write().await = Some(world);
        log::info!(
            "Simulation world '{}' started with '{}' backend ({} devices)",
            config.world_id,
            config.engine,
            config.devices.len()
        );
        Ok(())
    }

    pub(super) async fn stop_simulation(&self) {
        if let Some(world) = self.handle.simulation_world.write().await.take() {
            world.stop().await;
        }
    }
}
