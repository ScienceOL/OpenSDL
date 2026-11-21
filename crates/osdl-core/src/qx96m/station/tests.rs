use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use super::*;
use crate::event::OsdlEvent;
use crate::OsdlEngine;

const KEY: &[u8; 8] = b"FIXTURE!";

#[tokio::test]
async fn station_uses_one_session_for_observation_and_typed_reads() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
        panic!("fixture did not bind IPv4");
    };
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut link = BufReader::new(socket);
        let mut request = Vec::new();
        let identity = json!({
            "PCRName": "Fixture station",
            "RowsCount": 8,
            "ColumnCount": 12,
            "SN": "SIM-01",
            "SoftVersion": "1.0",
            "private_metadata": "fixture should not escape"
        });
        let reads = [
            (1, 0, true, identity.to_string()),
            (2, 3, false, "PCR仪器上无正在运行的实验!".to_owned()),
            (3, 3, true, json!({"ID":"00000000000000000000000000000001","private_metadata": "fixture should not escape"}).to_string()),
        ];
        for (id, kind, result, data) in reads {
            request.clear();
            assert!(link.read_until(b'\n', &mut request).await.unwrap() > 0);
            if id == 1 {
                // Go crypto/des and cipher.NewCBCEncrypter applied to the canonical
                // source request with the synthetic FIXTURE! key.
                assert_eq!(
                    &request,
                    concat!(
                        "C374B7B1905D254C3B95EA17358690B24EA24C2DA8F27F0DD5792CAF544FA6527",
                        "1D1C21B1DB288E427E4A8D36A83FA58A001BEFDDD73150FF09193E05E3EA0681BE",
                        "ACCF754BA183C\r\n"
                    )
                    .as_bytes()
                );
            }
            let reply = serde_json::to_string(
                &json!({"ID":id,"Type":kind,"State":0,"Result":result,"Data":data}),
            )
            .unwrap();
            let (encoded, _, errors) = encoding_rs::GBK.encode(&reply);
            assert!(!errors);
            link.get_mut()
                .write_all(&super::super::wire::encode(&encoded, KEY))
                .await
                .unwrap();
        }
    });

    let handle = OsdlEngine::new(Default::default(), vec![Box::new(Adapter)]).handle();
    let mut events = handle.subscribe_events();
    let station = Arc::new(Station {
        id: "jlm-qpcr".into(),
        transport_id: "qx96m:jlm-qpcr".into(),
        client: Client::new(endpoint, *KEY),
        online: AtomicBool::new(false),
    });
    handle
        .register_transport(station.transport_id.clone(), station.clone())
        .await;
    handle.register_device(station.device()).await.unwrap();
    assert!(!handle.get_device(&station.id).await.unwrap().online);
    assert!(events.try_recv().is_err());
    let monitor = tokio::spawn({
        let station = station.clone();
        let handle = handle.clone();
        async move { station.monitor(handle).await }
    });
    let OsdlEvent::DeviceOnline(device) = time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("station did not publish its observed identity");
    };
    assert_eq!(device.properties["identity"]["serial_number"], "SIM-01");
    assert_eq!(device.properties["state"], "idle");
    assert_eq!(
        device
            .actions
            .iter()
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>(),
        ["device_info", "run_status"]
    );

    let call = |action: &str| {
        handle.send_command(DeviceCommand {
            command_id: format!("fixture-{action}"),
            device_id: station.id.clone(),
            action: action.into(),
            params: json!({}),
        })
    };
    assert_eq!(
        call("device_info").await.unwrap().data.unwrap()["serial_number"],
        "SIM-01"
    );
    let error = call("undeclared_action").await.unwrap_err();
    assert_eq!(error, "unsupported QX96M read");
    let status = call("run_status").await.unwrap();
    assert_eq!(status.command_id, "fixture-run_status");
    assert_eq!(status.data.unwrap(), json!({"state":"active"}));
    peer.await.unwrap();
    monitor.abort();
    station.client.stop().await;
}

#[test]
fn key_file_requires_exact_printable_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("qx96m.key");
    std::fs::write(&file, KEY).unwrap();
    assert_eq!(load_key(&file).unwrap(), *KEY);
    std::fs::write(&file, b"FIXTURE!\n").unwrap();
    assert!(load_key(&file).is_err());
    std::fs::write(&file, b"FIXTURE\0").unwrap();
    assert!(load_key(&file).is_err());
    assert!(load_key(dir.path()).is_err());
}
