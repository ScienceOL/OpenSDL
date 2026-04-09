use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::config::{AdapterConfig, MqttConfig, OsdlConfig};
use osdl_core::{EmbeddedBroker, EventStore, MdnsAdvertiser, OsdlEngine};

#[tokio::main]
async fn main() {
    env_logger::init();

    // TODO: load config from file / CLI args
    let config = OsdlConfig {
        mqtt: MqttConfig::default(),
        adapters: vec![AdapterConfig {
            adapter_type: "unilabos".into(),
            registry_path: Some("../../registry/unilabos".into()),
        }],
    };

    // Start embedded MQTT broker
    let _broker = EmbeddedBroker::start(config.mqtt.port).expect("Failed to start MQTT broker");

    // Advertise via mDNS so child nodes can discover us
    let _mdns = MdnsAdvertiser::start(config.mqtt.port).expect("Failed to start mDNS");

    // Give broker a moment to bind the port
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Open event store (SQLite)
    let store = EventStore::open("osdl.db").expect("Failed to open event store");

    let adapters: Vec<Box<dyn osdl_core::adapter::ProtocolAdapter>> =
        vec![Box::new(UniLabOsAdapter::new())];

    let mut engine = OsdlEngine::new(config, adapters).with_store(store);

    // Spawn event consumer
    let event_rx = engine.take_event_rx();
    tokio::spawn(async move {
        let mut rx = event_rx.lock().await.take().unwrap();
        while let Some(event) = rx.recv().await {
            log::info!("Event: {:?}", event);
        }
    });

    log::info!("Starting OpenSDL...");
    engine.run().await;
}
