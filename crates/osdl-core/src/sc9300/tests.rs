use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::wire::tests::{response, IDENTITY_PAYLOAD};
use super::{Client, Link, Operation, Session, Station};
use crate::{
    config::Sc9300Config,
    protocol::{CommandStatus, DeviceCommand},
    EngineHandle, OsdlConfig, OsdlEngine,
};

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

    let (handle, station) = station_with_link(link).await;
    let deadline = tokio::time::Instant::now() + super::EXCHANGE_BUDGET;

    station.exchange(Operation::Status, deadline).await.unwrap();
    let before = handle.get_device("test-station").await.unwrap();
    assert!(before.online);
    assert_eq!(before.properties["status"]["command_serial"], 100);

    let identity = handle
        .send_command(command("device_info", json!({})))
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(identity["device_id"], "test-station");
    assert_eq!(identity["controller_device_id"], "00000042");
    assert_eq!(identity["firmware"], "1.3.1.0");
    assert_eq!(identity["properties"]["sc9300_firmware_minor"], 3);
    let after = handle.get_device("test-station").await.unwrap();
    assert_eq!(after.properties, before.properties);
    assert_eq!(
        after.properties_timestamp_ms,
        before.properties_timestamp_ms
    );

    peer.await.unwrap();
    assert!(station.exchange(Operation::Status, deadline).await.is_err());
    let unavailable = handle.get_device("test-station").await.unwrap();
    assert!(!unavailable.online);
    assert!(unavailable.properties.is_empty());
}

#[test]
fn action_payloads_match_reference_controller_vectors() {
    use super::actions::Request;
    use super::wire::Command;

    for line in include_str!("command_vectors.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let mut fields = line.split('\t');
        let action = fields.next().unwrap();
        let params = serde_json::from_str(fields.next().unwrap()).unwrap();
        let expected_command = u16::from_str_radix(fields.next().unwrap(), 16).unwrap();
        let expected_payload = match fields.next().unwrap() {
            "-" => vec![],
            hex_payload => hex::decode(hex_payload).unwrap(),
        };
        assert!(fields.next().is_none());

        let operation = Request::parse(&DeviceCommand {
            command_id: action.into(),
            device_id: "fixture".into(),
            action: action.into(),
            params,
        })
        .unwrap();
        let (command, payload) = match operation {
            Request::Exchange(Operation::Dispatch(command, payload)) => (command, payload),
            Request::Exchange(Operation::CheckLimit(payload)) => (Command::CheckLimit, payload),
            _ => panic!("action {action} did not compile to a serial frame"),
        };
        assert_eq!(command as u16, expected_command, "{action} command");
        assert_eq!(payload, expected_payload, "{action} payload");
    }
}

#[tokio::test]
async fn stop_can_dispatch_while_a_concurrent_wait_is_polling() {
    let (link, mut instrument) = duplex(512);
    let (started, first_poll) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        assert_eq!(request(&mut instrument).await[15..17], [0x01, 0x05]);
        instrument
            .write_all(&response(0x8105, &IDENTITY_PAYLOAD))
            .await
            .unwrap();

        let reset = request(&mut instrument).await;
        assert_eq!(
            &reset[15..29],
            &[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 58, 245]
        );
        instrument.write_all(&response(0x8101, &[1])).await.unwrap();

        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        started.send(()).unwrap();
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x11, 1, 1, 1, 0]))
            .await
            .unwrap();

        assert_eq!(request(&mut instrument).await[15..17], [1, 0]);
        instrument
            .write_all(&response(0x8100, &[0, 0, 0x27, 0x13, 0, 0, 0, 0]))
            .await
            .unwrap();

        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x14, 0, 0, 0, 0]))
            .await
            .unwrap();
    });

    let (handle, _) = station_with_link(link).await;
    let reset = handle
        .send_command(command("reset", json!({})))
        .await
        .unwrap();
    assert_eq!(reset.status, CommandStatus::Pending);
    assert_eq!(
        reset.data.unwrap(),
        json!({"command_serial":10001,"reply":"matching_response"})
    );

    let wait = tokio::spawn({
        let handle = handle.clone();
        async move {
            handle
                .send_command(command("wait_idle", json!({"timeoutSeconds":2.0})))
                .await
        }
    });
    first_poll.await.unwrap();
    let stop = handle
        .send_command(command("stop", json!({})))
        .await
        .unwrap();
    assert_eq!(stop.data.unwrap()["reply"], "matching_response");
    let settled = wait.await.unwrap().unwrap();
    assert_eq!(settled.status, CommandStatus::Succeeded);
    let data = settled.data.unwrap();
    assert_eq!(data["settled"], true);
    assert_eq!(data["device_id"], "test-station");
    assert_eq!(data["properties"]["sc9300_fault_code"], 0);
    peer.await.unwrap();
}

#[tokio::test]
async fn telemetry_reads_exactly_one_status_and_each_matching_head() {
    let (link, mut instrument) = duplex(512);
    let peer = tokio::spawn(async move {
        assert_eq!(request(&mut instrument).await[15..17], [1, 5]);
        instrument
            .write_all(&response(0x8105, &IDENTITY_PAYLOAD))
            .await
            .unwrap();

        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x10, 0, 0, 0, 0]))
            .await
            .unwrap();
        for axis in 1..=3 {
            assert_eq!(request(&mut instrument).await[15..18], [1, 3, axis]);
            let mut position = [0; 21];
            position[0] = 0;
            instrument.write_all(&response(0x8103, &[1])).await.unwrap();
            instrument
                .write_all(&response(0x8103, &position))
                .await
                .unwrap();
            position[0] = axis;
            position[1..5].copy_from_slice(&51_200_i32.to_be_bytes());
            position[5..9].copy_from_slice(&(-51_200_i32).to_be_bytes());
            position[9..13].copy_from_slice(&(51_200 * i32::from(axis)).to_be_bytes());
            instrument
                .write_all(&response(0x8103, &position))
                .await
                .unwrap();
        }
    });

    let (handle, _) = station_with_link(link).await;
    let result = handle
        .send_command(command("telemetry", json!({})))
        .await
        .unwrap();
    let data = result.data.unwrap();
    assert_eq!(data["status"]["state"], "success");
    assert_eq!(
        data["heads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|head| (&head["axis"], &head["z_mm"]))
            .collect::<Vec<_>>(),
        [
            (&json!("left"), &json!(12.7)),
            (&json!("right"), &json!(25.4)),
            (&json!("gripper"), &json!(38.1))
        ]
    );
    assert_eq!(data["heads"][0]["x_mm"], 60.0);
    assert_eq!(data["heads"][0]["y_mm"], -60.0);

    assert_eq!(data["device_id"], "test-station");
    assert!(data["timestamp"].as_i64().unwrap() > 0);
    assert_eq!(data["properties"]["sc9300_left_y_mm"], -60.0);
    assert_eq!(data["properties"]["sc9300_gripper_z_mm"], 38.1);
    assert_eq!(data["properties"]["sc9300_command_serial"], 10000);
    assert_eq!(data["properties"]["sc9300_fault_code"], 0);
    let device = handle.get_device("test-station").await.unwrap();
    assert_eq!(device.properties["status"], data["status"]);
    peer.await.unwrap();
}

#[tokio::test]
async fn a_limit_ack_is_not_a_reachability_readback() {
    let (link, mut instrument) = duplex(512);
    let peer = tokio::spawn(async move {
        assert_eq!(request(&mut instrument).await[15..17], [1, 5]);
        instrument
            .write_all(&response(0x8105, &IDENTITY_PAYLOAD))
            .await
            .unwrap();
        assert_eq!(request(&mut instrument).await[15..18], [1, 9, 2]);
        instrument.write_all(&response(0x8109, &[1])).await.unwrap();
    });

    let (handle, _) = station_with_link(link).await;
    let result = handle
        .send_command(command(
            "check_limit",
            json!({"axis":"right","x":0,"y":0,"z":0}),
        ))
        .await
        .unwrap();
    assert_eq!(result.status, CommandStatus::Pending);
    let data = result.data.unwrap();
    assert!(data["reachable"].is_null());
    assert_eq!(data["dispatch"]["reply"], "matching_response");
    peer.await.unwrap();
}

#[tokio::test]
async fn unrelated_ack_does_not_confirm_a_motion_and_a_silent_command_does_not_retry() {
    let (link, mut instrument) = duplex(512);
    let peer = tokio::spawn(async move {
        assert_eq!(request(&mut instrument).await[15..17], [1, 5]);
        instrument
            .write_all(&response(0x8105, &IDENTITY_PAYLOAD))
            .await
            .unwrap();
        assert_eq!(request(&mut instrument).await[15..17], [1, 2]);
        instrument.write_all(&response(0x8100, &[1])).await.unwrap();

        assert_eq!(request(&mut instrument).await[15..18], [1, 4, 1]);
        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x13, 0, 0, 0xff, 0x11]))
            .await
            .unwrap();
    });

    let (handle, _) = station_with_link(link).await;
    let moved = handle
        .send_command(command("move", json!({"axis":"left","x":0,"y":0,"z":0})))
        .await
        .unwrap();
    assert_eq!(moved.status, CommandStatus::Pending);
    assert_eq!(moved.data.unwrap()["reply"], "no_response");

    let motors = handle
        .send_command(command("set_motors", json!({"enabled":true})))
        .await
        .unwrap();
    assert_eq!(motors.status, CommandStatus::Pending);
    assert_eq!(motors.data.unwrap()["reply"], "not_expected");

    let failed = handle
        .send_command(command("wait_idle", json!({"timeoutSeconds":1})))
        .await
        .unwrap();
    assert_eq!(failed.status, CommandStatus::Failed);
    assert_eq!(failed.data.unwrap()["status"]["error_code"], 0x11);
    peer.await.unwrap();
}

fn command(action: &str, params: Value) -> DeviceCommand {
    DeviceCommand {
        command_id: action.into(),
        device_id: "test-station".into(),
        action: action.into(),
        params,
    }
}

async fn request(instrument: &mut DuplexStream) -> Vec<u8> {
    let mut header = [0; 4];
    instrument.read_exact(&mut header).await.unwrap();
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let mut data = Vec::from(header);
    data.resize(length + 20, 0);
    instrument.read_exact(&mut data[4..]).await.unwrap();
    data
}

async fn station_with_link(link: DuplexStream) -> (Arc<EngineHandle>, Arc<Station>) {
    let config = Sc9300Config {
        id: "test-station".into(),
        port: String::new(),
    };
    let engine = OsdlEngine::new(
        OsdlConfig {
            sc9300_instruments: vec![config.clone()],
            ..Default::default()
        },
        vec![],
    );
    let handle = Arc::new(engine.handle());
    let station = Arc::new(Station::new(&config, String::new(), &handle));
    handle
        .register_transport(station.transport_id.clone(), station.clone())
        .await;
    handle.register_device(station.device()).await.unwrap();
    let mut client = Client::new(Box::new(link) as Box<dyn Link>);
    let identity = client.identify(None).await.unwrap();
    station.state.lock().await.session = Some(Session { client, identity });
    (handle, station)
}

#[tokio::test]
async fn wait_idle_stops_at_the_caller_deadline_during_a_stalled_status_exchange() {
    use std::time::Duration;

    let (link, mut instrument) = duplex(512);
    let peer = tokio::spawn(async move {
        assert_eq!(request(&mut instrument).await[15..17], [1, 5]);
        instrument
            .write_all(&response(0x8105, &IDENTITY_PAYLOAD))
            .await
            .unwrap();
        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x10, 1, 1, 1, 0]))
            .await
            .unwrap();
        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        let mut extra = [0; 1];
        assert_eq!(instrument.read(&mut extra).await.unwrap(), 0);
    });

    let (handle, _) = station_with_link(link).await;
    let still_running = tokio::time::timeout(
        Duration::from_secs(2),
        handle.send_command(command("wait_idle", json!({"timeoutSeconds":1}))),
    )
    .await
    .expect("wait_idle must not inherit the longer serial read deadline")
    .unwrap();
    assert_eq!(still_running.status, CommandStatus::Running);
    assert_eq!(still_running.data.unwrap()["settled"], false);
    assert!(!handle.get_device("test-station").await.unwrap().online);
    peer.await.unwrap();
}

#[tokio::test]
async fn disconnect_after_a_write_keeps_the_dispatch_serial_and_withdraws_online_state() {
    let (link, mut instrument) = duplex(512);
    let peer = tokio::spawn(async move {
        assert_eq!(request(&mut instrument).await[15..17], [1, 5]);
        instrument
            .write_all(&response(0x8105, &IDENTITY_PAYLOAD))
            .await
            .unwrap();
        assert_eq!(request(&mut instrument).await[15..17], [0, 0]);
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x10, 0, 0, 0, 0]))
            .await
            .unwrap();
        assert_eq!(request(&mut instrument).await[15..17], [1, 2]);
    });

    let (handle, _) = station_with_link(link).await;
    handle
        .send_command(command("status", json!({})))
        .await
        .unwrap();
    assert!(handle.get_device("test-station").await.unwrap().online);
    let result = handle
        .send_command(command("move", json!({"axis":"left","x":0,"y":0,"z":0})))
        .await
        .unwrap();
    assert_eq!(result.status, CommandStatus::Pending);
    assert_eq!(
        result.data.unwrap(),
        json!({"command_serial":10002,"reply":"connection_lost"})
    );
    assert!(!handle.get_device("test-station").await.unwrap().online);
    peer.await.unwrap();
}
