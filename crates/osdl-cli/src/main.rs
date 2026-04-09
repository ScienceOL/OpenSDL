use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::config::{AdapterConfig, MqttConfig, OsdlConfig};
use osdl_core::OsdlEngine;

#[tokio::main]
async fn main() {
    env_logger::init();

    // TODO: load config from file / CLI args
    let config = OsdlConfig {
        mqtt: MqttConfig::default(),
        adapters: vec![AdapterConfig {
            adapter_type: "unilabos".into(),
            gateway_id: "default".into(),
            registry_path: Some("../../registry/unilabos".into()),
        }],
    };

    let adapters: Vec<Box<dyn osdl_core::adapter::PlatformAdapter>> =
        vec![Box::new(UniLabOsAdapter::new("default"))];

    let mut engine = OsdlEngine::new(config, adapters);

    log::info!("Starting OpenSDL...");
    engine.run().await;
}
