#!/usr/bin/env python3
"""Direct RS-485 probe via the USB-RS485 dongle on /dev/cu.usbserial-130.
Bypasses the ESP-NOW path entirely — talks straight to the bus.

Use this to isolate "is it the device, or the node firmware?" failures.

⚠️ IMPORTANT: stop `osdl serve` first. The ESP-NOW node also writes to the
same RS-485 bus on incoming commands; two masters at once will collide.

Default action: scan slave addresses 1..16 with Modbus 03 (read holding
registers, 6 regs from addr 0). Any response with the matching slave_id
in byte 0 means a device is alive at that address.

Usage:
    uv run --with pyserial python3 scripts/rs485_direct_probe.py             # scan 1..16
    uv run --with pyserial python3 scripts/rs485_direct_probe.py --slave 1   # single slave
    uv run --with pyserial python3 scripts/rs485_direct_probe.py --raw 2F345145F9  # send arbitrary hex
    uv run --with pyserial python3 scripts/rs485_direct_probe.py --baud 9600 ...   # other baud
"""
import argparse, serial, sys, time

PORT_DEFAULT = "/dev/cu.usbserial-130"
BAUD_DEFAULT = 115200

def crc16(data: bytes) -> bytes:
    """Modbus RTU CRC-16 (poly 0xA001, init 0xFFFF), little-endian return."""
    crc = 0xFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            if crc & 1:
                crc = (crc >> 1) ^ 0xA001
            else:
                crc >>= 1
    return bytes([crc & 0xFF, (crc >> 8) & 0xFF])

def build_read_holding(slave: int, start: int, count: int) -> bytes:
    frame = bytes([slave, 0x03, (start >> 8) & 0xFF, start & 0xFF,
                   (count >> 8) & 0xFF, count & 0xFF])
    return frame + crc16(frame)

def fmt(b: bytes) -> str:
    return " ".join(f"{x:02X}" for x in b)

def send_and_listen(s: serial.Serial, frame: bytes, listen_ms: int = 300) -> bytes:
    s.reset_input_buffer()
    s.write(frame); s.flush()
    deadline = time.time() + listen_ms / 1000
    buf = b""
    while time.time() < deadline:
        c = s.read(256)
        if c:
            buf += c
            # extend deadline by 50ms after we see something — small grace for
            # late bytes after first byte arrives
            deadline = max(deadline, time.time() + 0.050)
    return buf

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default=PORT_DEFAULT)
    ap.add_argument("--baud", type=int, default=BAUD_DEFAULT)
    ap.add_argument("--slave", type=int, help="probe just this slave id (1..247)")
    ap.add_argument("--start", type=int, default=0, help="modbus start register (default 0)")
    ap.add_argument("--count", type=int, default=6, help="modbus register count (default 6)")
    ap.add_argument("--scan-from", type=int, default=1)
    ap.add_argument("--scan-to", type=int, default=16)
    ap.add_argument("--raw", help="send arbitrary hex frame; CRC NOT auto-appended")
    ap.add_argument("--listen-ms", type=int, default=300)
    args = ap.parse_args()

    print(f"port = {args.port} @ {args.baud} 8N1")
    try:
        s = serial.Serial(args.port, args.baud, timeout=0.05)
    except Exception as e:
        sys.exit(f"open {args.port}: {e}")

    # Most USB-RS485 dongles auto-toggle DE/RE; if yours doesn't, this won't
    # work and you'd see TX bytes echo back in our own RX buffer.
    time.sleep(0.1)
    s.reset_input_buffer()

    if args.raw:
        h = args.raw.replace(" ", "").replace(":", "")
        frame = bytes.fromhex(h)
        print(f"TX raw: {fmt(frame)}  ({len(frame)} bytes)")
        rx = send_and_listen(s, frame, args.listen_ms)
        print(f"RX    : {fmt(rx) if rx else '(no reply)'}")
        s.close(); return

    if args.slave is not None:
        slaves = [args.slave]
    else:
        slaves = list(range(args.scan_from, args.scan_to + 1))

    print(f"scan slaves {slaves[0]}..{slaves[-1]}, action=read_holding({args.start}, {args.count})")
    print(f"{'slave':>5}  {'TX':<24}  RX")
    print("-" * 70)
    found = []
    for slave in slaves:
        frame = build_read_holding(slave, args.start, args.count)
        rx = send_and_listen(s, frame, args.listen_ms)
        marker = ""
        if rx:
            if rx[:1] == bytes([slave]):
                marker = "  <-- match"
                found.append(slave)
            elif rx == frame:
                # our own TX echoed back — dongle without proper auto-direction
                marker = "  (own echo only)"
            else:
                marker = "  (unexpected reply)"
        print(f"{slave:>5}  {fmt(frame):<24}  {fmt(rx) if rx else '(no reply)'}{marker}")
        time.sleep(0.05)

    print()
    if found:
        print(f"Slaves that responded: {found}")
    else:
        print("No slaves responded.")
        print("Things to check:")
        print("  - Devices powered on?")
        print("  - RS-485 A/B not swapped?")
        print("  - Dongle GND tied to the bus signal ground (not just USB shield)?")
        print("  - Baud / parity / stop bits match the device (try --baud 9600)?")
        print("  - osdl serve stopped? (if not, node contends as a second master)")
    s.close()

if __name__ == "__main__":
    main()
