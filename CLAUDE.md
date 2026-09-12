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
Mother (RPi / PC)                          Node (ESP32, ~$5)
┌────────────────────────────┐            ┌──────────────────┐
│ OsdlEngine (Rust)          │   MQTT     │ Firmware (Rust)   │
│  ├── Transport layer       │◄══════════►│ Serial ↔ MQTT    │
│  ├── ProtocolAdapter layer │            │ transparent bridge│
│  ├── MQTT Broker (embedded)│            └────────┬─────────┘
│  ├── mDNS Advertiser       │  USB-CDC            │ 485/232/USB
│  ├── Event Store (SQLite)  │◄══════════►Dongle ──┴─ ESP-NOW ─┐
│  └── Registry (YAML)       │  (ESP32 USB)                    │
└────────────────────────────┘                                 ▼
         │              │                              Node ─ Device
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
| `MdnsAdvertiser` | Advertises `_osdl._tcp.local` for node auto-discovery |

### Data Flow

**Command**: `send_command(cmd)` → adapter encodes → transport sends → device executes

**Response**: device responds → transport receives → `handle_transport_rx()` → adapter decodes → `OsdlEvent::DeviceStatus` emitted

**Registration** (MQTT serial): node publishes register → engine creates `MqttSerialTransport` → matches hardware via adapters → creates `Device` → `OsdlEvent::DeviceOnline`

### Terminology

- **Dongle** — the ESP32 board plugged into the **host** (Mac/PC) via USB-CDC.
  Owns the host-side serial port and bridges Mac ↔ ESP-NOW broadcast.
- **Node** — an ESP32 board plugged into the **lab device** (RS-485 / serial).
  One per physical bus or device. Filters ESP-NOW frames by its own MAC and
  bridges payloads to/from the device.

### Node firmware (ESP32)

Minimal firmware (~220 lines C++):
- Boot → WiFi → mDNS discover mother → MQTT connect
- Register: `osdl/nodes/{node_id}/register { hardware_id, baud_rate }`
- Serial bridge: `osdl/serial/{node_id}/tx` ↔ UART ↔ `osdl/serial/{node_id}/rx`
- Heartbeat: `osdl/nodes/{node_id}/heartbeat`

### MQTT Topic Convention

```
osdl/nodes/{node_id}/register     # node → mother: hardware ID, baud rate
osdl/nodes/{node_id}/heartbeat    # node → mother: alive ping
osdl/serial/{node_id}/tx          # mother → node: bytes to write to UART
osdl/serial/{node_id}/rx          # node → mother: bytes read from UART
```

## Project Structure

```
crates/
├── osdl-core/src/              # Engine + library core
│   ├── engine.rs               # OsdlEngine — main loop, dispatching
│   ├── orchestrator.rs         # Higher-level device orchestration
│   ├── transport/              # Transport trait + impls
│   │   ├── mqtt_serial.rs        # → MQTT → ESP32 → RS-485 → device
│   │   ├── espnow_dongle.rs      # ESP-NOW dongle bridge
│   │   ├── direct_serial.rs      # USB/RS-232/RS-485 on the mother node (real)
│   │   ├── tcp.rs                # TCP socket (real)
│   │   └── onvif.rs              # HTTP/SOAP control plane for IP cameras
│   ├── adapter/                # ProtocolAdapter trait
│   │   ├── unilabos.rs           # UniLabOs adapter
│   │   └── onvif.rs              # ONVIF camera adapter
│   ├── driver/                 # Driver trait + per-device codecs
│   │   ├── builtins/             # emm, laiyu_xyz, runze, sopa, xkc
│   │   └── registry.rs           # DriverRegistry (YAML factory)
│   ├── media/                  # Media plane: mediamtx (SRS WebRTC), onvif_camera
│   ├── broker.rs               # Embedded MQTT broker (rumqttd)
│   ├── mqtt.rs / mdns.rs       # MQTT + mDNS service discovery
│   ├── store.rs                # SQLite event store
│   ├── protocol.rs             # Device, Node, Command, Status types
│   ├── event.rs                # OsdlEvent enum
│   └── config.rs               # OsdlConfig
├── osdl-core/tests/
│   ├── e2e_mqtt.rs             # e2e tests (broker + engine + simulated ESP32)
│   └── integration.rs          # integration tests (adapters, store, engine)
├── lab-cli/src/                # `lab` binary — subcommands: serve, status, device, send, events, stop
├── osdl-server/                # gRPC server (tonic) — what `lab serve` runs (TCP + UDS)
├── osdl-proto/                 # tonic-generated gRPC protobuf crate (consumed by the runner)
├── osdl-firmware-protocol/     # Shared types between core and firmware
registry/unilabos/              # Device YAML schemas
firmware/
├── esp32/                      # Rust firmware leaf crate (xtensa-esp32-espidf)
├── esp32s3/                    # Rust firmware leaf crate (xtensa-esp32s3-espidf)
└── esp32-cpp/                  # Legacy C++ PlatformIO stub
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
cargo run --bin lab serve    # Boot engine + gRPC server (TCP + UDS)
cargo test               # Run all tests (e2e + integration)
```

## Integration with SciLaxy

```
SciLaxy Cloud ←WebSocket→ Runner (gRPC client) ←gRPC→ `lab serve` (OsdlEngine) → Transport → Device
```

The runner is a **pure gRPC client** of OpenSDL, not an embedded crate:
it depends on `osdl-proto` (the generated tonic crate) behind the
`feature = "osdl"`. The engine + gRPC server live in a separate `lab serve`
process — spawned and supervised by the desktop host (see
`desktop/electron`'s `lab_server.ts`) or run independently on a lab Pi.
The runner connects to whatever endpoint the user configured.

- Runner message types (desktop ↔ cloud WebSocket, same channel as PTY events): `osdl_list_devices`, `osdl_send_command`, `osdl_device_online`, `osdl_device_offline`, `osdl_media_source_online`, …
- OsdlEvent is forwarded to the cloud over the runner's existing WebSocket (same pattern as PTY events).
