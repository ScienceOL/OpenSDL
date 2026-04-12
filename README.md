# OpenSDL

**Open Self-Drive Lab** — A mesh-based system for laboratory hardware control with pluggable transports.

## What is OpenSDL?

OpenSDL connects laboratory hardware to your application through a unified control layer. It reuses existing device driver ecosystems (starting with [Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS)) without requiring their platform software to run.

```
              Your Application (Xyzen, LIMS, custom)
                        │
                        │  Rust crate / CLI
                        │
┌───────────────────────▼────────────────────────────────────┐
│                     Mother Node                             │
│                     (RPi / PC / Server)                     │
│                                                             │
│  ┌──────────┐  ┌────────────┐  ┌───────────┐  ┌─────────┐ │
│  │  Engine  │──│  Protocol  │──│ Transport │──│  MQTT   │ │
│  │          │  │  Adapters  │  │   Layer   │  │ Broker  │ │
│  └──────────┘  └────────────┘  └───────────┘  └────┬────┘ │
│                                                     │      │
│  registry/              SQLite Event Store          │      │
│  └── unilabos/  (YAML schemas + Rust codecs)        │      │
└─────────────────────────────────────────────────────┼──────┘
                                                      │
         Multiple transport paths ────────────────────┤
                                                      │
     ┌────────────────┬──────────────┬────────────────┘
     │                │              │
  WiFi/MQTT       USB Serial       TCP
     │                │              │
┌────▼────┐     ┌────▼────┐   ┌────▼────┐
│  ESP32  │     │ Direct  │   │ Network │
│  Child  │     │ Device  │   │ Device  │
│  Node   │     │         │   │         │
└────┬────┘     └────┬────┘   └─────────┘
     │ 485/232       │ USB
  Syringe          Balance
   Pump
```

## How It Works

**Mother node** — A Raspberry Pi, PC, or server running the OSDL engine with an embedded MQTT broker, SQLite event store, and mDNS service discovery. It holds the device registry, runs driver logic, and exposes a unified API.

**Child node** — A low-cost ESP32 module (~$5) with a serial interface (RS-485/232/USB). It is a **transparent serial-to-MQTT bridge** — it does not run drivers or understand device protocols. All intelligence lives on the mother.

**Transport layer** — Separates *how bytes reach devices* from *what bytes mean*:

| Transport | Latency | Use Case |
|-----------|---------|----------|
| **MqttSerial** | 5-20ms | RS-485/232 devices via ESP32 WiFi bridge |
| **DirectSerial** | < 1ms | USB devices plugged directly into mother |
| **TCP** | 1-5ms | Modbus TCP, SCPI, network instruments |

**Lifecycle:**
1. Child node boots → mDNS discovers mother → connects to MQTT broker → reports hardware ID
2. Mother matches hardware ID to a driver in the registry
3. Mother creates a Transport + Device, encodes/decodes via ProtocolAdapter
4. Commands flow: Application → Engine → Transport → Device
5. Responses flow: Device → Transport → Engine → Application

## Key Concepts

- **Transport** — How bytes reach a device (MQTT serial, direct USB, TCP socket). Each device has one transport. The engine doesn't care which kind.
- **ProtocolAdapter** — What bytes mean. Adapts a device driver ecosystem's description standard. Encodes commands to bytes, decodes responses to status. First supported: UniLabOS.
- **Lightweight child (~$5)** — ESP32 as a serial-to-MQTT bridge. No OS, no drivers, no Docker. ~220 lines of firmware with mDNS auto-discovery.
- **Event Store** — Append-only SQLite log of all events, commands, and raw serial bytes for forensic replay and debugging.
- **Embeddable** — Use `osdl-core` as a Rust library in your application, or run `osdl-cli` as a standalone process.

## Project Structure

```
crates/
├── osdl-core/                   # Core library
│   └── src/
│       ├── engine.rs            # OsdlEngine — main loop, dispatching
│       ├── transport/           # Transport trait + implementations
│       │   ├── mod.rs           # Transport trait, TransportRx
│       │   ├── mqtt_serial.rs   # MQTT serial (ESP32 bridge)
│       │   ├── direct_serial.rs # Direct USB/RS-232 (stub)
│       │   └── tcp.rs           # TCP socket (stub)
│       ├── adapter/             # ProtocolAdapter trait + implementations
│       │   ├── mod.rs           # ProtocolAdapter trait
│       │   ├── unilabos.rs      # UniLabOS ecosystem adapter
│       │   └── runze.rs         # Runze syringe pump codec
│       ├── broker.rs            # Embedded MQTT broker (rumqttd)
│       ├── mdns.rs              # mDNS service discovery
│       ├── store.rs             # SQLite event store
│       ├── protocol.rs          # Unified device model
│       ├── event.rs             # OsdlEvent enum
│       └── config.rs            # OsdlConfig
├── osdl-cli/                    # Standalone binary (mother node)
│   └── src/main.rs
registry/
└── unilabos/                    # Device YAML schemas
firmware/
└── esp32/                       # Child node firmware (PlatformIO)
```

## Build & Run

```bash
cargo build              # Build all crates
cargo run --bin osdl     # Run mother node
cargo test               # Run tests (24 tests: unit + integration + e2e)
```

## Status

Early development. Core engine, MQTT serial transport, Runze syringe pump driver, and ESP32 firmware are functional. Direct serial and TCP transports are stubbed.

## License

MIT
