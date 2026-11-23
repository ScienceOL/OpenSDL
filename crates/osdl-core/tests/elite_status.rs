use std::{net::Ipv4Addr, time::Duration};

use osdl_core::{CommandStatus, DeviceCommand, OsdlConfig, OsdlEngine};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

#[tokio::test]
async fn configured_elite_reads_one_vendor_snapshot_without_sending_control() {
    let dashboard = TcpListener::bind((Ipv4Addr::LOCALHOST, 29999))
        .await
        .unwrap();
    let rtsi = TcpListener::bind((Ipv4Addr::LOCALHOST, 30004))
        .await
        .unwrap();
    let dashboard_task = tokio::spawn(async move {
        for _ in 0..5 {
            let (mut socket, _) = dashboard.accept().await.unwrap();
            socket.write_all(concat!(
                "EliRobot dashboard shell, version 2.14.5.0.\r\n",
                "These shell commands are defined internally.  Type `help` to see this list.\r\n",
                "Type `help name` to find out more about the function `name`.\r\n",
                "Type `usage name` to displays the usage for given command or all commands if none is specified.\r\n",
            ).as_bytes()).await.unwrap();
            let mut query = [0; 9];
            socket.read_exact(&mut query).await.unwrap();
            assert_eq!(&query, b"robot -t\n");
            socket.write_all(b"ES66\r\n").await.unwrap();
            assert_eq!(socket.read(&mut query).await.unwrap(), 0);
        }
    });
    let rtsi_task = tokio::spawn(async move {
        for recipe_id in [1, 1, 1, 2, 1] {
            let (mut stream, _) = rtsi.accept().await.unwrap();
            mock_rtsi(&mut stream, recipe_id).await;
        }
    });

    let config: OsdlConfig =
        serde_yaml::from_str("elite_robots:\n  - id: test-elite\n    host: 127.0.0.1\n").unwrap();
    let mut engine = OsdlEngine::new(config, vec![]);
    let handle = engine.handle();
    let engine_task = tokio::spawn(async move { engine.run().await });
    let device = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(device) = handle.get_device("test-elite").await {
                if device.online {
                    break device;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the configured robot should be online after a complete sample");
    assert_eq!(device.device_type, "elite.es66");
    assert_eq!(device.actions.len(), 1);
    assert_eq!(device.actions[0].name, "status");
    assert!(!device.properties.contains_key("tcp_pose"));

    let command = |action: &str, params| DeviceCommand {
        command_id: action.into(),
        device_id: device.id.clone(),
        action: action.into(),
        params,
    };
    assert!(handle
        .send_command(command("move_joints", json!({})))
        .await
        .is_err());
    assert!(handle
        .send_command(command("status", json!({"payload": "raw script"})))
        .await
        .is_err());

    let another = DeviceCommand {
        command_id: "concurrent-status".into(),
        ..command("status", json!({}))
    };
    let (result, concurrent) = tokio::join!(
        handle.send_command(command("status", json!({}))),
        handle.send_command(another),
    );
    let result = result.unwrap();
    let concurrent = concurrent.unwrap();
    assert_eq!(concurrent.command_id, "concurrent-status");
    assert_eq!(concurrent.data, result.data);
    assert_eq!(result.status, CommandStatus::Succeeded);
    assert_eq!(result.command_id, "status");
    assert_eq!(result.device_id, "test-elite");
    let data = result.data.unwrap();
    assert_eq!(
        data["identity"],
        json!({"model":"ES66","controller_version":"2.14.5.1236"})
    );
    assert_eq!(
        data["state"]["robot_mode"],
        json!({"code":7,"name":"running"})
    );
    assert_eq!(
        data["state"]["safety_status"],
        json!({"code":3,"name":"protective_stop"})
    );
    assert_eq!(
        data["state"]["program_state"],
        json!({"code":3,"name":"stopped"})
    );
    assert_eq!(
        data["state"]["joint_modes"][0],
        json!({"code":253,"name":"running"})
    );
    assert_eq!(
        data["state"]["joint_modes"][5],
        json!({"code":0,"name":"unknown"})
    );
    assert_eq!(
        data["tcp_pose"],
        json!({
            "frame":"robot_base", "position_m":[0.3,-0.5,0.77], "rotation_vector_rad":[0.0,0.0,1.5]
        })
    );
    assert_eq!(
        data["joint_positions_rad"],
        json!([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
    );
    assert_eq!(data["controller_timestamp_s"], 100.0);

    let invalid = handle.send_command(command("status", json!({}))).await;
    assert_eq!(
        invalid.unwrap_err(),
        "Elite RTSI sample does not match the output recipe"
    );
    let offline = handle.get_device(&device.id).await.unwrap();
    assert!(!offline.online);
    assert!(offline.properties.is_empty());

    let recovered = handle
        .send_command(command("status", json!({})))
        .await
        .unwrap();
    assert_eq!(recovered.status, CommandStatus::Succeeded);
    assert!(handle.get_device(&device.id).await.unwrap().online);

    handle.request_stop();
    engine_task.await.unwrap();
    timeout(Duration::from_secs(3), dashboard_task)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(3), rtsi_task)
        .await
        .unwrap()
        .unwrap();
}

async fn mock_rtsi(stream: &mut TcpStream, recipe_id: u8) {
    let mut query = [0; 5];
    stream.read_exact(&mut query).await.unwrap();
    assert_eq!(&query, b"\0\x05V\0\x01");
    stream.write_all(b"\0\x04V\x01").await.unwrap();
    assert_eq!(read_frame(stream).await, (b'v', vec![]));
    let version: Vec<_> = [2_i32, 14, 5, 1236]
        .into_iter()
        .flat_map(i32::to_be_bytes)
        .collect();
    write_frame(stream, b'v', &version).await;

    let (kind, payload) = read_frame(stream).await;
    assert_eq!(kind, b'O');
    assert_eq!(f64::from_be_bytes(payload[..8].try_into().unwrap()), 125.0);
    assert_eq!(&payload[8..], b"timestamp,actual_TCP_pose,actual_joint_positions,robot_mode,joint_mode,safety_status,runtime_state");
    write_frame(
        stream,
        b'O',
        b"\x01DOUBLE,VECTOR6D,VECTOR6D,INT32,VECTOR6INT32,INT32,UINT32",
    )
    .await;
    assert_eq!(read_frame(stream).await, (b'S', vec![]));
    let mut sample = vec![recipe_id];
    for value in [
        100.0_f64, 0.3, -0.5, 0.77, 0.0, 0.0, 1.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0,
    ] {
        sample.extend(value.to_be_bytes());
    }
    for code in [7_i32, 253, 253, 253, 253, 253, 0, 3, 3] {
        sample.extend(code.to_be_bytes());
    }
    write_frame(stream, b'S', &[1]).await;
    write_frame(stream, b'U', &sample).await;
    assert_eq!(stream.read(&mut query).await.unwrap(), 0);
}

async fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let size = stream.read_u16().await.unwrap();
    let kind = stream.read_u8().await.unwrap();
    let mut payload = vec![0; usize::from(size) - 3];
    stream.read_exact(&mut payload).await.unwrap();
    (kind, payload)
}

async fn write_frame(stream: &mut TcpStream, kind: u8, payload: &[u8]) {
    let mut frame = Vec::new();
    frame.extend(((payload.len() + 3) as u16).to_be_bytes());
    frame.push(kind);
    frame.extend(payload);
    // Exercise header and payload fragmentation over TCP.
    stream.write_all(&frame[..1]).await.unwrap();
    stream.write_all(&frame[1..]).await.unwrap();
}
