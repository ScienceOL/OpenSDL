# Recipe — ChinWe separator station

Replaces the manual portions of `move_chinwe.rs` /  `test_chinwe.rs`. The
full 5-device ChinWe bus, all addressable from the CLI:

- `pump-1` … `pump-3` — Runze SY-03B syringe pumps
- `motor-4` — Emm V5.0 stepper used as the **stirrer**
- `motor-5` — Emm V5.0 stepper used as the **drain valve**

Bus manifest:
[`configs/chinwe-station.yaml`](configs/chinwe-station.yaml).

## Boot

```sh
osdl serve --detach \
  --instance chinwe \
  --config docs/recipes/configs/chinwe-station.yaml \
  --registry $(pwd)/registry/unilabos \
  --dongle-port /dev/cu.usbserial-A5069RR4

osdl --instance chinwe device wait id:'espnow:30EDA0B65B38:pump-1' --timeout 25s
osdl --instance chinwe device list
```

The five expected rows look like (replace MAC with yours):

| device id | type | role |
|---|---|---|
| `espnow:30EDA0B65B38:pump-1` | `syringe_pump.chinwe.pump1` | `syringe_pump` |
| `espnow:30EDA0B65B38:pump-2` | `syringe_pump.chinwe.pump2` | `syringe_pump` |
| `espnow:30EDA0B65B38:pump-3` | `syringe_pump.chinwe.pump3` | `syringe_pump` |
| `espnow:30EDA0B65B38:motor-4` | `stepper_motor.chinwe.emm4` | `stirrer` |
| `espnow:30EDA0B65B38:motor-5` | `stepper_motor.chinwe.emm5` | `drain_valve` |

## Aspirate / dispense flow on pump-1

```sh
P1='espnow:30EDA0B65B38:pump-1'

# Home plunger and valve.
osdl --instance chinwe send "$P1" initialize
sleep 8     # initialization takes ~6s, give it slack

# Aspirate 1 mL through valve port 1.
osdl --instance chinwe send "$P1" set_valve_position -p position=1
sleep 1
osdl --instance chinwe send "$P1" pull_plunger -p volume=1.0
sleep 5

# Dispense through valve port 2.
osdl --instance chinwe send "$P1" set_valve_position -p position=2
sleep 1
osdl --instance chinwe send "$P1" push_plunger -p volume=1.0
```

Watch state in real time from another shell:

```sh
osdl --instance chinwe events --kinds device_status,command_result
```

## Independent stirrer + drain commands

```sh
M4='espnow:30EDA0B65B38:motor-4'   # stirrer
M5='espnow:30EDA0B65B38:motor-5'   # drain valve

# Stir at 60 RPM for 10s, then stop. See stir-10s.md for the full recipe.
osdl --instance chinwe send "$M4" enable     -p enable=true
osdl --instance chinwe send "$M4" run_speed  -p speed=60 -p direction=0 -p acceleration=10
sleep 10
osdl --instance chinwe send "$M4" stop

# Open drain valve ~1/4 turn, hold 10s, close.
osdl --instance chinwe send "$M5" enable     -p enable=true
osdl --instance chinwe send "$M5" run_position \
   -p pulses=800 -p speed=30 -p direction=0 -p acceleration=10 -p absolute=false
sleep 10
osdl --instance chinwe send "$M5" run_position \
   -p pulses=800 -p speed=30 -p direction=1 -p acceleration=10 -p absolute=false
```

## Shutdown

```sh
osdl --instance chinwe stop
```
