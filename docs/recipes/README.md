# OpenSDL Recipes

Each recipe shows how to drive a real piece of lab hardware end-to-end via
the `osdl` CLI. They replace the per-scenario Rust example binaries that
used to live under `crates/{osdl-cli,osdl-core}/examples/` — same physical
behavior, but reachable from any client (shell, agent, desktop app) over
the gRPC API.

## Convention

Every recipe assumes:

- A built `osdl` binary on `$PATH` (or used as `./target/debug/osdl`).
- The workspace's `registry/unilabos` directory is reachable. We pass
  `--registry $(pwd)/registry/unilabos` from the workspace root in every
  example for clarity.
- For ESP-NOW recipes, the dongle board is plugged into a serial port
  (`/dev/cu.usbmodem*` on macOS for the Pocket-Dongle-S3 native USB).
  Override with `--dongle-port` or `OSDL_DONGLE_PORT`.

Each recipe runs the server in `--detach` mode under a per-scenario
`--instance` name so they can coexist on the same host. The lockfile
machinery in `osdl-server` makes `osdl --instance NAME …` route to the
right one.

## Index

| Recipe | What it does | Hardware needed |
|---|---|---|
| [`runze-single-pump.md`](runze-single-pump.md) | One Runze syringe pump end-to-end (init, query, set valve) | ESP-NOW dongle + 1 Runze pump (legacy 1:1 path) |
| [`chinwe-station.md`](chinwe-station.md) | The full 5-device ChinWe bus: 3 pumps + stirrer + drain valve, all addressable from `pump-1` … `motor-5` | Full ChinWe separator station |
| [`stir-10s.md`](stir-10s.md) | Run the stirrer at 60 RPM for 10 seconds, then stop | ChinWe (stirrer = `motor-4`) |
| [`drain-funnel.md`](drain-funnel.md) | Open the drain valve, hold for 10 s, close back | ChinWe (drain valve = `motor-5`) |
| [`emm-motor-roundtrip.md`](emm-motor-roundtrip.md) | Visible-and-audible round-trip on each Emm V5.0 motor for ID-by-eye | ChinWe (motor-4 + motor-5) |
| [`bus-manifest.md`](bus-manifest.md) | Validate the bus manifest by listing 5 registered devices behind one node | ChinWe |
| [`media-gateway.md`](media-gateway.md) | Expose an ONVIF camera as RTSP/HLS/WebRTC via mediamtx | ONVIF camera + creds |

## Recipes that don't have CLI equivalents (yet)

The following examples need raw transport access — they bypass the engine
to send arbitrary bytes for hardware bring-up — and don't have a clean
gRPC mapping today. They remain as Rust binaries under
`crates/osdl-cli/examples/`:

- `espnow_probe.rs` — minimum viable dongle+node loopback test.
- `chinwe_scan.rs` / `laiyu_scan.rs` — bus address probes.
- `emm_motors_probe_via_espnow.rs` — driver-level probe with bytes-on-the-wire visibility.
- `test_chinwe.rs` / `test_laiyu.rs` — direct-TCP/serial bring-up.

These will be replaced when an `osdl probe` RPC lands. Until then, the
existing `cargo run --example …` invocations remain the right tool for
new-hardware bring-up.
