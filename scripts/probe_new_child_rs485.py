#!/usr/bin/env python3
"""Send a test frame through the new child's USB-CDC; the on-board passthrough
firmware (~/Downloads/Child/main.cpp) forwards it onto RS485, where the
USB-RS485 sniffer (or a browser WebSerial terminal) on /dev/cu.usbserial-130
should see the same bytes come out at 9600 8N1.

Usage:
    uv run --with pyserial python3 scripts/probe_new_child_rs485.py
    uv run --with pyserial python3 scripts/probe_new_child_rs485.py --hex 010300000001840A
    uv run --with pyserial python3 scripts/probe_new_child_rs485.py --text "/1ZR\\r\\n"
"""
import argparse, serial, sys, time

CHILD_USB = "/dev/cu.usbserial-4"  # child's USB-CDC (115200 8N1)
USB_BAUD = 115200

def to_bytes(args) -> bytes:
    if args.hex:
        h = args.hex.replace(" ", "").replace(":", "")
        if len(h) % 2:
            sys.exit("--hex must have even number of nibbles")
        return bytes.fromhex(h)
    if args.text is not None:
        # interpret python escapes (\r \n \xNN ...)
        return args.text.encode("utf-8").decode("unicode_escape").encode("latin-1")
    # default: short ASCII probe
    return b"PING-PROBE\r\n"

def main():
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--hex",  help="payload as hex, e.g. 2F315A520D0A")
    g.add_argument("--text", help="payload as text (supports \\r \\n \\xNN)")
    ap.add_argument("--port", default=CHILD_USB, help=f"child USB port (default {CHILD_USB})")
    args = ap.parse_args()

    payload = to_bytes(args)
    print(f"port    = {args.port} @ {USB_BAUD} 8N1")
    print(f"payload = {payload!r} ({len(payload)} bytes, hex: {payload.hex().upper()})")

    s = serial.Serial(args.port, USB_BAUD, timeout=0.2)
    time.sleep(0.2)
    s.reset_input_buffer()
    s.write(payload)
    s.flush()
    print("[mac->child USB] wrote, waiting 1s for any echo back over USB...")
    t0 = time.time()
    buf = b""
    while time.time() - t0 < 1.0:
        c = s.read(256)
        if c: buf += c
    s.close()
    if buf:
        print(f"[child USB -> mac] {buf!r} (hex: {buf.hex().upper()})")
        print("  ^ this is what came back on the USB side (RS485 -> child -> USB).")
        print("    On a quiet bus you'd typically see nothing here unless a device replies.")
    else:
        print("[child USB -> mac] (no echo)")
    print("\nNow check the web serial tool on /dev/cu.usbserial-130 @ 9600 8N1 — you should")
    print("see the payload bytes appear there. If you do: TX path through the child works.")

if __name__ == "__main__":
    main()
