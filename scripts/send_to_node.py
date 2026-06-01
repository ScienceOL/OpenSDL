#!/usr/bin/env python3
"""Send a payload through the dongle to the laiyu MAX485 node (MAC F4:65:0B:47:B8:88).
The node's ESP-NOW handler writes those bytes onto its RS-485 bus, where the
USB-RS485 sniffer (or a browser WebSerial terminal) on /dev/cu.usbserial-130
@ 115200 8N1 should see them come out.

Path:  Mac -> /dev/cu.usbmodem* (dongle) -> ESP-NOW -> node
       -> UART2 (GPIO17 TX, MAX485 DE/RE on GPIO22) -> RS-485 -> sniffer

Usage:
    uv run --with pyserial python3 scripts/send_to_node.py
    uv run --with pyserial python3 scripts/send_to_node.py --hex DEADBEEF
    uv run --with pyserial python3 scripts/send_to_node.py --text "hello\\r\\n"

The default payload is `PROBE-NODE\\r\\n` — innocuous ASCII, not any
real RS-485 device protocol, so connected hardware (if any) will ignore it.
"""
import argparse, serial, sys, threading, time

DONGLE_PORT_DEFAULT = "/dev/cu.usbmodem111301"
NODE_MAC_DEFAULT    = "F4650B47B888"
DEFAULT_TEXT        = "PROBE-NODE\r\n"

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
    ap.add_argument("--port", default=DONGLE_PORT_DEFAULT, help=f"dongle UART (default {DONGLE_PORT_DEFAULT})")
    ap.add_argument("--mac",  default=NODE_MAC_DEFAULT,    help=f"target node MAC hex12 (default {NODE_MAC_DEFAULT})")
    ap.add_argument("--listen-sec", type=float, default=4.0, help="how long to listen for replies on the dongle")
    args = ap.parse_args()

    payload = to_bytes(args)
    if not payload:
        sys.exit("payload is empty")

    print(f"dongle = {args.port}")
    print(f"target  = {args.mac}  (node)")
    print(f"payload = {payload!r} ({len(payload)} bytes, hex: {payload.hex().upper()})")
    print()
    print("Make sure the browser serial tool is open on /dev/cu.usbserial-130 @ 115200 8N1.")
    print("Listening on dongle for [tx->radio] and any RX reply...")
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

    dongle = serial.Serial(args.port, 115200, timeout=0.1)
    line = f"TX {args.mac} {payload.hex().upper()}\n".encode()
    dongle.write(line); dongle.flush()
    dongle.close()
    print(f"[mac->dongle] sent: {line!r}")

    time.sleep(args.listen_sec)
    stop.set(); t.join(timeout=1.0)

    if log and isinstance(log[0][0], str) and log[0][0] == "OPEN_FAIL":
        sys.exit(f"could not open dongle port {args.port}: {log[0][1]}")
    txt = b"".join(c for _, c in log if not isinstance(c, str)).decode("utf-8", "replace")

    print("\n----- relevant dongle log -----")
    seen_tx = False
    for ln in txt.splitlines():
        if "tx->radio" in ln and args.mac in ln:
            print("  " + ln); seen_tx = True
        elif f"RX {args.mac}" in ln:
            print("  " + ln)
        elif "ER " in ln or "parse" in ln:
            print("  " + ln)

    print("\n----- check -----")
    print(f"  Mac->dongle TX accepted   : {'OK' if seen_tx else 'NOT SEEN — dongle did not log tx->radio'}")
    print(f"  Browser on /dev/cu.usbserial-130 @ 115200 8N1 should now show: {payload!r}")
    print(f"  (If a real device is wired up, look for any reply line above too.)")

if __name__ == "__main__":
    main()
