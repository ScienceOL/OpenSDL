# 固件与硬件接入状态（2026-06-02）

术语：**dongle** = 接 host (Mac/PC) 的 ESP32；**node** = 接实验设备 (RS-485 / serial) 的 ESP32。两者通过 ESP-NOW 广播通信。

## 三个硬件阶段

| 阶段 | Dongle | Node | 接入设备 | 状态 |
|---|---|---|---|---|
| 1 | ESP32-S3 + UART0（外置 USB-UART） | LilyGO T-Connect Pro（板载 TD501D485H 隔离 RS-485 + ST7796 LCD，UART1 GPIO17/18） | chinwe (Runze SY-03B 注射泵) | 历史，仍可用于 chinwe |
| 2 | 同上 | 标准 ESP32-D0WD-V3 + 外置 MAX485（UART2 GPIO16/17，DE/RE=GPIO22） | laiyu_xyz station（XYZ 轴 + sopa pipette） | 历史 |
| 3 | **ESP32-S3 + 屏幕**（原生 USB-Serial-JTAG，host 看到 `usbmodem*`） | **新 ESP32 量产板**（外置 CH340，host 看到 `usbserial-*`），暂时仍用初代 ESP32（待硬件方核实） | 量产用 | **当前** |

## 当前 Mac 上插的硬件（阶段 3）

| 端口 | 角色 | 识别特征 | espflash + esptool.py 双向核实 |
|------|------|---------|----|
| `/dev/cu.usbmodem11301` | **Dongle** | Espressif 原生 USB-Serial-JTAG，VID:PID `303a:1001`，SN `3C:0F:02:DE:7F:50` | ESP32-S3 (QFN56) rev v0.2，16MB flash，**8MB 嵌入 PSRAM**，MAC `3c:0f:02:de:7f:50` ✅ 符合阶段 3 描述 |
| `/dev/cu.usbserial-5B320494961` | **Node** | CH340，VID:PID `1a86:55d4`，SN `5B32049496` | ESP32-D0WDQ6-V3（**初代 ESP32**，LX6）rev v3.1，4MB flash，无 PSRAM，无原生 USB，MAC `a4:f0:0f:d8:55:5c` ❓ 与"接近 esp32-rs (S3)"描述不符 |

阶段 3 dongle 的关键变化：之前 dongle 是「ESP32 + 外置 USB-UART」，host 端口形如 `usbserial-*`；现在用 ESP32-S3 原生 USB-Serial-JTAG（同一颗 ESP32-S3 直接被 host 枚举为 CDC ACM），所以 dongle 端口形如 `usbmodem*`，可以靠端口名前缀区分 dongle/node。

## 仓库布局

```
crates/
├── osdl-core/                  host 端 engine / transport / adapter
└── osdl-firmware-protocol/     pure-Rust no_std，dongle/node/host 共用的
                                 ESP-NOW 帧 + REG 编解码（消重 + 单测覆盖）

firmware/
├── esp32-cpp/                  legacy C++ PlatformIO stub（已不在主路径）
├── esp32/                      Rust crate, target = xtensa-esp32-espidf
│   └── src/bin/
│       └── node.rs             ESP32 + 外置 MAX485 节点（laiyu_xyz）
└── esp32s3/                    Rust crate, target = xtensa-esp32s3-espidf
    └── src/bin/
        ├── dongle.rs           dongle 固件（host USB-Serial-JTAG ↔ ESP-NOW）
        ├── node-lcd.rs         LilyGO LCD node（chinwe），有 ST7796 屏渲染线程
        ├── uart_count.rs       UART1 TX 自检
        ├── espnow_mac.rs       早期 MAC 打印 helper
        ├── espnow_diag.rs      早期 ESP-NOW 接收诊断
        └── (main.rs → bin "legacy-mqtt")  pre-ESP-NOW MQTT 主固件，保留供参考
```

两个 leaf crate（`firmware/esp32` 和 `firmware/esp32s3`）都不是主 workspace 成员（embuild / esp-idf-sys 限制每 crate 一个 MCU target）。共享逻辑通过 `path` 依赖 `crates/osdl-firmware-protocol`，该 lib 是纯 Rust no_std，host 也能 link。

工具链：esp-idf-svc + xtensa；构建前 `source ~/export-esp.sh`，然后 `cd firmware/esp32` 或 `cd firmware/esp32s3` 再 `cargo build`。

## UART0 / USB-Serial-JTAG 线协议（host ↔ dongle）

- host → dongle：`TX <mac_hex12> <hex_bytes>\n`
- dongle → host：`I (...) dongle: RX <mac_hex12> <hex_bytes>` / `[tx->radio]` / `ER ...`

host 端 parser 只匹配 `RX ` 三个字符，对日志前缀（`I (...) dongle:` 等）prefix-agnostic。

## ESP-NOW 链路

- peer = `FF:FF:FF:FF:FF:FF` 广播，channel 1，无 peer 表
- node 按 frame 头 6 字节 dst_mac 自过滤
- host 端 `EspNowDongleClient` 维护 `MAC ↔ hardware_id` 表，每个 node 在 engine 中是独立的 `EspNowNodeTransport`，transport_id = `espnow:<MAC_HEX>`
- 同时挂 N 个 node 完全并行

## hardware_id 作用链路（当前实现）

完整链路（node 上电 → mother 暴露 Device）：

```
┌─ 固件 (节点)
│  const HARDWARE_ID: &str = "..."        ← 编译时写死，烧入 flash
│  reg_codec::build_with_hardware_id() 构造 "REG <HARDWARE_ID>" payload
│  espnow.send(BROADCAST, payload)
│
├─ 物理传输
│  ESP-NOW 广播 → dongle USB-Serial-JTAG → host
│
├─ Dongle client (crates/osdl-core/src/transport/espnow_dongle.rs)
│  调用 osdl_firmware_protocol::reg::parse → upsert MAC↔id 路由表
│  → 广播 RegEvent
│
└─ Engine (engine.rs handle_espnow_reg)
   1. 创建 EspNowNodeTransport(MAC) 加入 transports
   2. 查 config.buses.find(|b| b.match_hardware_id == hardware_id)
      ├─ 命中 → register_bus_devices() 展开成 N 个 Device（共享 transport，
      │           每个 Device 有独立 device_type/adapter/local_id）
      └─ 未命中 → fallback：在 adapter registry 里直接查 hardware_id，
                   匹配则注册 1 个 Device，否则 emit UnknownNode
```

**关键事实**：
- hardware_id **只在 mother 端做"路由 key"用**，匹配靠的是字符串相等。固件不解释它，dongle 不解释它，只有 engine 拿它去 YAML 里查表。
- 是否走 bus 路径 = "这个字符串是否出现在 `config.buses[].match_hardware_id`"。和字符串长什么样无关 — `bus.xxx` 前缀只是命名习惯，不是触发条件。
- **同一颗 node 固件配 chinwe 还是 laiyu，由两件事决定**：(a) 烧进固件的 `HARDWARE_ID` 字面值；(b) mother 启动时加载的 `--config` YAML 是否包含匹配该字面值的 `match_hardware_id`。两者要对得上才能正确暴露设备。
- 改设备归属 = 改固件 `const` 重烧。当前固件没有 NVS 持久化、没有运行时配置接口，HARDWARE_ID 是编译期常量。

## 待做：node 站点归属运行时化（方案 B 的形状）

当前痛点：每加一个站点需要改 const 重烧固件。已规划方向：**MAC 映射放 mother 端**。

- 固件 REG 改用 `osdl_firmware_protocol::reg::build_mac_only()`（payload 为 `b"REG"`，不带 hw_id）
- mother YAML 增加 `mac_assignments: { "AAFFEEDDCC11": "bus.laiyu_xyz.station1" }`
- engine 在 `handle_espnow_reg` 收到 `Reg::MacOnly` 时去 `mac_assignments` 查 hw_id，再走原 bus / 1:1 路径
- 旧 `Reg::WithHardwareId` 形式继续兼容（迁移期共存）

`crates/osdl-firmware-protocol` 已经把这两种 REG 都建模成 enum (`Reg::MacOnly` / `Reg::WithHardwareId`)，host 端 `parse_reg_payload` 暂时把 MacOnly 当作"无效"丢弃，等 engine 改造同步上线时切到查 `mac_assignments`。固件这边需要在切换那一刻改一行 `build_with_hardware_id(...)` → `build_mac_only()`。

## 已知 node MAC / hardware_id

| MAC | 阶段 | hardware_id（固件 `const`） | mother 端 YAML 路径 | 展开设备 |
|---|---|---|---|---|
| `30:ED:A0:B6:5B:38` | 1 | `syringe_pump_with_valve.runze.SY03B-T06` | bus（`chinwe-station.yaml`） | pump-1/2/3 + motor-4/5（5 个） |
| `F4:65:0B:47:B8:88` | 2 | `bus.laiyu_xyz.station1` | bus（`laiyu-xyz-station.yaml`） | axis-x/y/z + sopa pipette（4 个） |
| `a4:f0:0f:d8:55:5c` | 3 | （待烧录） | （laiyu 复用 `bus.laiyu_xyz.station1`，或重新分配） | 待定 |

两条历史 hw_id 都是 bus 路径 — 命名风格不一致只是历史遗留（chinwe 的 hardware_id 看起来像单设备型号，但 mother YAML 把它声明为 bus）。是不是 bus **完全由 mother 端 `config.buses` 决定**，不由固件字面值决定。

⚠️ chinwe 是生产中的真实设备 — 不要用 `/1ZR\r\n` 等会动机构的命令做链路验证，用 `scripts/probe_dongle.py` 发到 sink MAC 即可。

## 验证脚本

- `scripts/probe_dongle.py` — host → dongle → ESP-NOW 链路验证，发到 sink MAC，不触动任何 node 设备
- `scripts/probe_node_rs485.py` — 通过 node USB 透传发数据到 RS485（配合网页串口工具监听 RS485 总线，仅在烧 OSDL 固件之前的早期 bring-up 阶段使用）
- `scripts/send_to_node.py` — 通过 dongle 给指定 MAC 发命令
- `scripts/rs485_direct_probe.py` — RS485 总线直连验证
