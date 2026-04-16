# OpenSDL: 从零到一完整搭建指南

本文档以一个具体场景为例 —— **用 OpenSDL 控制一台 Runze SY03B 注射泵** —— 详细介绍从购买硬件到跑通全链路的每个步骤。

## 目录

1. [系统全景](#1-系统全景)
2. [硬件采购清单](#2-硬件采购清单)
3. [硬件接线](#3-硬件接线)
4. [软件环境准备](#4-软件环境准备)
5. [烧录 ESP32 固件](#5-烧录-esp32-固件)
6. [启动母机 (Mother Node)](#6-启动母机-mother-node)
7. [验证全链路](#7-验证全链路)
8. [常见问题](#8-常见问题)
9. [下一步](#9-下一步)

---

## 1. 系统全景

```
┌─────────────────────────────────────────────────────────┐
│                  你的电脑 / 树莓派 (母机)                    │
│                                                          │
│   ┌────────────────────────────────────────────────┐    │
│   │              OsdlEngine (Rust)                   │    │
│   │                                                  │    │
│   │  MQTT Broker ← 嵌入式，无需额外安装                │    │
│   │  mDNS 广播   ← 子机自动发现母机 IP                 │    │
│   │  SQLite 日志  ← 所有命令/响应可回溯                 │    │
│   │  Runze 协议编解码 ← 母机负责所有驱动逻辑            │    │
│   └──────────────────────┬───────────────────────────┘    │
│                          │ MQTT (端口 1883)                │
└──────────────────────────┼────────────────────────────────┘
                           │
                      WiFi 局域网
                           │
                  ┌────────▼────────┐
                  │   ESP32-S3 子机   │
                  │                  │
                  │  WiFi + MQTT     │
                  │  Serial ↔ MQTT   │
                  │  透明桥接         │
                  └────────┬─────────┘
                           │
                    RS-485 总线 (A/B 两线)
                           │
                  ┌────────▼─────────┐
                  │  Runze SY03B     │
                  │  注射泵           │
                  │  6/8 端口阀       │
                  │  25mL 注射器      │
                  └──────────────────┘
```

**核心思想**：ESP32 不懂任何设备协议，它只是一根 "WiFi 网线"，把串口字节透明转发给母机。母机上的 Rust 引擎负责所有编解码和设备管理。

---

## 2. 硬件采购清单

### 必须购买

| # | 物品 | 型号推荐 | 参考价格 | 购买渠道 | 备注 |
|---|------|---------|---------|---------|------|
| 1 | **ESP32-S3 开发板** | ESP32-S3-DevKitC-1 (N8R2 或 N16R8) | ¥25-35 | 淘宝 / 立创商城 | 带 USB-C，自带天线 |
| 2 | **RS-485 收发模块** | MAX485 / SP3485 模块 (带排针) | ¥3-5 | 淘宝 | 3.3V 供电，带 DE/RE 引脚 |
| 3 | **杜邦线** | 母对母 + 母对公 各 10 根 | ¥5 | 淘宝 | 用于 ESP32 ↔ RS-485 模块连接 |
| 4 | **USB-C 数据线** | 任意 USB-C 线 | ¥5 | - | 用于给 ESP32 供电 + 烧录固件 |

**子机硬件总成本：约 ¥40-50**

### 设备 (实验室已有 / 按需购买)

| # | 物品 | 型号 | 参考价格 | 备注 |
|---|------|------|---------|------|
| 5 | **注射泵** | Runze SY03B-T06 (6 端口) 或 SY03B-T08 (8 端口) | ¥3,000-5,000 | RS-485 接口，9600 波特率 |
| 6 | **RS-485 线缆** | 双绞线 (A/B 两芯) | ¥5/米 | 通常泵自带线缆 |

### 母机 (你已有的电脑)

| # | 物品 | 要求 | 备注 |
|---|------|------|------|
| 7 | **电脑 / 树莓派** | 任何能跑 Rust 的系统 (macOS / Linux / Windows) | 需要连同一个 WiFi 网络 |
| 8 | **WiFi 路由器** | 任意 2.4GHz WiFi | ESP32 只支持 2.4GHz |

### 可选但推荐

| # | 物品 | 型号推荐 | 参考价格 | 用途 |
|---|------|---------|---------|------|
| 9 | USB 5V 电源适配器 | 任意 5V/1A USB 适配器 | ¥10 | ESP32 独立供电 (不接电脑时) |
| 10 | 面包板 | 830 孔 | ¥8 | 方便接线调试 |
| 11 | USB-RS485 转换器 | CH340/FT232 USB-485 模块 | ¥15 | 直接从电脑调试泵 (跳过 ESP32) |

---

## 3. 硬件接线

### 3.1 ESP32-S3 ↔ RS-485 模块

```
ESP32-S3 DevKitC-1                    MAX485/SP3485 模块
┌──────────────────┐                 ┌─────────────────┐
│                  │                 │                 │
│           GPIO17 ├─────────────────┤ DI (数据输入)    │
│           GPIO18 ├─────────────────┤ RO (数据输出)    │
│           GPIO16 ├─────────────────┤ DE + RE (方向)   │
│                  │                 │                 │
│              3V3 ├─────────────────┤ VCC             │
│              GND ├─────────────────┤ GND             │
│                  │                 │                 │
└──────────────────┘                 │     A ──────┐   │
       │                             │     B ──────┤   │
    USB-C                            └─────────────┘   │
    (供电+烧录)                              │    │     │
                                             │    │
                                         ┌───┘    └───┐
                                         │            │
                                    RS-485 A      RS-485 B
                                    (接泵的 A+)   (接泵的 B-)
```

**接线表：**

| ESP32 引脚 | RS-485 模块 | 说明 |
|-----------|------------|------|
| GPIO 17 | DI | ESP32 发送 → 设备 |
| GPIO 18 | RO | 设备响应 → ESP32 |
| GPIO 16 | DE + RE (短接) | 发送/接收方向控制 |
| 3V3 | VCC | 供电 |
| GND | GND | 接地 |

### 3.2 RS-485 模块 ↔ 注射泵

```
RS-485 模块                     Runze SY03B 注射泵
┌──────────┐                   ┌──────────────────┐
│    A (D+) ├───── 双绞线 ──────┤ RS-485 A (+)     │
│    B (D-) ├───── 双绞线 ──────┤ RS-485 B (-)     │
│    GND    ├─── (可选接地) ─────┤ GND              │
└──────────┘                   └──────────────────┘
```

> **注意**：RS-485 是差分信号，只需 A 和 B 两根线。距离短 (< 5m) 时不需要终端电阻。注射泵的 RS-485 接线端子通常在设备背面，参考泵的说明书确认 A/B 标注。

### 3.3 完整物理连接

```
┌──────┐   USB-C   ┌──────────┐  杜邦线  ┌──────────┐  双绞线  ┌──────────┐
│ 电脑  ├──────────┤ ESP32-S3 ├─────────┤ MAX485   ├────────┤ SY03B    │
│(母机) │  烧录+供电 │          │  5 根    │ 模块     │  2 根   │ 注射泵   │
└──────┘           └──────────┘          └──────────┘         └──────────┘
   │
   └── 连接同一个 WiFi 路由器 (2.4GHz)
```

---

## 4. 软件环境准备

### 4.1 母机侧 (Rust)

```bash
# 安装 Rust (如果没有)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 克隆项目
git clone https://github.com/ScienceOL/OpenSDL.git
cd OpenSDL

# 编译
cargo build --release

# 验证 (跑测试)
cargo test
# 应该看到: 24 passed; 0 failed
```

### 4.2 ESP32 侧 (PlatformIO)

```bash
# 方法 1: VS Code 插件 (推荐)
# 安装 VS Code → 搜索安装 "PlatformIO IDE" 插件 → 重启

# 方法 2: 命令行
pip install platformio
# 或
brew install platformio  # macOS
```

---

## 5. 烧录 ESP32 固件

### 5.1 创建配置文件

```bash
cd firmware/esp32

# 复制配置模板
cp src/config.example.h src/config.h
```

编辑 `src/config.h`，填入你的实际配置：

```c
// ── 你需要修改的配置 ──

// WiFi — 必须是 2.4GHz 网络 (ESP32 不支持 5GHz)
#define WIFI_SSID     "你的WiFi名称"
#define WIFI_PASSWORD "你的WiFi密码"

// MQTT — 留空使用 mDNS 自动发现 (推荐)
#define MQTT_HOST     ""

// 节点 ID — 每个 ESP32 必须唯一
#define NODE_ID       "pump-01"

// 硬件 ID — 必须与 registry YAML 中的 device_type 完全匹配
#define HARDWARE_ID   "syringe_pump_with_valve.runze.SY03B-T06"

// 波特率 — Runze 泵默认 9600
#define DEVICE_BAUD   9600

// 引脚 — 如果你接线和上面一致，不需要改
#define UART_TX_PIN   17
#define UART_RX_PIN   18
#define RS485_DE_PIN  16
```

### 5.2 编译并烧录

```bash
# 用 USB-C 线连接 ESP32 到电脑

# 编译 + 烧录
pio run -t upload

# 打开串口监视器查看输出
pio device monitor
```

你应该看到类似输出：

```
[OSDL] Child node starting...
[OSDL] Node: pump-01  Hardware: syringe_pump_with_valve.runze.SY03B-T06
[WiFi] Connecting to YourWiFi......
[WiFi] Connected: 192.168.1.105
[mDNS] Searching for _osdl._tcp.local ...
```

> ESP32 会一直尝试发现母机。现在还没启动母机，所以它会不断重试 —— 这是正常的。

### 5.3 烧录后断开电脑

烧录完成后，ESP32 可以：
- 继续插在电脑 USB 上 (通过 USB 供电)
- 或接一个 5V USB 电源适配器独立供电

---

## 6. 启动母机 (Mother Node)

### 6.1 启动 OSDL 引擎

```bash
cd OpenSDL

# 启动引擎 (带日志输出)
RUST_LOG=info cargo run --release --bin osdl
```

启动后你会看到：

```
[INFO] Embedded MQTT broker started on port 1883
[INFO] mDNS: advertising _osdl._tcp.local on port 1883
[INFO] Starting OpenSDL...
[INFO] MQTT connected to localhost:1883
```

### 6.2 ESP32 自动连接

几秒后，ESP32 串口监视器会显示：

```
[mDNS] Found mother node: 192.168.1.100:1883 (osdl-mother.local)
[MQTT] Connecting to 192.168.1.100:1883 as pump-01...
[MQTT] Connected
[MQTT] Subscribed: osdl/serial/pump-01/tx
[MQTT] Registered: {"hardware_id":"syringe_pump_with_valve.runze.SY03B-T06","baud_rate":9600}
```

母机侧会显示：

```
[INFO] Node registered: pump-01 (hardware: syringe_pump_with_valve.runze.SY03B-T06, baud: 9600)
[INFO] Event: DeviceOnline(Device { id: "pump-01:syringe_pump_with_valve.runze.SY03B-T06", ... })
```

**至此，ESP32 子机 → 母机的通信链路已建立。**

---

## 7. 验证全链路

### 7.1 确认设备发现

母机日志中应该出现 `DeviceOnline` 事件，说明：

```
✓ ESP32 已连接 WiFi
✓ mDNS 发现了母机
✓ MQTT 连接成功
✓ 硬件 ID 匹配到了 Runze 驱动
✓ 设备已注册，可以接收命令
```

### 7.2 发送命令测试

目前可以通过 MQTT 客户端工具手动发送命令来验证全链路：

```bash
# 安装 mosquitto 客户端 (如果没有)
# macOS:
brew install mosquitto
# Ubuntu:
# sudo apt install mosquitto-clients

# 订阅所有 OSDL 主题 (在一个终端窗口)
mosquitto_sub -h localhost -t "osdl/#" -v

# 在另一个终端，发送 "初始化" 命令的原始字节
# Runze 初始化命令: /1ZR\r\n
mosquitto_pub -h localhost -t "osdl/serial/pump-01/tx" -m "/1ZR
"
```

如果接线正确且泵已上电，你应该看到：

1. **ESP32 串口监视器**：`[TX] 6 bytes → UART`
2. **泵物理动作**：阀门转动 + 柱塞归位
3. **ESP32 串口监视器**：`[RX] 4 bytes → MQTT`
4. **mosquitto_sub 输出**：`osdl/serial/pump-01/rx` 收到响应字节
5. **母机日志**：`DeviceStatus { status: "Idle", ... }` 事件

### 7.3 全链路数据流图

```
你发送 "初始化" 命令:

  mosquitto_pub                    OSDL Engine                ESP32
  (或你的应用)                     (母机 Rust)                (子机)
       │                              │                        │
       │  MQTT: osdl/serial/          │                        │
       │  pump-01/tx                  │                        │
       │  payload: "/1ZR\r\n"         │                        │
       ├─────────────────────────────►│                        │
       │                              │  MQTT: osdl/serial/    │
       │                              │  pump-01/tx            │
       │                              ├───────────────────────►│
       │                              │                        │ UART TX
       │                              │                        ├──────►泵
       │                              │                        │
       │                              │                        │ UART RX
       │                              │                        │◄──────泵
       │                              │  MQTT: osdl/serial/    │
       │                              │  pump-01/rx            │
       │                              │◄───────────────────────┤
       │                              │                        │
       │                              │ Runze 解码:            │
       │                              │ "`3000\n" → Idle,      │
       │                              │   position: 12.5 mL    │
       │                              │                        │
       │  Event: DeviceStatus         │                        │
       │◄─────────────────────────────┤                        │
```

---

## 8. 常见问题

### ESP32 连不上 WiFi

- 确认 WiFi 是 **2.4GHz** (ESP32 不支持 5GHz)
- 检查 SSID 和密码是否正确 (区分大小写)
- 确认路由器没有开启 MAC 地址过滤

### mDNS 发现不到母机

- 确认母机和 ESP32 在**同一个局域网** (同一个路由器)
- 某些路由器会隔离 WiFi 客户端 (AP Isolation)，需要关闭
- 可以临时在 `config.h` 中设置 `MQTT_HOST` 为母机的固定 IP

### MQTT 连接失败

- 确认母机的 1883 端口没有被防火墙拦截
- macOS: 系统设置 → 网络 → 防火墙 → 允许传入连接
- Linux: `sudo ufw allow 1883`

### 泵没有反应

- 检查 RS-485 A/B 线是否接对 (交换试试)
- 确认泵已上电
- 确认波特率一致 (默认 9600)
- 用 USB-RS485 转换器直接从电脑发命令测试泵本身

### ESP32 串口看不到任何输出

- 确认 USB-C 线是**数据线** (不是纯充电线)
- 串口监视器波特率设为 **115200**
- ESP32-S3 需要 `ARDUINO_USB_CDC_ON_BOOT=1` (已在 platformio.ini 中配置)

---

## 9. 下一步

### 接入更多设备

每增加一台设备，只需要：
1. 再买一个 ESP32 + RS-485 模块 (~¥40)
2. 修改 `config.h` 中的 `NODE_ID` 和 `HARDWARE_ID`
3. 烧录固件，上电

母机会自动发现并注册新设备。

### 支持的设备类型

| 设备 | hardware_id | 状态 |
|------|------------|------|
| Runze SY03B-T06 注射泵 (6 端口) | `syringe_pump_with_valve.runze.SY03B-T06` | ✅ 已支持 |
| Runze SY03B-T08 注射泵 (8 端口) | `syringe_pump_with_valve.runze.SY03B-T08` | ✅ 已支持 |
| 大龙加热磁力搅拌器 | `heater_stirrer_dalong` | 📋 已注册，编解码待实现 |

### 添加新设备驱动

1. 在 `registry/unilabos/` 下创建 YAML 描述文件
2. 在 `adapter/` 下实现编解码逻辑 (参考 `runze.rs`)
3. 在 `adapter/unilabos.rs` 中注册路由
4. 写测试 → 跑通 → 提交

### 接入 Xyzen Desktop

```
Xyzen Desktop (Tauri) → Runner → OsdlEngine → ESP32 → 设备
```

OpenSDL 引擎会嵌入到 Xyzen Desktop 应用中，通过 Runner 暴露给前端 UI 和云端 Agent，实现 AI 直接控制实验室硬件。

---

## 技术栈总览

| 层 | 技术 | 说明 |
|---|------|------|
| **母机引擎** | Rust + Tokio | 异步运行时，主循环 |
| **MQTT Broker** | rumqttd (嵌入式) | 无需单独部署 |
| **事件存储** | SQLite (WAL) | 追加写，可回溯 |
| **服务发现** | mDNS (mdns-sd) | 子机自动发现母机 |
| **子机固件** | C++ / Arduino / PlatformIO | ESP32-S3 固件 |
| **通信协议** | MQTT v4 (QoS 1) | 可靠传输 |
| **物理层** | RS-485 (半双工) | 差分信号，抗干扰 |
| **设备注册** | YAML | 声明式设备描述 |
