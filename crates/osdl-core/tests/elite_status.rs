use std::{net::Ipv4Addr, time::Duration};

use osdl_core::{CommandStatus, DeviceCommand, OsdlConfig, OsdlEngine};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::timeout,
};

static CONTROLLER_PORTS: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn configured_elite_reads_one_vendor_snapshot_without_sending_control() {
    let _ports = CONTROLLER_PORTS.lock().await;
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
    mock_rtsi_pose(stream, recipe_id, 100.0, [0.3, -0.5, 0.77], 3, 3).await;
}

async fn mock_rtsi_pose(
    stream: &mut TcpStream,
    recipe_id: u8,
    timestamp: f64,
    [x, y, z]: [f64; 3],
    safety: i32,
    program: i32,
) {
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
        timestamp, x, y, z, 0.0, 0.0, 1.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0,
    ] {
        sample.extend(value.to_be_bytes());
    }
    for code in [7_i32, 253, 253, 253, 253, 253, 0, safety, program] {
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

#[tokio::test]
async fn motion_requires_the_dedicated_task_and_a_later_fence_and_pose() {
    use std::sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    };
    use tokio::io::{AsyncBufReadExt, BufReader};

    let _ports = CONTROLLER_PORTS.lock().await;
    let host = Ipv4Addr::LOCALHOST;
    let dashboard = TcpListener::bind((host, 29999)).await.unwrap();
    let rtsi = TcpListener::bind((host, 30004)).await.unwrap();
    let interpreter = TcpListener::bind((host, 30020)).await.unwrap();
    let phase = Arc::new(AtomicU8::new(0));

    let active = phase.clone();
    let dashboard_task = tokio::spawn(async move {
        loop {
            let (socket, _) = dashboard.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            socket
                .get_mut()
                .write_all(b"EliRobot dashboard shell, version 2.14.5.0.\r\n")
                .await
                .unwrap();
            let mut query = String::new();
            socket.read_line(&mut query).await.unwrap();
            let reply: &[u8] = match query.as_str() {
                "robot -t\n" => b"ES66\r\n",
                "task\n" if active.load(Ordering::Relaxed) > 0 => {
                    b"Loaded task: /programs/opensdl-xyz.task\r\n"
                }
                "task\n" => b"No task file loaded\r\n",
                other => panic!("unexpected dashboard command {other:?}"),
            };
            socket.get_mut().write_all(reply).await.unwrap();
        }
    });

    let completed = phase.clone();
    let rtsi_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = rtsi.accept().await.unwrap();
            let (stamp, x) = if completed.load(Ordering::Relaxed) == 2 {
                (101.0, 0.31)
            } else {
                (100.0, 0.3)
            };
            mock_rtsi_pose(&mut stream, 1, stamp, [x, -0.5, 0.77], 1, 1).await;
        }
    });
    let completed = phase.clone();
    let interpreter_task = tokio::spawn(async move {
        let (socket, _) = interpreter.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let exchanges = [
            ("state", "state: 0: running: state"),
            ("sync()", "ack: 1: sync()"),
            ("statelastexecuted", "state: 1: statelastexecuted"),
            ("movel([0.310000,-0.500000,0.770000,0.000000,0.000000,1.500000],a=0.1,v=0.05,t=0,r=0)",
                "ack: 2: movel([0.310000,-0.500000,0.770000,0.000000,0.000000,1.500000],a=0.1,v=0.05,t=0,r=0)"),
            ("sync()", "ack: 3: sync()"),
            ("statelastexecuted", "state: 2: statelastexecuted"),
            ("statelastexecuted", "state: 3: statelastexecuted"),
        ];
        for (command, response) in exchanges {
            let mut query = String::new();
            socket.read_line(&mut query).await.unwrap();
            assert_eq!(query, format!("{command}\n"));
            if response.starts_with("state: 3:") {
                completed.store(2, Ordering::Relaxed);
            }
            socket
                .get_mut()
                .write_all(format!("{response}\r\n").as_bytes())
                .await
                .unwrap();
        }
    });

    let config: OsdlConfig = serde_yaml::from_str(concat!(
        "elite_robots:\n  - id: test-motion\n    host: 127.0.0.1\n",
        "    motion_task: opensdl-xyz.task\n",
    ))
    .unwrap();
    let mut engine = OsdlEngine::new(config, vec![]);
    let handle = engine.handle();
    let job = tokio::spawn(async move { engine.run().await });
    let device = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(device) = handle.get_device("test-motion").await {
                if device.online {
                    break device;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(device
        .actions
        .iter()
        .any(|action| action.name == "move_xyz"));
    let command = DeviceCommand {
        command_id: "motion".into(),
        device_id: device.id,
        action: "move_xyz".into(),
        params: json!({"x_m":0.31,"y_m":-0.5,"z_m":0.77}),
    };
    let rejected = handle.send_command(command.clone()).await.unwrap_err();
    assert!(rejected.contains("opensdl-xyz.task"), "{rejected}");
    phase.store(1, Ordering::Relaxed);
    let moved = handle.send_command(command).await.unwrap();
    assert_eq!(moved.status, CommandStatus::Succeeded);
    let data = moved.data.unwrap();
    assert_eq!(data["controller_move_id"], 2);
    assert_eq!(
        data["observed"]["tcp_pose"]["position_m"],
        json!([0.31, -0.5, 0.77])
    );
    assert_eq!(data["observed"]["controller_timestamp_s"], 101.0);
    interpreter_task.await.unwrap();
    job.abort();
    dashboard_task.abort();
    rtsi_task.abort();
    let _ = tokio::join!(job, dashboard_task, rtsi_task);
}
