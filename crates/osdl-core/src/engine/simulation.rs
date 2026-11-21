use super::*;
impl EngineHandle {
    pub(super) async fn register_simulation_device(
        &self,
        config: &crate::config::SimulationConfig,
        world: &Arc<crate::simulation::world::WorldRuntime>,
        device_config: &crate::config::SimulationDeviceConfig,
    ) -> Result<(), String> {
        let transport_id = simulation_transport_id(&config.world_id, &device_config.id);
        let backend_engine = config.engine.clone();
        let transport = Arc::new(SimulationTransport::for_device(
            world.clone(),
            device_config.id.clone(),
        ));

        self.register_transport(transport_id.clone(), transport)
            .await;

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
        if actions.iter().all(|action| action.name != "reset") {
            actions.push(ActionSchema {
                name: "reset".into(),
                description: "Restore this virtual device to its configured initial state".into(),
                params: serde_json::json!({"type":"object","properties":{}}),
            });
        }
        self.register_device(Device {
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
        })
        .await?;
        Ok(())
    }

    pub async fn apply_simulation_scene(
        &self,
        config: Option<&crate::config::SimulationConfig>,
        poses: &[crate::simulation::world::DevicePose],
    ) -> Result<(), String> {
        let guard = self.simulation_world.write().await;
        let world = guard.as_ref().ok_or("simulation unavailable")?;
        if let Some(config) = config {
            let (removed, added) = world
                .reconcile(config, self.simulation_backend_factory.as_ref())
                .await?;
            for id in removed {
                let device_id = world.device_id(&id);
                self.devices.write().await.remove(&device_id);
                self.transports
                    .write()
                    .await
                    .remove(&simulation_transport_id(&config.world_id, &id));
                self.emit(OsdlEvent::DeviceOffline { device_id });
            }
            for device in config.devices.iter().filter(|c| added.contains(&c.id)) {
                self.register_simulation_device(config, world, device)
                    .await?;
            }
            world.publish(&added).await;
        }
        world.apply_layout(poses).await;
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
            self.handle
                .register_simulation_device(&config, &world, device_config)
                .await?;
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
