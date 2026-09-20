# OpenSDL

**Open Self-Drive Lab** — A mesh-based system for laboratory hardware control with pluggable transports.

## Install the CLI

Install the pre-built **`lab`** command. Rust, Docker, and a source checkout are
not required. The installer detects your operating system and CPU.

**Linux / macOS:**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ScienceOL/OpenSDL/releases/latest/download/lab-cli-installer.sh | sh
```

**Windows (PowerShell):**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/ScienceOL/OpenSDL/releases/latest/download/lab-cli-installer.ps1 | iex"
```

Open a new terminal after installation, then check:

```bash
lab --version
lab --help
```

The default install directory is `~/.cargo/bin` (`%USERPROFILE%\.cargo\bin` on
Windows), or `$CARGO_HOME/bin` when configured. If `lab` is not found, follow
the installer's PATH instructions or add that directory to your PATH.

Update to the latest release by running the installer again, or use the
installed updater:

```bash
lab-cli-update
```

| Platform | Pre-built targets |
|---|---|
| macOS | Apple Silicon (ARM64), Intel (x86-64) |
| Linux | x86-64 GNU / musl, ARM64 GNU |
| Windows | x86-64 MSVC |

Archives and SHA-256 checksums are also available on the
[latest release page](https://github.com/ScienceOL/OpenSDL/releases/latest).
The installer filenames and updater use the package name `lab-cli`; the
command you run is `lab`.

## Quick start

Start a local server in one terminal:

```bash
lab serve
```

In a second terminal:

```bash
lab status
lab device list
lab stop
```

With no hardware configured, an empty device list is expected. By default,
`lab serve` starts an MQTT broker on port 1883 and uses a local Unix socket
on Linux/macOS or loopback TCP on Windows. `lab` discovers the local server
automatically. On Linux/macOS, use `lab serve --detach` to run it in the
background.

To connect laboratory hardware, download the matching source archive from the
release page (or clone this repository) for its `registry/unilabos` device
schemas and [recipe configurations](docs/recipes/README.md). The CLI installer
installs executables; it does not install device schemas or flash ESP32 boards.
From the repository root, start with:

```bash
lab serve --registry registry/unilabos
```

Without those schemas, the default server logs a registry-loading warning;
its API remains available, but it cannot match UniLabOS devices. Follow the
recipe for your hardware to configure transports and firmware. For all options,
run `lab serve --help`.

Portable asset commands run independently of the hardware server:

```bash
lab validate path/to/asset
lab pack path/to/asset --output path/to/oci-layout
lab inspect path/to/oci-layout
lab push --help
lab pull --help
```

## What is OpenSDL?

OpenSDL connects laboratory hardware to your application through a unified
control layer. Protocol adapters let it consume device descriptions and encode
commands for multiple driver ecosystems without running their platform software.
The included UniLabOS adapter supports the
[Uni-Lab-OS](https://github.com/deepmodeling/Uni-Lab-OS) device-description
ecosystem.

```
              Your Application (Liyan Labs, LIMS, custom)
                        │
                        │  gRPC / CLI
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
- **Lightweight node (~$5)** — ESP32 as a serial-to-MQTT bridge. No OS, no drivers, no Docker. Firmware bridges bytes and supports node discovery.
- **Event Store** — Append-only SQLite log of all events, commands, and raw serial bytes for forensic replay and debugging.
- **Embeddable** — Use `osdl-core` as a Rust library in your application, or run `lab-cli` as a standalone process.

## Project Structure

```
crates/
├── osdl-core/                   # Core library
│   └── src/
│       ├── engine.rs            # OsdlEngine — main loop, dispatching
│       ├── transport/           # Transport trait + implementations
│       │   ├── mod.rs           # Transport trait, TransportRx
│       │   ├── mqtt_serial.rs   # MQTT serial (ESP32 bridge)
│       │   ├── direct_serial.rs # Direct USB/RS-232/RS-485 (serial feature)
│       │   └── tcp.rs           # TCP socket
│       ├── adapter/             # ProtocolAdapter trait + implementations
│       │   ├── mod.rs           # ProtocolAdapter trait
│       │   ├── unilabos.rs      # UniLabOS ecosystem adapter
│       │   └── onvif.rs         # ONVIF camera adapter
│       ├── driver/builtins/      # Runze, Emm, Laiyu, Sopa, XKC codecs
│       ├── media/               # Camera streaming gateway
│       ├── broker.rs            # Embedded MQTT broker (rumqttd)
│       ├── mdns.rs              # mDNS service discovery
│       ├── store.rs             # SQLite event store
│       ├── protocol.rs          # Unified device model
│       ├── event.rs             # OsdlEvent enum
│       └── config.rs            # OsdlConfig
├── lab-cli/                    # lab CLI: server, client, portable assets
├── osdl-server/                # gRPC service over local sockets / TCP
├── osdl-proto/                 # Shared protobuf / gRPC contract
├── opensdl-assets/             # Asset validation, OCI packaging and registry I/O
└── osdl-firmware-protocol/      # Shared ESP-NOW wire protocol
registry/
└── unilabos/                    # Device YAML schemas
firmware/
├── esp32/                       # ESP32 Rust firmware
├── esp32s3/                     # ESP32-S3 Rust firmware
└── esp32-cpp/                   # MQTT bridge (C++ / PlatformIO)
```

## Build from source

Install a current stable [Rust toolchain](https://rustup.rs/) and your platform's
C/C++ build tools (Xcode Command Line Tools on macOS, a C compiler on Linux,
or Visual Studio Build Tools with the C++ workload on Windows). Protobuf's
compiler is supplied by the build; you do not need to install it separately.

```bash
git clone https://github.com/ScienceOL/OpenSDL.git
cd OpenSDL
cargo build --locked --release -p lab-cli
cargo run --locked --bin lab -- serve --registry registry/unilabos
cargo test --workspace --locked
```

To install the CLI from this checkout:

```bash
cargo install --locked --path crates/lab-cli
```

Direct USB/RS-232/RS-485 transport is implemented behind the
`osdl-core/serial` feature. Enable it when building for direct serial hardware:

```bash
cargo install --locked --path crates/lab-cli --features osdl-core/serial
```

## Status

OpenSDL is in early development. The current CLI is `lab` (since v0.2.0).
Implemented components include the gRPC server, embedded MQTT broker, mDNS
discovery, SQLite event store, MQTT serial, ESP-NOW, TCP, optional direct
serial, ONVIF camera control, and portable assets distributed through OCI
registries. Camera streaming additionally requires MediaMTX and, for the
recipes that use it, FFmpeg; these are not installed by the CLI installer.

Hardware command dispatch does not yet correlate replies into completed
command results. A `PENDING` response means dispatched, not physically
completed. See [known issues](docs/known-issues.md) and the
[hardware recipes](docs/recipes/README.md) for current operational limits.

## Development and releases

Pull requests run the workspace tests on Linux, macOS, and Windows, and build
all six release targets and generate the shell and PowerShell installers.
Tagged versions run the same checks before publication. See
[the release procedure](docs/releases.md) for versioning and installation checks.

## License

MIT
