use std::collections::HashMap;
use std::net::SocketAddr;
use std::thread;

/// Embedded MQTT broker using rumqttd.
///
/// Spawns a lightweight MQTT v4 broker in a background thread.
/// Child nodes (ESP32) and the mother's own MQTT client both connect to it.
pub struct EmbeddedBroker {
    port: u16,
}

impl EmbeddedBroker {
    /// Start the embedded MQTT broker on the given port.
    /// This spawns a background thread and returns immediately.
    pub fn start(port: u16) -> Result<Self, String> {
        let addr: SocketAddr = format!("0.0.0.0:{}", port)
            .parse()
            .map_err(|e| format!("invalid address: {}", e))?;

        let mut v4 = HashMap::new();
        v4.insert(
            "osdl".to_string(),
            rumqttd::ServerSettings {
                name: "osdl-v4".to_string(),
                listen: addr,
                tls: None,
                next_connection_delay_ms: 1,
                connections: rumqttd::ConnectionSettings {
                    connection_timeout_ms: 5000,
                    max_payload_size: 20480,
                    max_inflight_count: 100,
                    auth: None,
                    external_auth: None,
                    dynamic_filters: true,
                },
            },
        );

        let config = rumqttd::Config {
            id: 0,
            router: rumqttd::RouterConfig {
                max_connections: 100,
                max_outgoing_packet_count: 200,
                max_segment_size: 104857600, // 100 MB
                max_segment_count: 10,
                ..Default::default()
            },
            v4: Some(v4),
            v5: None,
            ws: None,
            cluster: None,
            console: None,
            bridge: None,
            prometheus: None,
            metrics: None,
        };

        let mut broker = rumqttd::Broker::new(config);

        thread::Builder::new()
            .name("osdl-mqtt-broker".to_string())
            .spawn(move || {
                if let Err(e) = broker.start() {
                    log::error!("MQTT broker error: {}", e);
                }
            })
            .map_err(|e| format!("failed to spawn broker thread: {}", e))?;

        log::info!("Embedded MQTT broker started on port {}", port);
        Ok(EmbeddedBroker { port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}
