//! 运动测试: ChinWe 分液工作站 — 电机转 1/4 圈 + 注射泵吸排 (TCP/WiFi)
//!
//! 用法:
//!   cargo run --example move_chinwe
//!   cargo run --example move_chinwe -- --host 192.168.31.13 --port 8899
//!
//! 默认: 192.168.31.13:8899
//! 测试内容:
//!   1. Emm 电机 4 转 1/4 圈 (800 脉冲, 30 RPM, 顺时针)
//!   2. 注射泵 1 初始化 → 吸液 1mL → 排液 1mL

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

    println!("=== ChinWe 运动测试 ===");
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
            .expect("无法加载 registry, 可用 --registry 指定路径");
    }

    // 连接 TCP
    let (rx_tx, mut rx_rx) = mpsc::unbounded_channel::<TransportRx>();
    let transport = TcpTransport::new(host.clone(), port, rx_tx);
    match transport.start().await {
        Ok(()) => println!("TCP 已连接\n"),
        Err(e) => {
            println!("TCP 连接失败: {}", e);
            return;
        }
    }

    // ========== 测试 1: Emm 电机 4 转 1/4 圈 ==========
    println!("--- 电机 4: 转 1/4 圈 (800 脉冲, 30 RPM, 顺时针) ---");

    // 读取初始位置
    let pos_before = send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "stepper_motor.chinwe.emm4",
        "get_position",
        serde_json::json!({}),
    )
    .await;
    if let Some(ref p) = pos_before {
        println!("  初始位置: {}", p.get("position").unwrap_or(&serde_json::json!("?")));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 发送 run_position
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "stepper_motor.chinwe.emm4",
        "run_position",
        serde_json::json!({
            "pulses": 800,
            "speed": 30,
            "direction": 0,
            "acceleration": 10,
            "absolute": false
        }),
    )
    .await;

    // 等待电机完成 (30 RPM, 1/4 圈 ≈ 0.5s, 留余量)
    println!("  等待电机运动...");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 读取结束位置
    let pos_after = send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "stepper_motor.chinwe.emm4",
        "get_position",
        serde_json::json!({}),
    )
    .await;
    if let Some(ref p) = pos_after {
        println!("  结束位置: {}", p.get("position").unwrap_or(&serde_json::json!("?")));
    }
    println!();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ========== 测试 2: 注射泵 1 吸排液 ==========
    println!("--- 注射泵 1: 初始化 → 吸液 1mL → 排液 1mL ---");

    // 初始化
    println!("  [初始化]");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "syringe_pump.chinwe.pump1",
        "initialize",
        serde_json::json!({}),
    )
    .await;
    println!("  等待初始化完成...");
    wait_pump_idle(&adapter, &transport, &mut rx_rx, "syringe_pump.chinwe.pump1", 10).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 切换阀门到位置 1 (吸液口)
    println!("  [阀门 → 位置 1]");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "syringe_pump.chinwe.pump1",
        "set_valve_position",
        serde_json::json!({"position": 1}),
    )
    .await;
    wait_pump_idle(&adapter, &transport, &mut rx_rx, "syringe_pump.chinwe.pump1", 5).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 吸液 1mL (pull_plunger)
    println!("  [吸液 1mL]");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "syringe_pump.chinwe.pump1",
        "pull_plunger",
        serde_json::json!({"volume": 1.0}),
    )
    .await;
    println!("  等待吸液完成...");
    wait_pump_idle(&adapter, &transport, &mut rx_rx, "syringe_pump.chinwe.pump1", 15).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 切换阀门到位置 2 (排液口)
    println!("  [阀门 → 位置 2]");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "syringe_pump.chinwe.pump1",
        "set_valve_position",
        serde_json::json!({"position": 2}),
    )
    .await;
    wait_pump_idle(&adapter, &transport, &mut rx_rx, "syringe_pump.chinwe.pump1", 5).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 排液 1mL (push_plunger)
    println!("  [排液 1mL]");
    send_and_decode(
        &adapter,
        &transport,
        &mut rx_rx,
        "syringe_pump.chinwe.pump1",
        "push_plunger",
        serde_json::json!({"volume": 1.0}),
    )
    .await;
    println!("  等待排液完成...");
    wait_pump_idle(&adapter, &transport, &mut rx_rx, "syringe_pump.chinwe.pump1", 15).await;

    println!("  注射泵测试完成");

    transport.stop().await.ok();
    println!("\n=== 测试完成 ===");
}

/// 轮询注射泵状态, 等待 Idle
async fn wait_pump_idle(
    adapter: &UniLabOsAdapter,
    transport: &TcpTransport,
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    device_type: &str,
    max_retries: u32,
) {
    for i in 0..max_retries {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let props = send_and_decode(adapter, transport, rx, device_type, "query_status", serde_json::json!({})).await;
        if let Some(ref p) = props {
            let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("");
            print!("  轮询 [{}/{}]: {}", i + 1, max_retries, status);
            if status == "Idle" {
                println!(" ✓");
                return;
            }
            println!();
        }
    }
    println!("  ⚠ 超时未等到 Idle");
}

async fn send_and_decode(
    adapter: &UniLabOsAdapter,
    transport: &TcpTransport,
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    device_type: &str,
    action: &str,
    params: serde_json::Value,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let cmd = DeviceCommand {
        command_id: format!("move-{}", action),
        device_id: "test".into(),
        action: action.into(),
        params,
    };

    match adapter.encode_command(device_type, &cmd) {
        Ok(bytes) => {
            println!("  发送 {}: {:02X?}", action, bytes);
            if let Err(e) = transport.send(&bytes).await {
                println!("  发送失败: {}", e);
                return None;
            }

            match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
                Ok(Some(msg)) => {
                    println!("  收到 {} 字节: {:02X?}", msg.data.len(), msg.data);
                    match adapter.decode_response(device_type, &msg.data) {
                        Some(props) => {
                            println!("  解码: {}", serde_json::to_string_pretty(&props).unwrap());
                            // Convert HashMap to serde_json::Map
                            let map: serde_json::Map<String, serde_json::Value> =
                                props.into_iter().collect();
                            Some(map)
                        }
                        None => {
                            println!("  解码失败");
                            None
                        }
                    }
                }
                Ok(None) => { println!("  通道关闭"); None }
                Err(_) => { println!("  超时 (3s)"); None }
            }
        }
        Err(e) => { println!("  编码失败: {}", e); None }
    }
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
