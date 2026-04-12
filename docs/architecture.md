# OpenSDL Architecture

## System Overview

OpenSDL is a mother-child mesh network for laboratory hardware control, with a pluggable transport layer that supports multiple communication paths.

```
┌────────────────────────────────────────────────────────────────────┐
│                     Mother Node (RPi / PC)                          │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │                      OsdlEngine (Rust)                         │ │
│  │                                                                │ │
│  │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐ │ │
│  │  │  Protocol   │  │    Node      │  │     Transport        │ │ │
│  │  │  Adapters   │  │   Manager    │  │     Layer            │ │ │
│  │  │             │  │              │  │                      │ │ │
│  │  │ - unilabos  │  │ - discovery  │  │ - MqttSerial (WiFi) │ │ │
│  │  │ - sila (…)  │  │ - provision  │  │ - DirectSerial (USB)│ │ │
│  │  │             │  │ - health     │  │ - TCP (Modbus/SCPI) │ │ │
│  │  │             │  │ - mDNS      │  │ - ESP-NOW (future)  │ │ │
│  │  └──────┬──────┘  └──────┬───────┘  └──────────┬───────────┘ │ │
│  │         │                │                      │             │ │
│  │         └────────────────┼──────────────────────┘             │ │
│  │                          │                                     │ │
│  │  ┌──────────────┐  ┌────▼──────┐  ┌──────────────────┐       │ │
│  │  │ MQTT Broker  │  │  Event    │  │  Device Registry  │       │ │
│  │  │ (embedded)   │  │  Store    │  │  (YAML schemas)   │       │ │
│  │  │  rumqttd     │  │ (SQLite)  │  │                   │       │ │
│  │  └──────┬───────┘  └──────────┘  └──────────────────┘       │ │
│  │         │                                                     │ │
│  └─────────┼─────────────────────────────────────────────────────┘ │
│            │                                                        │
└────────────┼────────────────────────────────────────────────────────┘
             │
             │  Multiple transport paths:
             │
     ┌───────┼──────────────────────────────────┐
     │       │                │                  │
     ▼       ▼                ▼                  ▼
  WiFi/MQTT    USB Serial      TCP            ESP-NOW
     │            │              │            (future)
┌────▼────┐  ┌───▼────┐   ┌────▼────┐
│  ESP32  │  │ Direct │   │ Network │
│  Child  │  │ Device │   │ Device  │
│  Node   │  │        │   │         │
└────┬────┘  └───┬────┘   └─────────┘
     │ 485/232   │ USB
  Syringe      Balance
   Pump
```

## Transport Layer

The Transport layer separates **how bytes reach devices** from **what bytes mean**:

```
ProtocolAdapter: set_temperature(80) → [FE B1 01 50]  (WHAT)
                                             │
Transport:                                   ▼          (HOW)
  MqttSerial  → MQTT → ESP32 → RS-485 → device
  DirectSerial → /dev/ttyUSB0 → device
  Tcp         → TCP socket → device
  ESP-NOW     → radio → ESP32 → device (future)
```

Each `Device` has a `transport_id` that identifies its transport:
- MQTT serial: the node ID (e.g., `"pump-01"`)
- Direct serial: the port path (e.g., `"/dev/ttyUSB0"`)
- TCP: the host:port (e.g., `"192.168.1.50:502"`)

### Transport Comparison

| Transport | Latency | Use Case | Extra Hardware |
|-----------|---------|----------|----------------|
| **MqttSerial** | 5-20ms | RS-485/232 devices via ESP32 WiFi bridge | ESP32 ($3) + transceiver ($1) |
| **DirectSerial** | < 1ms | USB devices plugged into mother node | None |
| **TCP** | 1-5ms | Modbus TCP, SCPI over TCP, network instruments | None |
| **ESP-NOW** (future) | 1-3ms | Low-latency wireless, no WiFi needed | ESP32 USB gateway ($3) |

## Child Node (ESP32)

Child nodes are **dumb serial-to-MQTT bridges**. No drivers, no protocol parsing, no OS.

Minimal firmware (~220 lines C++):
- Boot → WiFi connect → mDNS discover mother → MQTT connect
- Publish registration: `osdl/nodes/{node_id}/register { hardware_id, baud_rate }`
- Subscribe `osdl/serial/{node_id}/tx` → write bytes to UART
- UART receive → publish `osdl/serial/{node_id}/rx`
- Periodic heartbeat: `osdl/nodes/{node_id}/heartbeat`

Hardware: ESP32-S3 (~$3) + RS-485 transceiver (~$1) + PCB. Can be built as a small dongle.

### mDNS Auto-Discovery

Child nodes automatically discover the mother node via mDNS:

```
Mother advertises: _osdl._tcp.local  (port 1883)
Child queries:     _osdl._tcp.local  → gets mother IP
                   → connects to MQTT broker
```

No hardcoded IPs needed. If mDNS fails, the child falls back to a static IP from config.

## ProtocolAdapter Design

A ProtocolAdapter abstracts a **device driver ecosystem**, not individual devices.

```
What a ProtocolAdapter knows:

UniLabOS Adapter:
  ├── YAML format:  how to parse registry/unilabos/*.yaml
  │                  → extract device capabilities, action schemas, status types
  ├── Encode:       action + params → serial bytes (Rust codec per driver)
  ├── Decode:       serial bytes → status properties (HashMap)
  └── Match:        hardware_id → DeviceMatch (type, description, actions)

Example — Runze syringe pump codec:
  encode("initialize", {})          → "/1ZR\r\n"
  encode("set_position", {pos:12.5}) → "/1A3000R\r\n"
  decode("`3000\n")                 → { status: "Idle", position: 12.5 }
```

## Data Flow

### Command Execution (application → device)

```
Host application calls engine.send_command(cmd)
  → OsdlEngine finds Device by device_id
  → Routes to correct ProtocolAdapter (by device.adapter)
  → Adapter encodes command into raw bytes
  → Engine looks up Transport by device.transport_id
  → Transport.send(bytes) delivers bytes to device
  → Device executes
```

### Device Status Reporting (device → application)

```
Device sends response bytes
  → Transport receives bytes (MQTT RX / serial read / TCP read)
  → Engine.handle_transport_rx(transport_id, bytes)
  → Finds matching Device by transport_id
  → ProtocolAdapter decodes bytes → status properties
  → OsdlEvent::DeviceStatus emitted
  → Host application receives via event channel
```

### Child Node Registration (first boot, MQTT serial only)

```
Child boots
  → WiFi connect → mDNS discover → MQTT connect
  → Publish: osdl/nodes/{node_id}/register { hardware_id, baud_rate }
  → Engine creates MqttSerialTransport for this node
  → Looks up hardware_id in ProtocolAdapters
  → Creates Device with transport_id = node_id
  → OsdlEvent::DeviceOnline emitted
```

## Event Store (SQLite)

All events are logged to an append-only SQLite database (WAL mode) for forensic replay:

- **Events**: device online/offline, status updates, unknown nodes
- **Commands**: every command sent with full parameters
- **Serial bytes**: raw TX/RX bytes with direction and timestamp

Queryable by event type, device ID, and time range.

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
Xyzen Cloud → WebSocket → Runner → OsdlEngine → Transport → Device
```

- `osdl-core` as optional crate dependency in `xyzen-runner` (`feature = "osdl"`)
- New Runner message types: `osdl_list_devices`, `osdl_send_command`, etc.
- OsdlEvent forwarded to cloud via existing WebSocket (same pattern as PTY events)
- Desktop Tauri app also gets direct access for local device UI

## Wireless Communication Options

### Current: WiFi + MQTT (via ESP32 child node)

| Component | Typical Latency | Worst Case |
|-----------|----------------|------------|
| WiFi (same LAN) | 1-5ms | 50-500ms (interference) |
| MQTT QoS 1 | 2-10ms | 100ms+ |
| TCP reconnect | — | 3-10s |
| **Total** | **5-20ms** | **500ms - 10s** |

Sufficient for most lab devices (pumps, heaters, stirrers) whose mechanical response time is 100ms+.

### Future: ESP-NOW (planned)

ESP-NOW uses raw 802.11 frames (vendor-specific Action Frames) for MAC-to-MAC communication, bypassing the entire WiFi/IP/TCP/MQTT stack:

```
WiFi+MQTT: [802.11] → [IP] → [TCP] → [MQTT] → payload  (~100B overhead, stateful)
ESP-NOW:   [802.11] → [ESP-NOW header] → payload        (~30B overhead, stateless)
```

- **1-3ms latency**, minimal jitter
- No WiFi router, no broker, no TCP connections
- 250 bytes/packet (plenty for serial commands)
- Requires one ESP32 as USB gateway on the mother node

Architecture with ESP-NOW:

```
子機 (ESP32) ──ESP-NOW──▶ Gateway (ESP32) ──USB Serial──▶ Mother Node
                                                           │
                                                    EspNowTransport
```

### When to Use Which

| Scenario | Recommended Transport |
|----------|----------------------|
| General lab automation (pumps, heaters) | WiFi + MQTT |
| Low-latency closed-loop control (PID < 50ms) | ESP-NOW or Direct Serial |
| Network instruments (Modbus TCP, SCPI) | TCP |
| USB devices on mother node | Direct Serial |
| Emergency stop (E-Stop) | ESP-NOW (no connection to lose) |

## Security Considerations

- MQTT broker should use TLS + authentication in production
- Driver code is from the local registry — mother controls what gets loaded
- Child nodes are minimal firmware with no attack surface beyond MQTT
- Event store provides complete audit trail of all commands and responses
