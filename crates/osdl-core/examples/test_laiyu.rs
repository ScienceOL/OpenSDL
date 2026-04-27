//! 硬件测试: Laiyu XYZ 移液工作站 (RS-485 串口)
//!
//! 用法:
//!   cargo run --example test_laiyu --features serial
//!   cargo run --example test_laiyu --features serial -- --port /dev/ttyUSB1
//!
//! 默认: /dev/ttyUSB0, 115200 baud
//! 测试内容: 读取 X/Y/Z 三轴状态, 查询移液器枪头

use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::direct_serial::DirectSerialTransport;
use osdl_core::transport::{Transport, TransportRx};
use std::time::Duration;
use tokio::sync::mpsc;

const BAUD: u32 = 115200;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "/dev/ttyUSB0".into());
    let registry_path = args
        .iter()
        .position(|a| a == "--registry")
        .and_then(|i| args.get(i + 1))
        .cloned();

    println!("=== Laiyu XYZ 硬件测试 ===");
    println!("串口: {} @ {} baud", port, BAUD);
    println!();

    // 加载 registry
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    if let Some(ref path) = registry_path {
        adapter
            .load_registry(path)
            .unwrap_or_else(|e| panic!("无法加载 registry {}: {}", path, e));
    } else {
        adapter
            .load_registry("registry/unilabos")
            .or_else(|_| adapter.load_registry("../../registry/unilabos"))
            .expect("无法加载 registry (尝试了 registry/unilabos 和 ../../registry/unilabos, 可用 --registry 指定路径)");
    }

    // 打开串口
    let (rx_tx, mut rx_rx) = mpsc::unbounded_channel::<TransportRx>();
    let transport = DirectSerialTransport::new(port.clone(), BAUD, rx_tx);
    transport.start().await.expect("无法打开串口");
    println!("串口已打开\n");

    // 用于收集响应的 helper
    async fn recv_response(rx: &mut mpsc::UnboundedReceiver<TransportRx>) -> Option<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .ok()
            .flatten()
            .map(|r| r.data)
    }

    // 测试 1: 读取三轴状态
    for (device_type, axis) in [
        ("stepper_motor.laiyu_xyz.X", "X"),
        ("stepper_motor.laiyu_xyz.Y", "Y"),
        ("stepper_motor.laiyu_xyz.Z", "Z"),
    ] {
        let cmd = DeviceCommand {
            command_id: format!("test-{}", axis),
            device_id: format!("motor-{}", axis),
            action: "get_status".into(),
            params: serde_json::json!({}),
        };

        match adapter.encode_command(device_type, &cmd) {
            Ok(bytes) => {
                println!("[{}轴] 发送 get_status: {:02X?}", axis, bytes);
                transport.send(&bytes).await.expect("发送失败");

                match recv_response(&mut rx_rx).await {
                    Some(data) => {
                        println!("[{}轴] 收到 {} 字节: {:02X?}", axis, data.len(), data);
                        match adapter.decode_response(device_type, &data) {
                            Some(props) => {
                                println!("[{}轴] 解码结果: {}", axis, serde_json::to_string_pretty(&props).unwrap());
                            }
                            None => println!("[{}轴] 解码失败 (可能是其他设备的响应)", axis),
                        }
                    }
                    None => println!("[{}轴] 超时未收到响应", axis),
                }
            }
            Err(e) => println!("[{}轴] 编码失败: {}", axis, e),
        }
        println!();

        // RS-485 半双工, 等一下再发下一条
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 测试 2: 查询移液器枪头
    println!("--- 移液器 ---");
    let cmd = DeviceCommand {
        command_id: "test-pip".into(),
        device_id: "pipette".into(),
        action: "query_tip".into(),
        params: serde_json::json!({}),
    };
    match adapter.encode_command("pipette.sopa.YYQ", &cmd) {
        Ok(bytes) => {
            println!("[移液器] 发送 query_tip: {:02X?}", bytes);
            transport.send(&bytes).await.expect("发送失败");

            match recv_response(&mut rx_rx).await {
                Some(data) => {
                    println!("[移液器] 收到: {:02X?}", data);
                    match adapter.decode_response("pipette.sopa.YYQ", &data) {
                        Some(props) => println!("[移液器] 解码: {}", serde_json::to_string_pretty(&props).unwrap()),
                        None => println!("[移液器] 解码失败"),
                    }
                }
                None => println!("[移液器] 超时 (移液器可能未连接)"),
            }
        }
        Err(e) => println!("[移液器] 编码失败: {}", e),
    }

    transport.stop().await.ok();
    println!("\n=== 测试完成 ===");
}

