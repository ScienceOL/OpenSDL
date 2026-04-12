# OpenSDL Developer Guide

OpenSDL (Open Self-Drive Lab) is a mesh-based system for laboratory hardware control. A mother node (Rust) manages devices through pluggable transports (MQTT serial, direct serial, TCP), with an embedded MQTT broker, mDNS discovery, and SQLite event store.

For detailed architecture diagrams and data flow, see [`docs/architecture.md`](docs/architecture.md).

## Architecture

### Core Abstraction

```
ProtocolAdapter: set_position(12.5) → "/1A3000R\r\n"   (WHAT bytes mean)
                                            │
Transport:                                  ▼            (HOW bytes travel)
  MqttSerial   → MQTT → ESP32 → RS-485 → device
  DirectSerial → /dev/ttyUSB0 → device
  Tcp          → TCP socket → device
```

**Transport** handles byte delivery. **ProtocolAdapter** handles byte encoding/decoding. The engine connects them: looks up the device's transport, encodes the command via the adapter, sends via the transport, and decodes responses back.

### System Overview

```
Mother Node (RPi / PC)                    Child Node (ESP32, ~$5)
┌────────────────────────────┐           ┌──────────────────┐
│ OsdlEngine (Rust)          │   MQTT    │ Firmware (C++)    │
│  ├── Transport layer       │◄═════════►│ Serial ↔ MQTT    │
│  ├── ProtocolAdapter layer │           │ transparent bridge│
│  ├── MQTT Broker (embedded)│           └────────┬─────────┘
│  ├── mDNS Advertiser       │                    │ 485/232/USB
│  ├── Event Store (SQLite)  │                 Device
│  └── Registry (YAML)       │
└────────────────────────────┘
         │              │
    /dev/ttyUSB0    TCP socket
         │              │
    Direct USB      Network
     Device         Instrument
```

### Key Types

| Type | Purpose |
|------|---------|
| `Transport` trait | Send bytes to a device, report connection status |
| `ProtocolAdapter` trait | Encode commands → bytes, decode bytes → status |
| `Device` | Matched device with `transport_id`, `device_type`, `adapter`, actions |
| `OsdlEngine` | Main loop: MQTT events, transport RX, command routing |
| `EventStore` | Append-only SQLite log (events, commands, serial bytes) |
| `EmbeddedBroker` | rumqttd MQTT broker in a background thread |
| `MdnsAdvertiser` | Advertises `_osdl._tcp.local` for child auto-discovery |

### Data Flow

**Command**: `send_command(cmd)` → adapter encodes → transport sends → device executes

**Response**: device responds → transport receives → `handle_transport_rx()` → adapter decodes → `OsdlEvent::DeviceStatus` emitted

**Registration** (MQTT serial): child publishes register → engine creates `MqttSerialTransport` → matches hardware via adapters → creates `Device` → `OsdlEvent::DeviceOnline`

### Child Node (ESP32)

Minimal firmware (~220 lines C++):
- Boot → WiFi → mDNS discover mother → MQTT connect
- Register: `osdl/nodes/{node_id}/register { hardware_id, baud_rate }`
- Serial bridge: `osdl/serial/{node_id}/tx` ↔ UART ↔ `osdl/serial/{node_id}/rx`
- Heartbeat: `osdl/nodes/{node_id}/heartbeat`

### MQTT Topic Convention

```
osdl/nodes/{node_id}/register     # child → mother: hardware ID, baud rate
osdl/nodes/{node_id}/heartbeat    # child → mother: alive ping
osdl/serial/{node_id}/tx          # mother → child: bytes to write to UART
osdl/serial/{node_id}/rx          # child → mother: bytes read from UART
```

## Project Structure

```
crates/
├── osdl-core/src/
│   ├── engine.rs            # OsdlEngine — main loop, dispatching
│   ├── transport/           # Transport trait + MqttSerial, DirectSerial (stub), TCP (stub)
│   ├── adapter/             # ProtocolAdapter trait + unilabos + runze codec
│   ├── broker.rs            # Embedded MQTT broker (rumqttd)
│   ├── mdns.rs              # mDNS service discovery
│   ├── store.rs             # SQLite event store
│   ├── protocol.rs          # Device, Node, Command, Status types
│   ├── event.rs             # OsdlEvent enum
│   └── config.rs            # OsdlConfig
├── osdl-core/tests/
│   ├── e2e_mqtt.rs          # 6 e2e tests (broker + engine + simulated ESP32)
│   └── integration.rs       # 6 integration tests (adapters, store, engine)
├── osdl-cli/src/main.rs     # Standalone binary
registry/unilabos/           # Device YAML schemas
firmware/esp32/              # Child node firmware (PlatformIO, ESP32-S3)
```

## Code Style

- Rust 2021 edition, async by default (tokio runtime)
- Use `list[T]` / `str | None` style annotations in Python (if applicable)
- `thiserror` for error types, `serde` + `serde_json` for serialization
- Minimal dependencies — keep the crate lightweight and embeddable
- Tests: `TestHarness` + `ChildNode` helpers in `e2e_mqtt.rs` for shared setup

## Build & Run

```bash
cargo build              # Build all crates
cargo run --bin osdl     # Run mother node
cargo test               # Run all 24 tests
```

## Integration with Xyzen

```
Xyzen Cloud → WebSocket → Runner → OsdlEngine → Transport → Device
```

- `osdl-core` as optional crate dependency in `xyzen-runner` (`feature = "osdl"`)
- New Runner message types: `osdl_list_devices`, `osdl_send_command`, etc.
- OsdlEvent forwarded to cloud via existing WebSocket (same pattern as PTY events)
