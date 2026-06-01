# osdl-esp32-max485-firmware

Sibling Rust firmware crate for boards built around a **standard ESP32**
(Tensilica LX6, e.g. ESP32-D0WD-V3) with an **external MAX485** RS-485
transceiver requiring manual DE/RE direction control.

This is a separate crate from `firmware/esp32-rs` because the esp-rs
toolchain and esp-idf-template both assume one Cargo project per Xtensa
MCU target — the official path. `firmware/esp32-rs` builds for ESP32-S3;
this crate builds for ESP32. The two share no code (the firmware here
intentionally has no LCD/SPI/board-specific peripherals), so the
duplication is small.

Wire protocol with the dongle is identical to the LilyGO node in
`firmware/esp32-rs/src/bin/espnow_node.rs`, so the Mac side
(`EspNowDongleClient`) needs no changes.

## Hardware

- Standard ESP32-D0WD-V3 dev board
- External MAX485 transceiver: **DE/RE tied together on GPIO22**
- RS-485 on **UART2: TX=GPIO17, RX=GPIO16, 115200 8N1** (laiyu_xyz Modbus RTU)

## Build & flash

```bash
source ~/export-esp.sh
cd firmware/esp32-max485

cargo build --release
espflash flash --port /dev/cu.usbserial-XXXX \
    target/xtensa-esp32-espidf/release/espnow-node
```

First build pulls esp-idf v5.5.3 for the ESP32 target; allow 10–20 minutes.
Subsequent builds are cached under `target/`.

## What it does

- ESP-NOW broadcast on channel 1, identical to `firmware/esp32-rs`
- Filters inbound frames by `dst_mac` (first 6 bytes of payload)
- Bridges payload bytes to RS-485, toggling DE/RE around each write
  using `uart_wait_tx_done` so the last bit clears the shift register
  before dropping back to RX
- 1 Hz telemetry counter + uptime broadcast (`u32 LE counter | u32 LE uptime_ms`)
- Periodic REG announcement: `REG bus.laiyu_xyz.station1`
- Mother-side `BusConfig.match_hardware_id = bus.laiyu_xyz.station1`
  fans this single REG out to 4 logical devices (X/Y/Z stepper + YYQ
  pipette) — see `docs/recipes/configs/laiyu-xyz-station.yaml`.

## Layout

```
firmware/esp32-max485/
├── Cargo.toml
├── rust-toolchain.toml
├── .cargo/config.toml      # target = xtensa-esp32-espidf, MCU = esp32
├── sdkconfig.defaults
├── build.rs
└── src/bin/
    └── espnow_node.rs
```
