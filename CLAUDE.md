# OpenSDL Developer Guide

OpenSDL (Open Self-Drive Lab) is a Rust-based protocol adapter that bridges laboratory hardware platforms to applications via MQTT. It is stateless, embeddable, and platform-agnostic.

## Architecture

### Core Abstraction

OpenSDL does NOT drive hardware directly. It adapts **platform-level protocols** — each lab platform (UniLabOS, SiLA, vendor systems) has its own device discovery, status reporting, and command format. OpenSDL normalizes these into a unified device model.

```
Application → OsdlEngine → PlatformAdapter → MQTT → Hardware Platform → Devices
```

### Key Components

| Component | Purpose |
|-----------|---------|
| `OsdlEngine` | Main entry point. Manages MQTT connection, routes messages to adapters, emits events. |
| `PlatformAdapter` trait | One implementation per hardware platform. Handles protocol translation. |
| `protocol` | Unified data model: `Device`, `DeviceStatus`, `DeviceCommand`, `CommandResult`. |
| `mqtt` | MQTT client wrapper (rumqttc). |
| `event` | `OsdlEvent` enum — device online/offline, status updates, command feedback. |

### Integration Pattern

OpenSDL is designed to be embedded in a host application as a Rust crate dependency. The host:
1. Creates an `OsdlEngine` with config
2. Spawns `engine.run()` in a tokio task
3. Takes the event receiver (`engine.take_event_rx()`) to forward events
4. Calls `engine.list_devices()`, `engine.send_command()` etc. for request-response operations

This mirrors the PtyManager pattern — an async engine with an unbounded event channel for push, plus direct methods for pull.

### MQTT Topic Convention

```
{platform}/{gateway_id}/devices                  # retained, device list
{platform}/{gateway_id}/{device_id}/status       # QoS1, status reports
{platform}/{gateway_id}/{device_id}/online       # retained + LWT
{platform}/{gateway_id}/{device_id}/command      # QoS1, command dispatch
{platform}/{gateway_id}/{device_id}/command/ack  # QoS1, command result
```

## Project Structure

```
crates/
├── osdl-core/                   # Core library
│   └── src/
│       ├── lib.rs               # Public API exports
│       ├── engine.rs            # OsdlEngine — MQTT loop + adapter dispatch
│       ├── config.rs            # OsdlConfig (TOML + env)
│       ├── protocol.rs          # Device, DeviceStatus, DeviceCommand, CommandResult
│       ├── mqtt.rs              # MQTT client wrapper
│       ├── event.rs             # OsdlEvent enum
│       └── adapter/
│           ├── mod.rs           # PlatformAdapter trait
│           └── unilabos.rs      # Uni-Lab-OS adapter
└── osdl-cli/                    # Standalone binary
    └── src/
        └── main.rs
```

## Code Style

- Rust 2021 edition
- Async by default (tokio runtime)
- `thiserror` for error types
- `serde` + `serde_json` for all data structures
- Minimal dependencies — keep the crate lightweight and embeddable

## Build & Run

```bash
# Build all crates
cargo build

# Run CLI
cargo run --bin osdl

# Run tests
cargo test
```

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `rumqttc` | MQTT 5.0 async client |
| `tokio` | Async runtime |
| `serde` / `serde_json` | Serialization |
| `async-trait` | Async trait support |
| `thiserror` | Error handling |
