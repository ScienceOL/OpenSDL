#!/usr/bin/env python3
"""Send /1ZR\r\n (Runze pump #1 init) through the gateway and watch replies.

Pipeline: Mac -> gateway UART0 -> ESP-NOW -> child -> RS-485 -> pump -> reply back.

Usage:
    uv run --with pyserial python3 scripts/send_1zr.py
    OSDL_GATEWAY_PORT=/dev/cu.usbserial-XXX \
    OSDL_CHILD_PORT=/dev/cu.usbmodem-XXX \
    OSDL_CHILD_MAC=30EDA0B65B38 \
        uv run --with pyserial python3 scripts/send_1zr.py

Find your ports:
    ls /dev/cu.usb*
    - gateway is the CH343 UART0 (115200 baud)  -> typically /dev/cu.usbserial-*
    - child   is the native ESP32-S3 USB-CDC    -> typically /dev/cu.usbmodem*
"""
import os, serial, threading, time

CHILD_MAC  = os.environ.get("OSDL_CHILD_MAC",  "30EDA0B65B38")
GW_PORT    = os.environ.get("OSDL_GATEWAY_PORT", "/dev/cu.usbserial-A5069RR4")
CHILD_PORT = os.environ.get("OSDL_CHILD_PORT",   "/dev/cu.usbmodem11301")
LISTEN_SEC = 6.0

results = {"gw": [], "child": []}


def listen(path, baud, buckets, duration):
    try:
        s = serial.Serial(path, baud, timeout=0.1)
    except Exception as e:
        buckets.append(("OPEN_FAIL", e))
        return
    t0 = time.time()
    while time.time() - t0 < duration:
        c = s.read(512)
        if c:
            buckets.append((time.time() - t0, c))
    s.close()


def collect_bytes(buckets):
    """Return (bytes, open_fail_error_or_None) from a listener bucket."""
    if buckets and buckets[0][0] == "OPEN_FAIL":
        return b"", buckets[0][1]
    return b"".join(c for _, c in buckets), None


print(f"gateway = {GW_PORT}")
print(f"child   = {CHILD_PORT}")
print(f"child MAC = {CHILD_MAC}")

ts = [
    threading.Thread(target=listen, args=(GW_PORT,    115200, results["gw"],    LISTEN_SEC)),
    threading.Thread(target=listen, args=(CHILD_PORT, 115200, results["child"], LISTEN_SEC)),
]
for t in ts:
    t.start()
time.sleep(1.0)  # let listeners settle

gw = serial.Serial(GW_PORT, 115200, timeout=0.1)
gw.reset_input_buffer()
gw.reset_output_buffer()
line = f"TX {CHILD_MAC} 2F315A520D0A\n".encode()
gw.write(line)
gw.flush()
print(f"\n[mac->gw] {line!r}   = /1ZR\\r\\n  (6B: 2F 31 5A 52 0D 0A)")
gw.close()

for t in ts:
    t.join()

print("\n====== GATEWAY log (tx->radio / ER / RX from child) ======")
gw_bytes, gw_err = collect_bytes(results["gw"])
if gw_err is not None:
    print(f"  [gateway port {GW_PORT!r} could not be opened: {gw_err}]")
else:
    gtxt = gw_bytes.decode("utf-8", "replace")
    seen = set()
    for ln in gtxt.splitlines():
        if any(k in ln for k in ("tx->radio", "ER", "parse")):
            print("  " + ln)
        elif f"RX {CHILD_MAC}" in ln:
            # CH343 on macOS sometimes replays the ring-buffer on reopen; dedupe identical lines.
            key = ln.split(": ", 1)[-1]
            if key not in seen:
                seen.add(key)
                print("  " + ln)

print("\n====== CHILD log (rx-for-me + any uart rx) ======")
child_bytes, child_err = collect_bytes(results["child"])
if child_err is not None:
    print(f"  [child port {CHILD_PORT!r} not available — skipping child log ({child_err})]")
else:
    ctxt = child_bytes.decode("utf-8", "replace")
    for ln in ctxt.splitlines():
        if any(k in ln for k in ("rx-for-me", "uart rx", "uart tx", "[rx", "short", "failed")):
            print("  " + ln)
