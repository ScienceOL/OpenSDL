# Recipe — Emm V5.0 motor visible round-trip

Replaces `emm_motor_nudge_via_espnow.rs` and
`emm_motor_roundtrip_via_espnow.rs`. Use this when you've just plugged in
a fresh ChinWe station and need to identify which motor is the stirrer
(spins freely) vs. the drain valve (rotates a small amount). The motion
is large (800 pulses ≈ 1/4 turn) and slow (5 RPM ≈ 3 s per leg) so it's
trivially visible and audible.

Each motor goes forward then back, so net `Δposition = 0` — safe to
re-run.

## The recipe

```sh
for n in 4 5; do
  DEV="espnow:30EDA0B65B38:motor-$n"
  echo "--- motor-$n ---"

  osdl --instance chinwe send "$DEV" enable -p enable=true
  sleep 0.3

  # Forward.
  osdl --instance chinwe send "$DEV" run_position \
    -p pulses=800 -p speed=5 -p direction=0 -p acceleration=10 -p absolute=false
  sleep 5

  # Back.
  osdl --instance chinwe send "$DEV" run_position \
    -p pulses=800 -p speed=5 -p direction=1 -p acceleration=10 -p absolute=false
  sleep 5
done
```

Watch decoded events:

```sh
osdl --instance chinwe events --kinds device_status,command_result --json
```

## Identification

- The **stirrer** (typically `motor-4`) shaft visibly spins ~90° each leg.
- The **drain valve** (typically `motor-5`) shaft also rotates ~90°, but
  the motion is constrained by the valve mechanism — listen for the
  detent click.

If they're swapped on your hardware, update the `role:` and `description:`
fields in [`configs/chinwe-station.yaml`](configs/chinwe-station.yaml)
accordingly.
