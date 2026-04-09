# OpenSDL

**Open Self-Drive Lab** — A mesh-based system for laboratory hardware control via MQTT.

## What is OpenSDL?

OpenSDL is a mother-child mesh system that connects laboratory hardware to your application. It reuses existing device driver ecosystems (starting with [Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS)) without requiring their platform software to run.

```
              Your Application (Xyzen, LIMS, custom)
                        │
                        │  Rust crate / CLI
                        │
┌───────────────────────▼──────────────────────────────┐
│                  Mother Node                          │
│                  (RPi / PC / Server)                  │
│                                                       │
│  ┌────────────┐  ┌────────────┐  ┌────────────────┐  │
│  │ OsdlEngine │  │  Protocol  │  │  MQTT Broker   │  │
│  │            │──│  Adapters  │──│  (embedded)    │  │
│  └────────────┘  └────────────┘  └───────┬────────┘  │
│                                          │            │
│  registry/                               │            │
│  ├── unilabos/*.yaml  (device schemas)   │            │
│  └── unilabos/drivers/ (Python drivers)  │            │
└──────────────────────────────────────────┼────────────┘
                                           │ MQTT (WiFi / LAN)
                          ┌────────────────┼────────────────┐
                          │                │                │
                    ┌─────▼─────┐   ┌──────▼────┐   ┌──────▼────┐
                    │ Child Node│   │ Child Node│   │ Child Node│
                    │ (RPi Zero)│   │ (RPi Zero)│   │ (RPi Zero)│
                    │           │   │           │   │           │
                    │ Docker    │   │ Docker    │   │ Docker    │
                    │ container │   │ container │   │ container │
                    │  ┌──────┐ │   │  ┌──────┐ │   │  ┌──────┐ │
                    │  │Python│ │   │  │Python│ │   │  │Python│ │
                    │  │driver│ │   │  │driver│ │   │  │driver│ │
                    │  └──┬───┘ │   │  └──┬───┘ │   │  └──┬───┘ │
                    └─────┼─────┘   └─────┼─────┘   └─────┼─────┘
                          │ Serial        │ Serial        │ Serial
                          │ (485/232/USB) │               │
                       Heater           Pump           Balance
```

## How It Works

**Mother node** — A Raspberry Pi, PC, or server running the OSDL engine with an embedded MQTT broker. It manages child nodes, holds the device registry, and exposes a unified API to your application.

**Child node** — A small Linux SBC (e.g. Raspberry Pi Zero 2 W, ~$15) with a USB-to-serial adapter. Each child runs one device driver in a Docker container, communicating with hardware via real serial ports and with the mother via MQTT.

**Lifecycle:**
1. Child node boots, connects to MQTT broker, reports its hardware ID
2. Mother looks up the hardware ID in the registry, pushes the matching driver + config
3. Child runs the driver in a Docker container with serial port access (`--device /dev/ttyUSB0`)
4. Driver operates the device natively — no I/O interception, no virtual serial ports
5. Status reports and commands flow over MQTT between child and mother

## Key Concepts

- **ProtocolAdapter** — Adapts a device driver ecosystem's description format (YAML schemas, driver code, MQTT conventions). First supported: UniLabOS. The adapter does not abstract individual hardware — it abstracts the *standard* that describes hardware.
- **Driver runs on the child** — Real Python drivers execute on the physical node connected to the device. The mother never touches serial bytes.
- **Docker isolation** — Each driver runs in its own container. Serial access via `--device` mapping, zero performance overhead.
- **MQTT mesh** — All mother-child communication over MQTT. Child nodes are self-contained; they continue operating if the mother goes offline.
- **Embeddable** — Use `osdl-core` as a Rust library in your application, or run `osdl-cli` as a standalone process.

## Project Structure

```
crates/
├── osdl-core/     # Core library: engine, protocol, MQTT, adapter trait
└── osdl-cli/      # Standalone binary (mother node entry point)
registry/
└── unilabos/      # Device definitions + drivers in UniLabOS format
```

## Status

Early development. Not yet usable.

## License

MIT
