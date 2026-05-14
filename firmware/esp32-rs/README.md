# osdl-esp32-firmware

Rust firmware for the ESP32-S3 side of the OpenSDL lab-automation bridge.

```
┌─── Mac (OpenSDL host) ─────────────────────┐
│                                              │
│   writes `TX <mac> <hex>\n` / reads `RX ...` │
│   over USB-CDC (CH343, 115200)               │
└─────────┼────────────────────────────────────┘
          │ USB-C
          ▼
   Gateway  (YD-ESP32-S3)
          │
          │ ESP-NOW, channel 1, broadcast
          ▼
   Child   (LilyGO T-Connect Pro)
          │ UART1 @ GPIO17/18 → TD501D485H-A transceiver
          ▼
   RS-485 bus → lab device (Runze syringe pump, etc.)
```

The **gateway** speaks a simple ASCII line protocol over USB to the host;
the **child** filters ESP-NOW frames by its own MAC and bridges them to
RS-485. UART replies from the device go back up the same path.

## Hardware

| Board                 | Role    | USB path (macOS, typical)       | Chip              |
|-----------------------|---------|---------------------------------|-------------------|
| YD-ESP32-S3 (N16R8)   | Gateway | `/dev/cu.usbserial-*` (CH343)   | ESP32-S3, 16 MB   |
| LilyGO T-Connect Pro  | Child   | `/dev/cu.usbmodem*` (native USB)| ESP32-S3, 16 MB   |

Confirm the paths on your machine with `ls /dev/cu.usb*`.

YD-ESP32-S3 note: use the **USB** port (native-USB) for flashing. The **COM**
port (CH343) is where serial logs come out, and is also where the gateway
accepts host commands on UART0.

## Wire protocol (host ↔ gateway, over USB-CDC)

Host → gateway:

```
TX <dst_mac_hex> <hex_bytes>\n
```

Gateway → host:

```
RX <src_mac_hex> <hex_bytes>\n
ER <reason>\n
```

Example — send `/1ZR\r\n` (Runze init) to the child at MAC `30EDA0B65B38`:

```
TX 30EDA0B65B38 2F315A520D0A
```

The child strips the MAC prefix, writes the remaining 6 bytes to UART1,
and whatever comes back on the RS-485 bus is broadcast back as an
`RX …` line.

## Bins

| `cargo run --bin …`        | Purpose                                                          |
|----------------------------|-------------------------------------------------------------------|
| `espnow-gateway`           | Runs on YD-ESP32-S3. Bridges Mac USB-CDC ↔ ESP-NOW.              |
| `espnow-child`             | Runs on LilyGO T-Connect Pro. Bridges ESP-NOW ↔ RS-485 (UART1).  |
| `uart-count`               | Minimal UART1 TX sanity check. Emits an incrementing integer.    |
| `espnow-mac` / `espnow-diag` | Early ESP-NOW probes kept for reference.                       |

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
`~/export-esp.sh`. From `espup install --help`:

> `-f, --export-file <EXPORT_FILE>`  Relative or full path for the export
> file that will be generated. **If no path is provided, the file will be
> generated under home directory.**

### 4. Source the export script in every shell that builds firmware

```bash
source ~/export-esp.sh
```

The script sets `PATH` and `LIBCLANG_PATH` so cargo can find the Xtensa
toolchain and bindgen can find libclang. **Forgetting this step is the #1
cause of confusing build errors** (missing target, missing libclang, etc.).

You can put `source ~/export-esp.sh` in your `~/.zshrc` / `~/.bashrc` for
convenience, but it affects every shell — fine in a dedicated firmware-dev
box, less fine if you use the same terminal for other Rust projects.

### 5. Verify the install

```bash
# All three should resolve to ~/.cargo/bin/
which espup espflash ldproxy

# Must exist and look like below — points at the toolchain you just installed:
cat ~/export-esp.sh
# export LIBCLANG_PATH="$HOME/.rustup/toolchains/esp/xtensa-esp32-elf-clang/.../esp-clang/lib"
# export PATH="$HOME/.rustup/toolchains/esp/xtensa-esp-elf/.../xtensa-esp-elf/bin:$PATH"

# After `source ~/export-esp.sh`, this should print the Xtensa path:
echo "$LIBCLANG_PATH"
```

If `~/export-esp.sh` doesn't exist, `espup install` was interrupted — rerun
it, or write a custom path with `espup install -f /some/path/export-esp.sh`.

### 6. Copy the config template and fill in real values

```bash
cp src/config.example.rs src/config.rs
# then edit src/config.rs → WIFI_SSID, WIFI_PASSWORD, etc.
```

`src/config.rs` is `.gitignore`d so credentials never land in a commit. The
ESP-NOW bins (`espnow-gateway` / `espnow-child`) don't actually use WiFi,
but the file still has to exist because `src/main.rs` references it.

## Build & flash

```bash
source ~/export-esp.sh

# Gateway (YD-ESP32-S3)
cargo build --release --bin espnow-gateway
espflash flash --port /dev/cu.usbserial-XXXX --flash-size 16mb \
    target/xtensa-esp32s3-espidf/release/espnow-gateway

# Child (LilyGO T-Connect Pro)
cargo build --release --bin espnow-child
espflash flash --port /dev/cu.usbmodem-XXXX --flash-size 16mb \
    target/xtensa-esp32s3-espidf/release/espnow-child

# Monitor logs (read-only)
espflash monitor --port /dev/cu.usbmodem-XXXX --non-interactive
```

`--flash-size 16mb` is required for the YD board; auto-detect sometimes
reports 4 MB incorrectly.

## Quick test — is the RS-485 link alive?

Plug both boards in, then from the host:

```bash
OSDL_GATEWAY_PORT=/dev/cu.usbserial-XXXX \
OSDL_CHILD_PORT=/dev/cu.usbmodem-XXXX   \
OSDL_CHILD_MAC=30EDA0B65B38             \
    uv run --with pyserial python3 scripts/send_1zr.py
```

This sends `/1ZR\r\n` (Runze pump-1 init) through the full path and prints
what each hop saw. A healthy run looks like:

```
[mac->gw] b'TX 30EDA0B65B38 2F315A520D0A\n'

====== GATEWAY log (tx->radio / ER / RX from child) ======
  I ... espnow_gateway: [tx->radio] to=30EDA0B65B38 len=6
  I ... espnow_gateway: RX 30EDA0B65B38 FF2F3040030D0A

====== CHILD log (rx-for-me + any uart rx) ======
  I ... espnow_child: [rx-for-me] 6 bytes -> UART: [2F, 31, 5A, 52, 0D, 0A]
  I ... espnow_child: [uart rx -> radio] 7 bytes: [FF, 2F, 30, 40, 03, 0D, 0A]
```

The `FF 2F 30 40 03 0D 0A` in both the gateway RX line and the child
`uart rx -> radio` log is the pump's reply.

## Common errors

| Error                                            | Cause                                         | Fix                                                  |
|--------------------------------------------------|-----------------------------------------------|------------------------------------------------------|
| `can't find crate for 'std'`                     | Forgot to `source ~/export-esp.sh`            | Source it in this shell                              |
| `couldn't find libclang`                         | Same — `LIBCLANG_PATH` unset                  | Source it in this shell                              |
| `toolchain 'esp' is not installed`               | `espup install` not completed                 | Rerun `espup install`                                |
| `~/export-esp.sh: No such file`                  | `espup install` was interrupted               | Rerun, or `espup install -f <path>` to choose a path |
| `espflash: port is busy`                         | Another `espflash monitor` is still attached  | `pkill espflash` or close the other terminal         |
| Board boot-loops after flashing                  | Wrong flash size                              | Always pass `--flash-size 16mb`                      |
| `cargo check` fails with main-workspace errors   | CWD isn't `firmware/esp32-rs`                 | `cd` into this directory first                       |
| First build sits in `esp-idf-sys` for minutes    | Normal — embuild is fetching ESP-IDF v5.5.3   | Wait 3–5 minutes; subsequent builds are cached       |

## Debugging tips

- `espflash monitor` holds the serial port exclusively. Kill it before
  running `send_1zr.py` or any tool that writes to the same tty.
- Gateway log output goes out of UART0 (CH343 / the "COM" USB-C port),
  **not** the native-USB port. `cat /dev/cu.usbserial-*` on macOS can
  behave poorly with CH343 — prefer `espflash monitor --non-interactive`.
- If the child writes the command to UART1 (`[rx-for-me]` fires) but no
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
        ├── espnow_gateway.rs # YD gateway firmware
        ├── espnow_child.rs   # LilyGO child firmware
        ├── uart_count.rs     # UART1 TX sanity check
        ├── espnow_mac.rs     # early MAC print helper
        └── espnow_diag.rs    # early ESP-NOW receive diag
```

This crate builds independently of the OpenSDL Rust workspace — the Xtensa
target and ESP-IDF toolchain would confuse the host-side `cargo check`.
Always run `source ~/export-esp.sh` in the shell first, then `cargo build`
from this directory.
