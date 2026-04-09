use crate::config::MqttConfig;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use std::time::Duration;

pub struct MqttBridge {
    pub client: AsyncClient,
    pub eventloop: EventLoop,
}

impl MqttBridge {
    pub fn new(config: &MqttConfig) -> Self {
        let mut opts = MqttOptions::new(&config.client_id, &config.host, config.port);
        opts.set_keep_alive(Duration::from_secs(config.keepalive_secs));

        let (client, eventloop) = AsyncClient::new(opts, 256);
        Self { client, eventloop }
    }

    pub async fn subscribe(&self, topic: &str) -> Result<(), rumqttc::ClientError> {
        self.client.subscribe(topic, QoS::AtLeastOnce).await
    }

    pub async fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        retain: bool,
    ) -> Result<(), rumqttc::ClientError> {
        self.client
            .publish(topic, QoS::AtLeastOnce, retain, payload)
            .await
    }
}
