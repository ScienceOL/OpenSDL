#!/usr/bin/env python3
"""Send /1ZR\r\n (Runze pump #1 init) through the dongle and watch replies.

Pipeline: Mac -> dongle USB-CDC -> ESP-NOW -> node -> RS-485 -> pump -> reply back.

Usage:
    uv run --with pyserial python3 scripts/send_1zr.py
    OSDL_DONGLE_PORT=/dev/cu.usbmodem-XXX \
    OSDL_NODE_PORT=/dev/cu.usbmodem-XXX \
    OSDL_NODE_MAC=30EDA0B65B38 \
        uv run --with pyserial python3 scripts/send_1zr.py

Find your ports:
    ls /dev/cu.usb*
    - dongle is the ESP32-S3 native USB-CDC      -> /dev/cu.usbmodem* (115200)
    - node   is the second ESP32 native USB-CDC  -> /dev/cu.usbmodem*
"""
import os, serial, threading, time

NODE_MAC    = os.environ.get("OSDL_NODE_MAC",    "30EDA0B65B38")
DONGLE_PORT = os.environ.get("OSDL_DONGLE_PORT", "/dev/cu.usbmodem111301")
NODE_PORT   = os.environ.get("OSDL_NODE_PORT",   "/dev/cu.usbmodem11301")
LISTEN_SEC  = 6.0

results = {"dongle": [], "node": []}


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


print(f"dongle = {DONGLE_PORT}")
print(f"node   = {NODE_PORT}")
print(f"node MAC = {NODE_MAC}")

ts = [
    threading.Thread(target=listen, args=(DONGLE_PORT, 115200, results["dongle"], LISTEN_SEC)),
    threading.Thread(target=listen, args=(NODE_PORT,   115200, results["node"],   LISTEN_SEC)),
]
for t in ts:
    t.start()
time.sleep(1.0)  # let listeners settle

dongle = serial.Serial(DONGLE_PORT, 115200, timeout=0.1)
dongle.reset_input_buffer()
dongle.reset_output_buffer()
line = f"TX {NODE_MAC} 2F315A520D0A\n".encode()
dongle.write(line)
dongle.flush()
print(f"\n[mac->dongle] {line!r}   = /1ZR\\r\\n  (6B: 2F 31 5A 52 0D 0A)")
dongle.close()

for t in ts:
    t.join()

print("\n====== DONGLE log (tx->radio / ER / RX from node) ======")
dongle_bytes, dongle_err = collect_bytes(results["dongle"])
if dongle_err is not None:
    print(f"  [dongle port {DONGLE_PORT!r} could not be opened: {dongle_err}]")
else:
    gtxt = dongle_bytes.decode("utf-8", "replace")
    seen = set()
    for ln in gtxt.splitlines():
        if any(k in ln for k in ("tx->radio", "ER", "parse")):
            print("  " + ln)
        elif f"RX {NODE_MAC}" in ln:
            # macOS USB-CDC sometimes replays the ring-buffer on reopen; dedupe identical lines.
            key = ln.split(": ", 1)[-1]
            if key not in seen:
                seen.add(key)
                print("  " + ln)

print("\n====== NODE log (rx-for-me + any uart rx) ======")
node_bytes, node_err = collect_bytes(results["node"])
if node_err is not None:
    print(f"  [node port {NODE_PORT!r} not available — skipping node log ({node_err})]")
else:
    ctxt = node_bytes.decode("utf-8", "replace")
    for ln in ctxt.splitlines():
        if any(k in ln for k in ("rx-for-me", "uart rx", "uart tx", "[rx", "short", "failed")):
            print("  " + ln)
