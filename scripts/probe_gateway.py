#!/usr/bin/env python3
"""Verify the Mac -> gateway -> ESP-NOW broadcast path WITHOUT touching any
real device.

Strategy:
  - Send a TX line addressed to a sink MAC that no child uses (default
    FFFFFFFFFFFE). Gateway still broadcasts the frame on ESP-NOW and logs
    `[tx->radio]`, but every child filters it out because dst_mac != my_mac.
  - Concurrently listen on the gateway's UART log to capture:
      * `[tx->radio] to=... len=...`  -> Mac -> gateway USB path works
      * any incoming `RX <mac> ...`    -> child -> gateway return path works

Usage:
    uv run --with pyserial python3 scripts/probe_gateway.py
    uv run --with pyserial python3 scripts/probe_gateway.py --dst FFFFFFFFFFFE --hex DEADBEEF
"""
import argparse, serial, sys, threading, time

GW_PORT_DEFAULT = "/dev/cu.usbserial-A5069RR4"
SINK_MAC = "FFFFFFFFFFFE"  # one bit off broadcast — no real child matches
DEFAULT_HEX = "DEADBEEF"   # arbitrary 4 bytes; harmless, no child will act

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default=GW_PORT_DEFAULT, help=f"gateway UART (default {GW_PORT_DEFAULT})")
    ap.add_argument("--dst",  default=SINK_MAC,        help=f"target MAC hex12 (default {SINK_MAC} — sink, no child matches)")
    ap.add_argument("--hex",  default=DEFAULT_HEX,     help=f"payload hex (default {DEFAULT_HEX})")
    ap.add_argument("--listen-sec", type=float, default=4.0)
    args = ap.parse_args()

    if len(args.dst) != 12:
        sys.exit("--dst must be 12 hex chars (no separators), e.g. FFFFFFFFFFFE")
    if len(args.hex) % 2:
        sys.exit("--hex must have an even number of nibbles")

    log_chunks = []
    stop = threading.Event()

    def listener():
        try:
            s = serial.Serial(args.port, 115200, timeout=0.1)
        except Exception as e:
            log_chunks.append(("OPEN_FAIL", str(e))); return
        while not stop.is_set():
            c = s.read(2048)
            if c:
                log_chunks.append((time.time(), c))
        s.close()

    t = threading.Thread(target=listener, daemon=True)
    t.start()
    time.sleep(1.5)  # capture some baseline log first

    # Open a *separate* handle to write the TX line. Listener stays open.
    gw = serial.Serial(args.port, 115200, timeout=0.1)
    line = f"TX {args.dst} {args.hex}\n".encode()
    gw.write(line); gw.flush()
    gw.close()
    print(f"[mac->gw] sent: {line!r}")
    print(f"  dst MAC  = {args.dst}  (sink: no child should match)")
    print(f"  payload  = {args.hex}  ({len(args.hex)//2} bytes)")

    time.sleep(args.listen_sec)
    stop.set(); t.join(timeout=1.0)

    if log_chunks and isinstance(log_chunks[0][0], str) and log_chunks[0][0] == "OPEN_FAIL":
        sys.exit(f"could not open gateway port {args.port}: {log_chunks[0][1]}")

    raw = b"".join(c for _, c in log_chunks if not isinstance(c, str))
    txt = raw.decode("utf-8", "replace")

    print("\n----- gateway log (filtered) -----")
    seen_tx_radio = False
    seen_parse_err = False
    rx_macs = set()
    for ln in txt.splitlines():
        if "tx->radio" in ln:
            print("  " + ln); seen_tx_radio = True
        elif "ER " in ln or "parse" in ln:
            print("  " + ln); seen_parse_err = True
        elif "RX " in ln:
            # collect distinct MACs that we hear from
            try:
                mac = ln.split("RX ", 1)[1].split()[0]
                if len(mac) == 12:
                    rx_macs.add(mac)
            except IndexError:
                pass

    print("\n----- summary -----")
    print(f"  Mac->gateway USB write   : {'OK' if seen_tx_radio else 'NOT SEEN'}  ({'tx->radio logged' if seen_tx_radio else 'no [tx->radio] line — gateway did not see our TX'})")
    print(f"  parse / ER lines         : {'YES (check above)' if seen_parse_err else 'none'}")
    if rx_macs:
        print(f"  child(ren) heard via RX  : {sorted(rx_macs)}")
    else:
        print("  child(ren) heard via RX  : none (no children broadcasting in this window)")

if __name__ == "__main__":
    main()
