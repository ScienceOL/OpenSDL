# OpenSDL

**Open Self-Drive Lab** — A platform-agnostic protocol adapter for laboratory hardware control via MQTT.

## What is OpenSDL?

OpenSDL sits between your application and laboratory hardware platforms. It doesn't drive devices directly — that's the job of platforms like [Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS), SiLA, or vendor-specific systems. Instead, OpenSDL provides a **unified interface** to talk to any of them.

```
Your Application (Xyzen, LIMS, custom)
        │
        │  Rust crate / CLI
        │
   ┌────▼─────────────────────────────┐
   │            OpenSDL                │
   │                                   │
   │  ┌───────────┐  ┌───────────┐    │
   │  │ UniLabOS  │  │  SiLA 2   │ ...│  ← PlatformAdapter trait
   │  │ Adapter   │  │  Adapter  │    │
   │  └─────┬─────┘  └─────┬─────┘    │
   └────────┼───────────────┼──────────┘
            │ MQTT          │ MQTT / gRPC
            ▼               ▼
      UniLabOS Gateway    SiLA Server
            │               │
         Hardware         Hardware
```

## Key Concepts

- **PlatformAdapter** — Each lab platform (UniLabOS, SiLA, vendor SDK) gets one adapter that translates its device management protocol into OpenSDL's unified model.
- **MQTT-native** — Devices report status and receive commands over MQTT on your local network.
- **Stateless** — OpenSDL holds no persistent state. It is a real-time bridge between your application and hardware platforms.
- **Embeddable** — Use `osdl-core` as a Rust library in your own application, or run `osdl-cli` as a standalone process for headless / edge deployment.

## Architecture

OpenSDL does **not** abstract individual devices (serial, Modbus, OPC-UA) — that is the responsibility of each hardware platform. OpenSDL abstracts the **platforms themselves**: each platform has its own discovery mechanism, status reporting format, and command protocol. A `PlatformAdapter` normalizes these differences into a single device model.

```
crates/
├── osdl-core/     # Core library: engine, protocol, MQTT client, adapter trait
└── osdl-cli/      # Standalone binary
```

## Status

Early development. Not yet usable.

## License

MIT
