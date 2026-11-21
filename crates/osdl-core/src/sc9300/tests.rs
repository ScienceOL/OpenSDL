use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

use super::wire::tests::{response, IDENTITY_PAYLOAD};
use super::{Client, Link, Operation, Session, Station};
use crate::{config::Sc9300Config, OsdlConfig, OsdlEngine};

#[tokio::test]
async fn identity_read_preserves_status_and_a_failed_follow_up_withdraws_it() {
    let (link, mut instrument) = duplex(512);
    let peer = tokio::spawn(async move {
        let mut request = [0; 20];
        for (read, reply) in [
            (0x0105u16, response(0x8105, &IDENTITY_PAYLOAD)),
            (0x0000, response(0x8000, &[0, 0, 0, 100, 1, 5, 0, 0])),
            (0x0105, response(0x8105, &IDENTITY_PAYLOAD)),
        ] {
            instrument.read_exact(&mut request).await.unwrap();
            assert_eq!(request[15..17], read.to_be_bytes());
            instrument.write_all(&reply).await.unwrap();
        }
    });

    let engine = OsdlEngine::new(OsdlConfig::default(), vec![]);
    let handle = std::sync::Arc::new(engine.handle());
    let config = Sc9300Config {
        id: "test-station".into(),
        port: String::new(),
    };
    let station = std::sync::Arc::new(Station::new(&config, String::new(), &handle));
    handle
        .register_transport(station.transport_id.clone(), station.clone())
        .await;
    handle.register_device(station.device()).await.unwrap();
    let mut client = Client::new(Box::new(link) as Box<dyn Link>);
    let identity = client.identify(None).await.unwrap();
    station.state.lock().await.session = Some(Session { client, identity });

    station.read(Operation::Status).await.unwrap();
    let before = handle.get_device(&config.id).await.unwrap();
    assert!(before.online);
    assert_eq!(before.properties["status"]["command_serial"], 100);

    station.read(Operation::DeviceInfo).await.unwrap();
    let after = handle.get_device(&config.id).await.unwrap();
    assert_eq!(after.properties, before.properties);
    assert_eq!(
        after.properties_timestamp_ms,
        before.properties_timestamp_ms
    );

    peer.await.unwrap();
    assert!(station.read(Operation::Status).await.is_err());
    let unavailable = handle.get_device(&config.id).await.unwrap();
    assert!(!unavailable.online);
    assert!(unavailable.properties.is_empty());
}
