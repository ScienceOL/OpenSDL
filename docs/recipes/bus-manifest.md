# Recipe — Verify a bus manifest

Replaces `bus_manifest_live.rs`. Boots a server with the ChinWe bus
manifest, waits for the node to register, and prints the resulting
device set so you can confirm the engine built 5 independently-
addressable records — same end state Xyzen Runner reaches when the user
drops the same manifest into `~/.xyzen/config.yaml`.

No commands are sent; the recipe is purely about REG-time validation.

## The recipe

```sh
osdl serve --detach \
  --instance bus-check \
  --config docs/recipes/configs/chinwe-station.yaml \
  --registry $(pwd)/registry/unilabos \
  --dongle-port /dev/cu.usbserial-A5069RR4

# Block on the first device — the bus manifest registers all 5 at once,
# so any one appearing means we're good.
osdl --instance bus-check device wait \
  id:'espnow:30EDA0B65B38:pump-1' --timeout 25s

# Show the full set.
osdl --instance bus-check device list

osdl --instance bus-check stop
```

## Pass criteria

- Five devices listed.
- Each has a `role` from the manifest (`syringe_pump` ×3, `stirrer`,
  `drain_valve`).
- Each shares the same `transport_id` (the node's MAC under `espnow:…`).

## Common failures

| Symptom | Cause |
|---|---|
| `device wait` times out | Node didn't REG — power, dongle port, or MAC mismatch |
| Only 1 device, id `espnow:<MAC>` (no `:pump-1` suffix) | `match_hardware_id` in your YAML doesn't match the node's announced `hardware_id` — engine fell back to the legacy 1:1 path |
| 5 devices but missing roles / wrong descriptions | Stale YAML — re-launch after editing the config |
| 4 devices instead of 5 | A `device_type` in the manifest doesn't resolve in the registry — engine logs a warn for the skipped entry. Check the server log. |
