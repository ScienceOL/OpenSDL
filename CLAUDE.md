# OpenSDL Developer Guide

OpenSDL (Open Self-Drive Lab) is a mesh-based system for laboratory hardware control. It consists of a mother node (Rust) and child nodes (Linux SBCs running Docker), communicating via MQTT.

For detailed architecture diagrams and data flow, see [`docs/architecture.md`](docs/architecture.md).

## Architecture

### System Overview

OpenSDL is a mother-child mesh network. The mother node orchestrates; child nodes execute device drivers.

```
┌─────────────────────── Mother Node ───────────────────────┐
│                                                            │
│  OsdlEngine (Rust)                                         │
│    ├── ProtocolAdapter layer (device standard abstraction) │
│    ├── Node Manager (child lifecycle, driver provisioning) │
│    ├── MQTT Broker (embedded, LAN)                         │
│    └── Registry (YAML schemas + driver code)               │
│                                                            │
└──────────────────────────┬─────────────────────────────────┘
                           │ MQTT (WiFi / LAN)
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼─────┐ ┌───▼───┐ ┌──────▼────┐
        │ Child Node│ │ Child │ │ Child Node│
        │           │ │ Node  │ │           │
        │ Docker    │ │       │ │ Docker    │
        │ container │ │  ...  │ │ container │
        │ [driver]  │ │       │ │ [driver]  │
        └─────┬─────┘ └───────┘ └─────┬─────┘
              │ Serial (485/232/USB)   │
           Device                   Device
```

### Key Principle: Driver Runs on the Child

Device drivers (Python files from UniLabOS or other ecosystems) execute **on the child node** that is physically connected to the hardware. The mother never handles serial bytes or I/O — it only manages the mesh and translates between the application and MQTT.

This means:
- No virtual serial ports, no I/O interception
- Drivers use real `serial.Serial("/dev/ttyUSB0")` calls
- Each driver runs in a Docker container with `--device` serial mapping
- Child nodes are self-contained and can survive mother disconnection

### ProtocolAdapter

A `ProtocolAdapter` does NOT abstract individual devices. It abstracts a **device driver ecosystem's standard**:

- **UniLabOS adapter**: understands UniLabOS YAML registry format + its Python driver calling conventions + its MQTT topic structure
- **Future SiLA adapter**: would understand SiLA XML definitions + its gRPC conventions
- Each adapter knows how to: parse device descriptions, provision drivers to child nodes, and translate MQTT messages between the standard's format and OpenSDL's unified model

### Child Node Lifecycle

1. Child boots → connects to mother's MQTT broker → publishes hardware ID on `osdl/nodes/{node_id}/register`
2. Mother receives registration → looks up hardware ID in registry → determines which driver + config to deploy
3. Mother pushes driver image/files to child (Docker image pull or file transfer over MQTT)
4. Child starts Docker container: `docker run --device /dev/ttyUSB0 driver-{device_type}`
5. Container runs driver, subscribes/publishes on MQTT topics for status + commands
6. On subsequent boots, child uses cached driver — instant start without mother

### MQTT Topic Convention

```
# Node management
osdl/nodes/{node_id}/register              # child → mother: registration + hardware ID
osdl/nodes/{node_id}/provision             # mother → child: driver config + image
osdl/nodes/{node_id}/heartbeat             # child → mother: periodic health check

# Device communication (within a protocol standard)
osdl/{platform}/{node_id}/{device_id}/status       # child → mother: device status (QoS1, retained)
osdl/{platform}/{node_id}/{device_id}/command       # mother → child: device command (QoS1)
osdl/{platform}/{node_id}/{device_id}/command/ack   # child → mother: command result (QoS1)
osdl/{platform}/{node_id}/{device_id}/online        # child → mother: LWT (retained)
```

### Integration with Host Application

OpenSDL is designed to be embedded in a host application (e.g. Xyzen Desktop via Tauri) as a Rust crate:

1. Host creates `OsdlEngine` with config
2. Host spawns `engine.run()` in a tokio task
3. Host takes event receiver (`engine.take_event_rx()`) for async push events (device status, node online/offline)
4. Host calls `engine.list_devices()`, `engine.send_command()` etc. for request-response

When integrated with Xyzen:
```
Xyzen Cloud → WebSocket → Runner → OsdlEngine → MQTT → Child Nodes → Devices
```

## Project Structure

```
crates/
├── osdl-core/                   # Core library
│   └── src/
│       ├── lib.rs               # Public API exports
│       ├── engine.rs            # OsdlEngine — main loop, MQTT, dispatching
│       ├── config.rs            # OsdlConfig
│       ├── protocol.rs          # Unified device model (Device, DeviceStatus, DeviceCommand)
│       ├── mqtt.rs              # MQTT client wrapper
│       ├── event.rs             # OsdlEvent enum
│       └── adapter/
│           ├── mod.rs           # ProtocolAdapter trait
│           └── unilabos.rs      # UniLabOS ecosystem adapter
└── osdl-cli/                    # Standalone binary (mother node)
    └── src/
        └── main.rs
registry/
└── unilabos/                    # Device schemas (YAML) + drivers (Python)
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
cargo run --bin osdl     # Run mother node CLI
cargo test               # Run tests
```

## References

- [Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS) — First supported device driver ecosystem
- Industry parallels: Balena.io (fleet management), AWS Greengrass (edge modules), EdgeX Foundry (IoT edge platform)
