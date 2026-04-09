# OpenSDL

**Open Self-Drive Lab** — A mesh-based system for laboratory hardware control via MQTT.

## What is OpenSDL?

OpenSDL is a mother-child mesh system that connects laboratory hardware to your application. It reuses existing device driver ecosystems (starting with [Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS)) without requiring their platform software to run.

```
              Your Application (Xyzen, LIMS, custom)
                        │
                        │  Rust crate / CLI
                        │
┌───────────────────────▼────────────────────────────────────┐
│                     Mother Node                             │
│                     (RPi / PC / Server)                     │
│                                                             │
│  ┌──────────┐  ┌────────────┐  ┌──────────┐  ┌──────────┐ │
│  │  Engine  │──│  Protocol  │──│  Driver   │──│   MQTT   │ │
│  │          │  │  Adapters  │  │  Manager  │  │  Broker  │ │
│  └──────────┘  └────────────┘  └──────────┘  └────┬─────┘ │
│                                                    │       │
│  registry/                                         │       │
│  └── unilabos/  (YAML schemas + driver code)       │       │
└────────────────────────────────────────────────────┼───────┘
                                                     │
                          MQTT (WiFi / LAN)          │
                    ┌────────────┬───────────────────┘
                    │            │
              ┌─────▼─────┐ ┌───▼─────┐
              │ Child Node│ │  Child  │
              │  (ESP32)  │ │  Node   │   ...
              │   ~$5     │ │ (ESP32) │
              │           │ │         │
              │ Serial ◄──┤ │         │
              │ bridge    │ │         │
              └─────┬─────┘ └────┬────┘
                    │ 485/232    │ USB
                 Heater        Pump
```

## How It Works

**Mother node** — A Raspberry Pi, PC, or server running the OSDL engine with an embedded MQTT broker. It holds the device registry, runs driver logic, and exposes a unified API to your application.

**Child node** — A low-cost ESP32 module (~$5) with a serial interface (RS-485/232/USB). It is a **transparent serial-to-MQTT bridge** — it does not run drivers or understand device protocols. All intelligence lives on the mother.

**Two driver paths on the mother:**

| Path | How it works | When to use |
|------|-------------|-------------|
| **Rust native** | Driver written in Rust, generates serial bytes directly, sends over MQTT | New drivers, performance-critical |
| **Python compat** | Existing driver (e.g. UniLabOS), `MqttSerial` injected to replace `serial.Serial`, bytes route over MQTT | Reusing 30+ existing UniLabOS drivers |

Both paths produce the same result: serial bytes sent over MQTT to the child node.

**Lifecycle:**
1. Child node boots → connects to MQTT broker → reports hardware ID
2. Mother matches hardware ID to a driver in the registry
3. Mother instantiates the driver (Rust native or Python with MqttSerial)
4. Commands flow: Application → Mother (driver) → MQTT → Child → Serial → Device
5. Responses flow: Device → Serial → Child → MQTT → Mother (driver) → Application

## Key Concepts

- **ProtocolAdapter** — Adapts a device driver ecosystem's description standard. First supported: UniLabOS. The adapter parses YAML schemas, understands driver conventions, and translates between the ecosystem's format and OpenSDL's unified model.
- **Lightweight child (~$5)** — ESP32 as a serial-to-MQTT bridge. No OS, no drivers, no Docker. Just firmware that transparently tunnels serial bytes over MQTT.
- **Driver on the mother** — All protocol intelligence runs on the mother node. Rust native drivers for new devices; Python compatibility layer for existing ecosystems.
- **MqttSerial** — Drop-in replacement for `serial.Serial` that routes read/write over MQTT. Lets existing Python drivers run unmodified on the mother, talking to remote child nodes.
- **Embeddable** — Use `osdl-core` as a Rust library in your application, or run `osdl-cli` as a standalone process.

## Project Structure

```
crates/
├── osdl-core/     # Core library: engine, protocol, MQTT, adapter trait, driver manager
└── osdl-cli/      # Standalone binary (mother node entry point)
registry/
└── unilabos/      # Device definitions (YAML) + Python drivers
firmware/
└── esp32/         # Child node firmware (serial-to-MQTT bridge)
```

## Status

Early development. Not yet usable.

## License

MIT
