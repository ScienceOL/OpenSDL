# osdl-esp32-firmware

Rust firmware for the ESP32-S3 boards used in OpenSDL's two roles:

- **Dongle** — plugs into the host (Mac/PC) over USB-CDC. Bridges the
  host's serial line protocol to ESP-NOW broadcasts.
- **Node** — plugs into a lab device's RS-485 / serial bus. Filters
  ESP-NOW frames by its own MAC, forwards payloads to the device, and
  ships replies back over ESP-NOW.

```
┌─── Mac / PC (OpenSDL host) ────────────────┐
│                                              │
│   writes `TX <mac> <hex>\n` / reads `RX ...` │
│   over USB-CDC (115200, native USB)          │
└─────────┼────────────────────────────────────┘
          │ USB-C
          ▼
   Dongle  (ESP32-S3, e.g. Pocket-Dongle-S3)
          │
          │ ESP-NOW, channel 1, broadcast
          ▼
   Node    (LilyGO T-Connect Pro or ESP32 + MAX485)
          │ UART → RS-485 transceiver
          ▼
   RS-485 bus → lab device (Runze pump, stepper motor, …)
```

The wire protocol between dongle and node is identical regardless of
hardware variant; the host-side `EspNowDongleClient` doesn't care which
board it talks to.

## Hardware

| Role   | Reference board                | Host USB path (macOS, typical) | Chip            |
|--------|--------------------------------|--------------------------------|-----------------|
| Dongle | Pocket-Dongle-S3 (with screen) | `/dev/cu.usbmodem*` (native)   | ESP32-S3, 16 MB |
| Node   | LilyGO T-Connect Pro           | `/dev/cu.usbmodem*` (native)   | ESP32-S3, 16 MB |
| Node   | ESP32 + external MAX485        | (sibling crate `firmware/esp32-max485`) | ESP32, 4 MB |

Confirm the paths on your machine with `ls /dev/cu.usb*`.

The dongle uses the ESP32-S3's **built-in USB-Serial-JTAG**, so it
enumerates as `/dev/cu.usbmodem*` rather than `/dev/cu.usbserial-*`.
There's no external FTDI/CH340 chip in the path.

## Wire protocol (host ↔ dongle, over USB-CDC)

Host → dongle:

```
TX <dst_mac_hex> <hex_bytes>\n
```

Dongle → host:

```
RX <src_mac_hex> <hex_bytes>\n
ER <reason>\n
```

Example — send `/1ZR\r\n` (Runze init) to a node at MAC `30EDA0B65B38`:

```
TX 30EDA0B65B38 2F315A520D0A
```

The node strips the MAC prefix, writes the remaining 6 bytes to its
RS-485 UART, and whatever comes back on the bus is broadcast back as an
`RX …` line.

## Bins

| `cargo run --bin …`        | Purpose                                                                |
|----------------------------|------------------------------------------------------------------------|
| `espnow-dongle`            | Runs on the dongle. Bridges Mac USB-CDC ↔ ESP-NOW.                     |
| `espnow-node`              | Runs on the node. Bridges ESP-NOW ↔ RS-485 (UART1).                    |
| `uart-count`               | Minimal UART1 TX sanity check. Emits an incrementing integer.          |
| `espnow-mac` / `espnow-diag` | Early ESP-NOW probes kept for reference.                             |

## Prerequisites

ESP32 is an Xtensa target, which isn't part of upstream LLVM, so the usual
`rustup target add` route doesn't work. You need Espressif's custom toolchain.

### 1. Install Rust itself (if you don't have it)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### 2. Install the Espressif helper tools

```bash
cargo install espup espflash ldproxy
```

- **`espup`** — installs and manages the Xtensa Rust toolchain
- **`espflash`** — flashes ELFs to the board (replaces `esptool.py`)
- **`ldproxy`** — linker shim so cargo can invoke ESP-IDF's linker

### 3. Install the Xtensa Rust toolchain

```bash
espup install
```

This pulls ~500 MB of Xtensa LLVM + a custom Rust channel named `esp`, and
writes an **export script** to your home directory — by default
`~/export-esp.sh`.

### 4. Source the export script in every shell that builds firmware

```bash
source ~/export-esp.sh
```

The script sets `PATH` and `LIBCLANG_PATH` so cargo can find the Xtensa
toolchain and bindgen can find libclang. **Forgetting this step is the #1
cause of confusing build errors** (missing target, missing libclang, etc.).

### 5. Copy the config template and fill in real values

```bash
cp src/config.example.rs src/config.rs
# then edit src/config.rs → WIFI_SSID, WIFI_PASSWORD, etc.
```

`src/config.rs` is `.gitignore`d so credentials never land in a commit. The
ESP-NOW bins (`espnow-dongle` / `espnow-node`) don't actually use WiFi,
but the file still has to exist because `src/main.rs` references it.

## Build & flash

```bash
source ~/export-esp.sh

# Dongle
cargo build --release --bin espnow-dongle
espflash flash --port /dev/cu.usbmodem-XXXX \
    target/xtensa-esp32s3-espidf/release/espnow-dongle

# Node (LilyGO T-Connect Pro)
cargo build --release --bin espnow-node
espflash flash --port /dev/cu.usbmodem-XXXX \
    target/xtensa-esp32s3-espidf/release/espnow-node

# Monitor logs (read-only)
espflash monitor --port /dev/cu.usbmodem-XXXX --non-interactive
```

`espflash` auto-detects flash size on these boards. If it gets it wrong,
pass `--flash-size 16mb` (or 4mb for plain ESP32 nodes via the sibling
crate).

## Quick test — is the RS-485 link alive?

Plug both boards in, then from the host:

```bash
OSDL_DONGLE_PORT=/dev/cu.usbmodem-XXXX \
OSDL_NODE_PORT=/dev/cu.usbmodem-XXXX   \
OSDL_NODE_MAC=30EDA0B65B38             \
    uv run --with pyserial python3 scripts/send_1zr.py
```

This sends `/1ZR\r\n` (Runze pump-1 init) through the full path and prints
what each hop saw. A healthy run looks like:

```
[mac->dongle] b'TX 30EDA0B65B38 2F315A520D0A\n'

====== DONGLE log (tx->radio / ER / RX from node) ======
  I ... espnow_dongle: [tx->radio] to=30EDA0B65B38 len=6
  I ... espnow_dongle: RX 30EDA0B65B38 FF2F3040030D0A

====== NODE log (rx-for-me + any uart rx) ======
  I ... espnow_node: [rx-for-me] 6 bytes -> UART: [2F, 31, 5A, 52, 0D, 0A]
  I ... espnow_node: [uart rx -> radio] 7 bytes: [FF, 2F, 30, 40, 03, 0D, 0A]
```

The `FF 2F 30 40 03 0D 0A` in both the dongle RX line and the node
`uart rx -> radio` log is the pump's reply.

## Common errors

| Error                                            | Cause                                         | Fix                                                  |
|--------------------------------------------------|-----------------------------------------------|------------------------------------------------------|
| `can't find crate for 'std'`                     | Forgot to `source ~/export-esp.sh`            | Source it in this shell                              |
| `couldn't find libclang`                         | Same — `LIBCLANG_PATH` unset                  | Source it in this shell                              |
| `toolchain 'esp' is not installed`               | `espup install` not completed                 | Rerun `espup install`                                |
| `~/export-esp.sh: No such file`                  | `espup install` was interrupted               | Rerun, or `espup install -f <path>` to choose a path |
| `espflash: port is busy`                         | Another `espflash monitor` is still attached  | `pkill espflash` or close the other terminal         |
| `cargo check` fails with main-workspace errors   | CWD isn't `firmware/esp32-rs`                 | `cd` into this directory first                       |
| First build sits in `esp-idf-sys` for minutes    | Normal — embuild is fetching ESP-IDF v5.5.3   | Wait 3–5 minutes; subsequent builds are cached       |

## Debugging tips

- `espflash monitor` holds the serial port exclusively. Kill it before
  running `send_1zr.py` or any tool that writes to the same tty.
- The dongle's logs and the host-side line protocol share the same
  USB-CDC. `log::info!` lines (boot info, `[tx->radio]`, `RX ...`) and
  the host's `TX ...\n` writes go through the same pipe — the line
  parser on the host matches `RX ` anywhere in the line so log noise
  doesn't break it.
- If the node writes the command to UART (`[rx-for-me]` fires) but no
  `uart rx -> radio` comes back, look at the RS-485 wiring first:
  A/B polarity is commonly swapped, and the isolated-side ground (SGND)
  must be connected to the bus signal ground — NOT the MCU DGND.
- ESP-NOW is pinned to channel 1 on both sides (`esp_wifi_set_channel(1)`).
  Both boards must be on the same channel or frames drop silently.

## Layout

```
firmware/esp32-rs/
├── Cargo.toml                # standalone crate (not part of OpenSDL workspace)
├── rust-toolchain.toml       # pins the `esp` Xtensa toolchain
├── sdkconfig.defaults        # ESP-IDF config knobs
├── build.rs                  # embuild glue
├── scripts/
│   └── send_1zr.py           # Python probe for the end-to-end path
└── src/
    ├── main.rs
    ├── config.example.rs     # copy to config.rs and fill in
    └── bin/
        ├── espnow_dongle.rs  # Dongle firmware (Mac USB-CDC ↔ ESP-NOW)
        ├── espnow_node.rs    # Node firmware (ESP-NOW ↔ RS-485)
        ├── uart_count.rs     # UART1 TX sanity check
        ├── espnow_mac.rs     # early MAC print helper
        └── espnow_diag.rs    # early ESP-NOW receive diag
```

This crate builds independently of the OpenSDL Rust workspace — the Xtensa
target and ESP-IDF toolchain would confuse the host-side `cargo check`.
Always run `source ~/export-esp.sh` in the shell first, then `cargo build`
from this directory.
