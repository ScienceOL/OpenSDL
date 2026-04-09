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
│  │  ┌───────────────────┤  Driver Manager      │         │    │
│  │  │ Rust native       │  MqttSerial (Python) │         │    │
│  │  └───────────────────┴──────────────────────┘         │    │
│  │                                                        │    │
│  └────────────────────────┬───────────────────────────────┘    │
│                           │                                     │
│  ┌────────────────────────▼────────────────────────────┐       │
│  │              MQTT Broker (embedded)                   │       │
│  └────────────────────────┬────────────────────────────┘       │
└───────────────────────────┼─────────────────────────────────────┘
                            │
                            │ MQTT over WiFi / LAN
                            │
       ┌────────────────────┼────────────────┐
       │                    │                │
 ┌─────▼─────┐       ┌─────▼─────┐    ┌─────▼─────┐
 │ Child Node│       │ Child Node│    │ Child Node│
 │  (ESP32)  │       │  (ESP32)  │    │  (ESP32)  │
 │   ~$5     │       │   ~$5     │    │   ~$5     │
 │           │       │           │    │           │
 │ Serial ◄──┤       │ Serial ◄──┤    │ Serial ◄──┤
 │ bridge    │       │ bridge    │    │ bridge    │
 └─────┬─────┘       └─────┬─────┘    └─────┬─────┘
       │ RS-485            │ RS-232         │ USB
    Heater               Pump            Balance
```

## Child Node (ESP32)

Child nodes are **dumb serial-to-MQTT bridges**. No drivers, no protocol parsing, no OS.

Minimal firmware (~hundreds of lines):
- Boot → WiFi connect → MQTT connect
- Publish registration: `osdl/nodes/{node_id}/register { hardware_id, baud_rate }`
- Subscribe `osdl/serial/{node_id}/tx` → write bytes to UART
- UART receive → publish `osdl/serial/{node_id}/rx`

Hardware: ESP32-S3 (~$3) + RS-485 transceiver (~$1) + PCB. Can be built as a small dongle.

## Dual Driver Model

All driver logic runs on the **mother node**. Two paths, both producing serial bytes sent over MQTT:

### Path A — Rust Native Driver (preferred for new devices)

```rust
fn set_temperature(&self, temp: f64) -> Vec<u8> {
    build_modbus_frame(0x01, 0x06, 0x000B, (temp * 10.0) as u16)
}
// → MQTT publish to osdl/serial/{node_id}/tx
```

### Path B — Python Compatibility Layer (for existing UniLabOS drivers)

```python
# Existing driver runs unmodified on mother, with injected MqttSerial
heater = HeaterStirrer_DaLong.__new__(HeaterStirrer_DaLong)
heater.serial = MqttSerial("heater-01", mqtt_client)
heater.set_temperature(80)
# MqttSerial.write() → MQTT publish to osdl/serial/{node_id}/tx
```

`MqttSerial` is a drop-in replacement for `serial.Serial` that routes bytes over MQTT to the child node. Existing Python drivers need zero code changes.

## Data Flow

### Command Execution (application → device)

```
Host application calls engine.send_command(cmd)
  → OsdlEngine routes to correct ProtocolAdapter
  → Adapter invokes driver (Rust native or Python with MqttSerial)
  → Driver generates serial bytes
  → MQTT publish: osdl/serial/{node_id}/tx
  → Child node receives, writes bytes to UART
  → Device executes
```

### Device Status Reporting (device → application)

```
Device sends response bytes on serial
  → Child node reads UART
  → MQTT publish: osdl/serial/{node_id}/rx
  → Mother receives bytes
  → Driver parses response, extracts status
  → OsdlEvent::DeviceStatus emitted
  → Host application receives via event channel
```

### Child Node Registration (first boot)

```
Child boots
  → WiFi connect → MQTT connect
  → Publish: osdl/nodes/{node_id}/register { hardware_id, baud_rate }
  → Mother's Node Manager receives
  → Looks up hardware_id in registry
  → Instantiates matching driver (Rust native or Python with MqttSerial)
  → Device is now controllable
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
  ├── MQTT topics:   osdl/serial/{node_id}/tx and /rx for byte tunneling
  │                  → status payload format, command payload format
  └── MqttSerial:    how to inject MqttSerial into existing Python drivers
                     → replaces serial.Serial, routes bytes over MQTT

Future SiLA Adapter:
  ├── XML format:  SiLA 2 Feature Definition Language
  ├── Driver format: SiLA server implementations
  ├── Communication: SiLA uses gRPC (would need MQTT bridge)
  └── Integration:  different driver instantiation
```

## MQTT Topic Convention

```
# Node management
osdl/nodes/{node_id}/register              # child → mother: hardware ID, baud rate
osdl/nodes/{node_id}/heartbeat             # child → mother: alive ping

# Serial byte tunneling
osdl/serial/{node_id}/tx                   # mother → child: bytes to write to UART
osdl/serial/{node_id}/rx                   # child → mother: bytes read from UART

# Device-level (after mother parses serial responses via driver)
osdl/devices/{device_id}/status            # mother publishes parsed device status
osdl/devices/{device_id}/online            # retained + LWT
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
Child Nodes (ESP32) → Serial → Devices
```

Runner integration points:
- `osdl-core` as optional Rust crate dependency (`feature = "osdl"`)
- New message types in Runner protocol: `osdl_list_devices`, `osdl_send_command`, etc.
- OsdlEvent forwarded to cloud via existing WebSocket, same pattern as PTY events
- Desktop Tauri app also gets direct access for local UI (device panel)

## Security Considerations

- MQTT broker should use TLS + authentication in production
- MqttSerial runs Python drivers in a sandboxed process on the mother
- Driver code is from the local registry — mother controls what gets loaded
- Child nodes are minimal firmware with no attack surface beyond MQTT
