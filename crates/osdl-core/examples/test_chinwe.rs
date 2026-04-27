//! 硬件测试: ChinWe 分液工作站 (TCP/WiFi)
//!
//! 用法:
//!   cargo run --example test_chinwe
//!   cargo run --example test_chinwe -- --host 192.168.31.201 --port 8899
//!
//! 默认: 192.168.31.201:8899
//! 测试内容: 读取液位传感器, 查询注射泵状态, 读取电机位置

use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::tcp::TcpTransport;
use osdl_core::transport::{Transport, TransportRx};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let host = arg_value(&args, "--host").unwrap_or_else(|| "192.168.31.13".into());
    let port: u16 = arg_value(&args, "--port")
        .and_then(|s| s.parse().ok())
        .unwrap_or(8899);
    let registry_path = arg_value(&args, "--registry");

    println!("=== ChinWe 分液工作站硬件测试 ===");
    println!("TCP: {}:{}", host, port);
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

    // 连接 TCP
    let (rx_tx, mut rx_rx) = mpsc::unbounded_channel::<TransportRx>();
    let transport = TcpTransport::new(host.clone(), port, rx_tx);
    match transport.start().await {
        Ok(()) => println!("TCP 已连接\n"),
        Err(e) => {
            println!("TCP 连接失败: {}", e);
            println!("请确认 ChinWe 设备已开机且 WiFi 可达");
            return;
        }
    }

    // 测试 1: XKC 液位传感器 (Modbus RTU, slave 6)
    println!("--- XKC 液位传感器 (slave 6) ---");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "sensor.chinwe.xkc",
        "read_level",
        serde_json::json!({}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 测试 2: 注射泵 1 状态查询 (Runze ASCII, addr 1)
    println!("--- 注射泵 1 (addr 1) ---");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "syringe_pump.chinwe.pump1",
        "query_status",
        serde_json::json!({}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 测试 3: Emm 电机 4 读位置 (binary, id 4)
    println!("--- Emm 电机 4 (id 4) ---");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "stepper_motor.chinwe.emm4",
        "get_position",
        serde_json::json!({}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 测试 4: Emm 电机 5 读位置 (binary, id 5)
    println!("--- Emm 电机 5 (id 5) ---");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "stepper_motor.chinwe.emm5",
        "get_position",
        serde_json::json!({}),
    )
    .await;

    transport.stop().await.ok();
    println!("\n=== 测试完成 ===");
}

async fn send_and_decode(
    adapter: &UniLabOsAdapter,
    transport: &TcpTransport,
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    device_type: &str,
    action: &str,
    params: serde_json::Value,
) {
    let cmd = DeviceCommand {
        command_id: format!("test-{}", action),
        device_id: "test".into(),
        action: action.into(),
        params,
    };

    match adapter.encode_command(device_type, &cmd) {
        Ok(bytes) => {
            println!("  发送 {}: {:02X?}", action, bytes);
            if let Err(e) = transport.send(&bytes).await {
                println!("  发送失败: {}", e);
                return;
            }

            match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
                Ok(Some(msg)) => {
                    println!("  收到 {} 字节: {:02X?}", msg.data.len(), msg.data);
                    match adapter.decode_response(device_type, &msg.data) {
                        Some(props) => {
                            println!(
                                "  解码: {}",
                                serde_json::to_string_pretty(&props).unwrap()
                            );
                        }
                        None => println!("  解码失败 (地址不匹配或帧格式错误)"),
                    }
                }
                Ok(None) => println!("  通道关闭"),
                Err(_) => println!("  超时 (3s)"),
            }
        }
        Err(e) => println!("  编码失败: {}", e),
    }
    println!();
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
