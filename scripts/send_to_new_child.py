#!/usr/bin/env python3
"""Send a payload through the gateway to the new child (MAC F4:65:0B:47:B8:88).
The child's ESP-NOW handler writes those bytes onto its RS-485 bus, where the
USB-RS485 sniffer (or a browser WebSerial terminal) on /dev/cu.usbserial-130
@ 115200 8N1 should see them come out.

Path:  Mac -> /dev/cu.usbserial-A5069RR4 (gateway) -> ESP-NOW -> new child
       -> UART2 (GPIO17 TX, MAX485 DE/RE on GPIO22) -> RS-485 -> sniffer

Usage:
    uv run --with pyserial python3 scripts/send_to_new_child.py
    uv run --with pyserial python3 scripts/send_to_new_child.py --hex DEADBEEF
    uv run --with pyserial python3 scripts/send_to_new_child.py --text "hello\\r\\n"

The default payload is `PROBE-NEW-CHILD\\r\\n` — innocuous ASCII, not any
real RS-485 device protocol, so connected hardware (if any) will ignore it.
"""
import argparse, serial, sys, threading, time

GW_PORT_DEFAULT  = "/dev/cu.usbserial-A5069RR4"
NEW_CHILD_MAC    = "F4650B47B888"
DEFAULT_TEXT     = "PROBE-NEW-CHILD\r\n"

def to_bytes(args) -> bytes:
    if args.hex:
        h = args.hex.replace(" ", "").replace(":", "")
        if len(h) % 2:
            sys.exit("--hex must have even number of nibbles")
        return bytes.fromhex(h)
    if args.text is not None:
        return args.text.encode("utf-8").decode("unicode_escape").encode("latin-1")
    return DEFAULT_TEXT.encode()

def main():
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--hex",  help="payload as hex, e.g. DEADBEEF")
    g.add_argument("--text", help="payload as text (supports \\r \\n \\xNN)")
    ap.add_argument("--port", default=GW_PORT_DEFAULT, help=f"gateway UART (default {GW_PORT_DEFAULT})")
    ap.add_argument("--mac",  default=NEW_CHILD_MAC,    help=f"target child MAC hex12 (default {NEW_CHILD_MAC})")
    ap.add_argument("--listen-sec", type=float, default=4.0, help="how long to listen for replies on the gateway")
    args = ap.parse_args()

    payload = to_bytes(args)
    if not payload:
        sys.exit("payload is empty")

    print(f"gateway = {args.port}")
    print(f"target  = {args.mac}  (new child)")
    print(f"payload = {payload!r} ({len(payload)} bytes, hex: {payload.hex().upper()})")
    print()
    print("Make sure the browser serial tool is open on /dev/cu.usbserial-130 @ 115200 8N1.")
    print("Listening on gateway for [tx->radio] and any RX reply...")
    print()

    log = []
    stop = threading.Event()
    def listener():
        try:
            s = serial.Serial(args.port, 115200, timeout=0.1)
        except Exception as e:
            log.append(("OPEN_FAIL", str(e))); return
        while not stop.is_set():
            c = s.read(2048)
            if c:
                log.append((time.time(), c))
        s.close()
    t = threading.Thread(target=listener, daemon=True)
    t.start()
    time.sleep(1.0)

    gw = serial.Serial(args.port, 115200, timeout=0.1)
    line = f"TX {args.mac} {payload.hex().upper()}\n".encode()
    gw.write(line); gw.flush()
    gw.close()
    print(f"[mac->gw] sent: {line!r}")

    time.sleep(args.listen_sec)
    stop.set(); t.join(timeout=1.0)

    if log and isinstance(log[0][0], str) and log[0][0] == "OPEN_FAIL":
        sys.exit(f"could not open gateway port {args.port}: {log[0][1]}")
    txt = b"".join(c for _, c in log if not isinstance(c, str)).decode("utf-8", "replace")

    print("\n----- relevant gateway log -----")
    seen_tx = False
    for ln in txt.splitlines():
        if "tx->radio" in ln and args.mac in ln:
            print("  " + ln); seen_tx = True
        elif f"RX {args.mac}" in ln:
            print("  " + ln)
        elif "ER " in ln or "parse" in ln:
            print("  " + ln)

    print("\n----- check -----")
    print(f"  Mac->gateway TX accepted   : {'OK' if seen_tx else 'NOT SEEN — gateway did not log tx->radio'}")
    print(f"  Browser on /dev/cu.usbserial-130 @ 115200 8N1 should now show: {payload!r}")
    print(f"  (If a real device is wired up, look for any reply line above too.)")

if __name__ == "__main__":
    main()
