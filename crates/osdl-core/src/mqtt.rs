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

    /// Split into client + eventloop for separate ownership.
    pub fn split(self) -> (AsyncClient, EventLoop) {
        (self.client, self.eventloop)
    }

    pub async fn subscribe(client: &AsyncClient, topic: &str) -> Result<(), rumqttc::ClientError> {
        client.subscribe(topic, QoS::AtLeastOnce).await
    }

    pub async fn publish(
        client: &AsyncClient,
        topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), rumqttc::ClientError> {
        client
            .publish(topic, QoS::AtLeastOnce, false, payload)
            .await
    }
}
