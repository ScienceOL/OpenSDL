//! 运动测试: Laiyu XYZ 移液工作站 — X 轴移动 10mm 再回位 (RS-485 串口)
//!
//! 用法:
//!   cargo run --example move_laiyu --features serial
//!   cargo run --example move_laiyu --features serial -- --port /dev/ttyUSB0
//!
//! 默认: /dev/ttyUSB0, 115200 baud
//! 测试内容:
//!   1. 使能 X 轴电机
//!   2. 读取当前位置
//!   3. 向正方向移动 10mm
//!   4. 等待 standby
//!   5. 移动回原位

use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::protocol::DeviceCommand;
use osdl_core::transport::direct_serial::DirectSerialTransport;
use osdl_core::transport::{Transport, TransportRx};
use std::time::Duration;
use tokio::sync::mpsc;

const BAUD: u32 = 115200;
/// X 轴: 16384 steps/rev, lead 80mm → 204.8 steps/mm
const X_STEPS_PER_MM: f64 = 204.8;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let port = arg_value(&args, "--port").unwrap_or_else(|| "/dev/ttyUSB0".into());
    let registry_path = arg_value(&args, "--registry");
    let distance_mm: f64 = arg_value(&args, "--distance")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0);

    println!("=== Laiyu XYZ 运动测试 ===");
    println!("串口: {} @ {} baud", port, BAUD);
    println!("X 轴移动距离: {} mm", distance_mm);
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

    // 打开串口
    let (rx_tx, mut rx_rx) = mpsc::unbounded_channel::<TransportRx>();
    let transport = DirectSerialTransport::new(port.clone(), BAUD, rx_tx);
    transport.start().await.expect("无法打开串口");
    println!("串口已打开\n");

    let device = "stepper_motor.laiyu_xyz.X";

    // 步骤 1: 使能 X 轴
    println!("--- 步骤 1: 使能 X 轴 ---");
    send_and_decode(&adapter, &transport, &mut rx_rx, device, "enable", serde_json::json!({"enable": true})).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 步骤 2: 读取当前位置
    println!("--- 步骤 2: 读取当前位置 ---");
    let initial = send_and_decode(&adapter, &transport, &mut rx_rx, device, "get_status", serde_json::json!({})).await;
    let initial_steps = initial
        .as_ref()
        .and_then(|p| p.get("position_steps"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    let initial_mm = initial
        .as_ref()
        .and_then(|p| p.get("position_mm"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    println!("  当前: {} steps = {:.2} mm", initial_steps, initial_mm);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 步骤 3: 移动 +distance_mm
    let target_steps = initial_steps + (distance_mm * X_STEPS_PER_MM) as i32;
    println!("--- 步骤 3: 移动到 {} steps ({:.2} mm → {:.2} mm) ---", target_steps, initial_mm, initial_mm + distance_mm);

    send_and_decode(
        &adapter, &transport, &mut rx_rx, device,
        "move_to_position",
        serde_json::json!({"position": target_steps, "speed": 2000, "acceleration": 500}),
    ).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 启动运动
    send_and_decode(&adapter, &transport, &mut rx_rx, device, "start_motion", serde_json::json!({})).await;

    // 步骤 4: 轮询等待 standby
    println!("--- 步骤 4: 等待运动完成 ---");
    wait_standby(&adapter, &transport, &mut rx_rx, device, 20).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 确认位置
    println!("--- 确认到达位置 ---");
    let mid = send_and_decode(&adapter, &transport, &mut rx_rx, device, "get_status", serde_json::json!({})).await;
    if let Some(ref p) = mid {
        println!("  当前: {} steps = {:.2} mm",
            p.get("position_steps").unwrap_or(&serde_json::json!("?")),
            p.get("position_mm").and_then(|v| v.as_f64()).unwrap_or(0.0));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 步骤 5: 回到原位
    println!("--- 步骤 5: 回到原位 ({} steps = {:.2} mm) ---", initial_steps, initial_mm);
    send_and_decode(
        &adapter, &transport, &mut rx_rx, device,
        "move_to_position",
        serde_json::json!({"position": initial_steps, "speed": 2000, "acceleration": 500}),
    ).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    send_and_decode(&adapter, &transport, &mut rx_rx, device, "start_motion", serde_json::json!({})).await;

    println!("--- 等待回位完成 ---");
    wait_standby(&adapter, &transport, &mut rx_rx, device, 20).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 最终位置
    println!("--- 最终位置 ---");
    send_and_decode(&adapter, &transport, &mut rx_rx, device, "get_status", serde_json::json!({})).await;

    transport.stop().await.ok();
    println!("\n=== 测试完成 ===");
}

/// 轮询 get_status, 等待 standby
async fn wait_standby(
    adapter: &UniLabOsAdapter,
    transport: &DirectSerialTransport,
    rx: &mut mpsc::UnboundedReceiver<TransportRx>,
    device_type: &str,
    max_retries: u32,
) {
    for i in 0..max_retries {
        tokio::time::sleep(Duration::from_millis(300)).await;

        let props = send_and_decode(adapter, transport, rx, device_type, "get_status", serde_json::json!({})).await;
        if let Some(ref p) = props {
            let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let mm = p.get("position_mm").and_then(|v| v.as_f64()).unwrap_or(0.0);
            print!("  轮询 [{}/{}]: {} @ {:.2}mm", i + 1, max_retries, status, mm);
            if status == "standby" {
                println!(" ✓");
                return;
            }
            println!();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("  ⚠ 超时未等到 standby");
}

async fn send_and_decode(
    adapter: &UniLabOsAdapter,
    transport: &DirectSerialTransport,
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

            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(msg)) => {
                    println!("  收到 {} 字节: {:02X?}", msg.data.len(), msg.data);
                    match adapter.decode_response(device_type, &msg.data) {
                        Some(props) => {
                            println!("  解码: {}", serde_json::to_string_pretty(&props).unwrap());
                            let map: serde_json::Map<String, serde_json::Value> =
                                props.into_iter().collect();
                            Some(map)
                        }
                        None => {
                            println!("  解码失败 (可能是其他设备的响应)");
                            None
                        }
                    }
                }
                Ok(None) => { println!("  通道关闭"); None }
                Err(_) => { println!("  超时 (2s)"); None }
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
