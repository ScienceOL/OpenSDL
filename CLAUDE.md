# OpenSDL Developer Guide

OpenSDL (Open Self-Drive Lab) is a mesh-based system for laboratory hardware control. A mother node (Rust) manages low-cost child nodes (ESP32 serial bridges), communicating via MQTT.

For detailed architecture diagrams and data flow, see [`docs/architecture.md`](docs/architecture.md).

## Architecture

### System Overview

```
Mother Node (RPi / PC)                    Child Node (ESP32, ~$5)
┌────────────────────────────┐           ┌──────────────────┐
│ OsdlEngine (Rust)          │   MQTT    │ Firmware (C/Rust) │
│  ├── ProtocolAdapter layer │◄═════════►│ Serial ↔ MQTT    │
│  ├── Driver Manager        │           │ transparent bridge│
│  ├── MQTT Broker (embedded)│           └────────┬─────────┘
│  └── Registry (YAML+code)  │                    │ 485/232/USB
└────────────────────────────┘                 Device
```

**Child nodes are dumb serial bridges.** All intelligence (drivers, protocol parsing, device management) lives on the mother.

### Dual Driver Model

Two ways to drive a device, both producing serial bytes that get sent over MQTT:

**Path A — Rust native driver (preferred for new devices):**
```rust
// Directly generates serial bytes
fn set_temperature(&self, temp: f64) -> Vec<u8> {
    build_modbus_frame(0x01, 0x06, 0x000B, (temp * 10.0) as u16)
}
// → MQTT publish to osdl/serial/{node_id}/tx
```

**Path B — Python compatibility layer (for existing UniLabOS drivers):**
```python
# Existing driver runs unmodified on mother, with injected MqttSerial
heater = HeaterStirrer_DaLong.__new__(HeaterStirrer_DaLong)
heater.serial = MqttSerial("heater-01", mqtt_client)
heater.set_temperature(80)
# MqttSerial.write() → MQTT publish to osdl/serial/{node_id}/tx
```

`MqttSerial` is a drop-in replacement for `serial.Serial` that routes bytes over MQTT to the child node. Existing Python drivers need zero code changes.

### ProtocolAdapter

A `ProtocolAdapter` abstracts a **device driver ecosystem's standard**, not individual devices:

- **UniLabOS adapter**: parses UniLabOS YAML registry → understands device capabilities, action schemas, status types. Knows how to load UniLabOS Python drivers and inject MqttSerial.
- **Future adapters** (SiLA, vendor-specific): would parse their respective formats.

The adapter is responsible for:
1. Parsing device description files (YAML/XML) from `registry/`
2. Instantiating the correct driver for a given hardware ID
3. Translating between OpenSDL's unified model and the ecosystem's conventions

### Child Node (ESP32)

Minimal firmware (~hundreds of lines):
- Boot → WiFi connect → MQTT connect
- Publish registration: `osdl/nodes/{node_id}/register { hardware_id, baud_rate }`
- Subscribe `osdl/serial/{node_id}/tx` → write bytes to UART
- UART receive → publish `osdl/serial/{node_id}/rx`

That's it. No device knowledge, no protocol parsing.

Hardware: ESP32-S3 ($3) + RS-485 transceiver ($1) + PCB. Can be built as a small dongle.

### MQTT Topic Convention

```
# Node management
osdl/nodes/{node_id}/register              # child → mother: hardware ID, baud rate
osdl/nodes/{node_id}/heartbeat             # child → mother: alive ping

# Serial byte tunneling
osdl/serial/{node_id}/tx                   # mother → child: bytes to write to UART
osdl/serial/{node_id}/rx                   # child → mother: bytes read from UART

# Device-level (after mother parses serial responses via driver)
osdl/devices/{device_id}/status            # mother publishes parsed device status
osdl/devices/{device_id}/online            # retained + LWT
```

### Integration with Xyzen

```
Xyzen Cloud → WebSocket → Runner → OsdlEngine → MQTT → ESP32 → Serial → Device
```

- `osdl-core` as optional crate dependency in `xyzen-runner` (`feature = "osdl"`)
- New Runner message types: `osdl_list_devices`, `osdl_send_command`, etc.
- OsdlEvent forwarded to cloud via existing WebSocket (same pattern as PTY events)
- Desktop Tauri app also gets direct access for local device UI

## Project Structure

```
crates/
├── osdl-core/                   # Core library
│   └── src/
│       ├── lib.rs               # Public API exports
│       ├── engine.rs            # OsdlEngine — main loop, MQTT, dispatching
│       ├── config.rs            # OsdlConfig
│       ├── protocol.rs          # Unified device model
│       ├── mqtt.rs              # MQTT client wrapper
│       ├── event.rs             # OsdlEvent enum
│       ├── driver/              # Driver manager + MqttSerial
│       │   ├── mod.rs
│       │   ├── mqtt_serial.rs   # MqttSerial (serial.Serial replacement)
│       │   └── registry.rs      # Load drivers by hardware ID
│       └── adapter/
│           ├── mod.rs           # ProtocolAdapter trait
│           └── unilabos.rs      # UniLabOS ecosystem adapter
└── osdl-cli/                    # Standalone binary (mother node)
    └── src/
        └── main.rs
registry/
└── unilabos/                    # YAML schemas + Python drivers
firmware/
└── esp32/                       # Child node firmware
```

## Code Style

- Rust 2021 edition
- Async by default (tokio runtime)
- `thiserror` for error types
- `serde` + `serde_json` for serialization
- Minimal dependencies — keep the crate lightweight and embeddable

## Build & Run

```bash
cargo build              # Build all crates
cargo run --bin osdl     # Run mother node
cargo test               # Run tests
```

## References

- [Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS) — First supported device driver ecosystem
- Industry parallels: Balena.io, AWS Greengrass, EdgeX Foundry
