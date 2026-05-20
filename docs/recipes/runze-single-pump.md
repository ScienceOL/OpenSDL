# Recipe — Single Runze syringe pump (legacy 1:1)

Replaces the original `runze_via_espnow.rs` example. Drives one Runze
SY-03B pump that's the **only** device on its RS-485 bus, fronted by an
ESP-NOW child whose firmware advertises `hardware_id =
syringe_pump_with_valve.runze.SY03B-T06`.

This is the simplest path: no bus manifest, the engine takes the legacy
1:1 registration route and creates one device keyed on the child's MAC.

## Prerequisites

- ESP-NOW gateway plugged into a serial port. We use
  `/dev/cu.usbserial-A5069RR4` below; substitute yours.
- ESP-NOW child + Runze pump powered on.

## Walk-through

### 1. Start the server

```sh
osdl serve --detach \
  --instance pump \
  --registry $(pwd)/registry/unilabos \
  --espnow-port /dev/cu.usbserial-A5069RR4
```

The detach flag forks a background daemon and prints the log file path.
Substitute `--detach` with running in the foreground if you'd rather watch
log output directly (drop the flag entirely).

### 2. Wait for the pump to register

```sh
osdl --instance pump device wait \
  type:syringe_pump_with_valve.runze.SY03B-T06 \
  --timeout 20s
```

The device id will be `espnow:<MAC>` (e.g. `espnow:30EDA0B65B38`). Capture
it for later commands:

```sh
DEV=$(osdl --instance pump device list --json \
       | python3 -c 'import sys,json; print(json.load(sys.stdin)[0]["id"])')
echo "$DEV"
```

### 3. Initialize and probe

```sh
osdl --instance pump send "$DEV" initialize
# pump goes Busy for ~6s while homing, then Idle.

osdl --instance pump send "$DEV" query_status
osdl --instance pump send "$DEV" query_position
osdl --instance pump send "$DEV" query_valve_position

# Snapshot decoded properties (status / position / etc.).
osdl --instance pump device get "$DEV"
```

### 4. Move the valve

```sh
osdl --instance pump send "$DEV" set_valve_position -p position=3
sleep 2
osdl --instance pump send "$DEV" set_valve_position -p position=1
```

Integer JSON params arrive at the codec as `u64` (the
`Struct.NumberValue` round-trip preserves whole-number floats — see
`osdl-server/src/convert.rs`).

### 5. Watch live events

In another terminal:

```sh
osdl --instance pump events --kinds device_status,command_result
```

### 6. Stop the server

```sh
osdl --instance pump stop
```

## Success criteria

- `device wait` returns within 20 s with a populated action list.
- `initialize` causes the pump to physically home (audible motion, then
  silence as it returns to Idle).
- `set_valve_position` transitions the valve to the requested port; you
  can hear the rotor click into place.
