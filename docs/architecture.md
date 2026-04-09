# OpenSDL Architecture

## System Overview

OpenSDL is a mother-child mesh network for laboratory hardware control.

```
┌──────────────────────────────────────────────────────────────┐
│                     Mother Node (RPi / PC)                    │
│                                                               │
│  ┌──────────────────────────────────────────────────────┐    │
│  │                    OsdlEngine (Rust)                   │    │
│  │                                                        │    │
│  │  ┌─────────────┐  ┌──────────────┐  ┌─────────────┐  │    │
│  │  │  Protocol   │  │    Node      │  │   Device    │  │    │
│  │  │  Adapters   │  │   Manager    │  │  Registry   │  │    │
│  │  │             │  │              │  │             │  │    │
│  │  │ - unilabos  │  │ - discovery  │  │ - YAML      │  │    │
│  │  │ - sila (…)  │  │ - provision  │  │ - drivers   │  │    │
│  │  │             │  │ - health     │  │ - schemas   │  │    │
│  │  └──────┬──────┘  └──────┬───────┘  └──────┬──────┘  │    │
│  │         └────────────┬───┘                  │         │    │
│  │                      │                      │         │    │
│  └──────────────────────┼──────────────────────┘         │    │
│                         │                                 │    │
│  ┌──────────────────────▼────────────────────────────┐   │    │
│  │              MQTT Broker (embedded)                 │   │    │
│  └──────────────────────┬────────────────────────────┘   │    │
└─────────────────────────┼────────────────────────────────┘    │
                          │                                      │
                          │ MQTT over WiFi / LAN                 │
                          │                                      │
         ┌────────────────┼────────────────┐                     │
         │                │                │                      │
   ┌─────▼─────┐   ┌─────▼─────┐   ┌─────▼─────┐               │
   │ Child Node│   │ Child Node│   │ Child Node│               │
   │ (RPi Zero)│   │ (RPi Zero)│   │ (RPi Zero)│               │
   │           │   │           │   │           │               │
   │ ┌───────┐ │   │ ┌───────┐ │   │ ┌───────┐ │               │
   │ │Docker │ │   │ │Docker │ │   │ │Docker │ │               │
   │ │  ┌──┐ │ │   │ │  ┌──┐ │ │   │ │  ┌──┐ │ │               │
   │ │  │Py│ │ │   │ │  │Py│ │ │   │ │  │Py│ │ │               │
   │ │  └┬─┘ │ │   │ │  └┬─┘ │ │   │ │  └┬─┘ │ │               │
   │ └───┼───┘ │   │ └───┼───┘ │   │ └───┼───┘ │               │
   └─────┼─────┘   └─────┼─────┘   └─────┼─────┘               │
         │ 485/232/USB    │               │                      │
         │                │               │
      Heater            Pump           Balance
```

## Data Flow

### Device Status Reporting (child → application)

```
Device (hardware)
  → serial bytes
  → Python driver (in Docker on child node)
  → driver parses response, extracts status
  → MQTT publish: osdl/unilabos/{node_id}/{device_id}/status
  → Mother MQTT broker receives
  → OsdlEngine ProtocolAdapter parses payload
  → OsdlEvent::DeviceStatus emitted
  → Host application receives via event channel
```

### Command Execution (application → device)

```
Host application calls engine.send_command(cmd)
  → OsdlEngine routes to correct ProtocolAdapter
  → Adapter serializes to platform-specific MQTT format
  → MQTT publish: osdl/unilabos/{node_id}/{device_id}/command
  → Child node's MQTT client receives
  → Passes to Python driver in Docker container
  → Driver generates serial bytes, writes to port
  → Device executes
  → Driver reads response
  → MQTT publish: osdl/unilabos/{node_id}/{device_id}/command/ack
  → Mother receives, emits OsdlEvent::CommandFeedback
```

### Child Node Provisioning (first boot)

```
Child boots with fresh OS
  → MQTT connect to mother broker
  → Publish: osdl/nodes/{node_id}/register { hardware_id, serial_ports, ... }
  → Mother's Node Manager receives
  → Looks up hardware_id in registry
  → Finds matching driver (e.g. registry/unilabos/drivers/dalong.py)
  → Publish: osdl/nodes/{node_id}/provision { driver_image, config, ... }
  → Child pulls Docker image, starts container
  → Container runs driver with --device /dev/ttyUSB0
  → Driver connects to device, begins status reporting
```

## ProtocolAdapter Design

A ProtocolAdapter abstracts a **device driver ecosystem**, not individual devices.

```
What a ProtocolAdapter knows:

UniLabOS Adapter:
  ├── YAML format:  how to parse registry/unilabos/*.yaml
  │                  → extract device capabilities, action schemas, status types
  ├── Driver format: UniLabOS Python driver conventions
  │                  → class with methods, serial.Serial usage, property decorators
  ├── MQTT topics:   osdl/unilabos/{node_id}/{device_id}/...
  │                  → status payload format, command payload format
  └── Provisioning:  how to package a UniLabOS driver into a Docker image
                     → Dockerfile template, pyserial dependency, entry point

Future SiLA Adapter:
  ├── XML format:  SiLA 2 Feature Definition Language
  ├── Driver format: SiLA server implementations
  ├── Communication: SiLA uses gRPC (would need MQTT bridge on child)
  └── Provisioning:  different container setup
```

## Child Node Hardware

Recommended: **Raspberry Pi Zero 2 W** (~$15)
- WiFi built-in → MQTT connectivity
- USB OTG → USB-to-485/232 adapter
- Runs Linux + Docker
- 512MB RAM — sufficient for Python + pyserial driver

Serial adapter options:
- USB-to-RS485 dongle (~$5)
- USB-to-RS232 dongle (~$5)
- Direct USB if device supports it

Total per child node: **~$20-25**

### Serial Port Stability

Use udev rules to ensure stable device names across reboots and hot-plug:

```bash
# /etc/udev/rules.d/99-osdl.rules
SUBSYSTEM=="tty", ATTRS{idVendor}=="1a86", ATTRS{serial}=="ABC123", SYMLINK+="osdl/heater-01"
```

Docker maps the stable symlink:
```bash
docker run --device /dev/osdl/heater-01:/dev/ttyUSB0 driver-heater
```

## Integration with Xyzen

When embedded in Xyzen Desktop (Tauri):

```
Xyzen Cloud Backend
  │ WebSocket
  ▼
Xyzen Runner (xyzen-runner crate)
  │ in-process Rust calls (osdl-core as dependency)
  ▼
OsdlEngine
  │ MQTT
  ▼
Child Nodes → Devices
```

Runner integration points:
- `osdl-core` as optional Rust crate dependency (`feature = "osdl"`)
- New message types in Runner protocol: `osdl_list_devices`, `osdl_send_command`, etc.
- OsdlEvent forwarded to cloud via existing WebSocket, same pattern as PTY events
- Desktop Tauri app also gets direct access for local UI (device panel)

## Security Considerations

- MQTT broker should use TLS + authentication in production
- Docker containers run with minimal privileges (only `--device` for serial)
- Driver code is from the registry — mother controls what gets deployed
- Child nodes should verify driver checksums before running
