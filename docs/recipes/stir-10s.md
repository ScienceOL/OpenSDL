# Recipe — Stir 10 seconds at 60 RPM

Replaces `stir_10s.rs`. Spins motor-4 at 60 RPM for 10 seconds and stops.

Assumes a server is already running with the ChinWe bus manifest (see
[`chinwe-station.md`](chinwe-station.md)) under `--instance chinwe`. If
not, boot one first.

## The recipe

```sh
M4='espnow:30EDA0B65B38:motor-4'   # adjust MAC to yours

osdl --instance chinwe send "$M4" enable -p enable=true
sleep 0.3

osdl --instance chinwe send "$M4" run_speed \
  -p speed=60 -p direction=0 -p acceleration=10

sleep 10

osdl --instance chinwe send "$M4" stop
```

Or the same as a one-liner so the timing is exact:

```sh
M4='espnow:30EDA0B65B38:motor-4'
osdl --instance chinwe send "$M4" enable -p enable=true && \
sleep 0.3 && \
osdl --instance chinwe send "$M4" run_speed -p speed=60 -p direction=0 -p acceleration=10 && \
sleep 10 && \
osdl --instance chinwe send "$M4" stop
```

## What you should see

- Each `osdl send` returns `status: PENDING` (the engine has dispatched
  the command). The Emm decoder emits `status: ack` device_status events
  for each write — visible via:

  ```sh
  osdl --instance chinwe events --kinds device_status &
  ```

- Physically: the stirrer spins for ~10 seconds, then halts.

## Notes

- `direction=0` is one rotation sense; flip to `1` to spin the other way.
- `acceleration=10` is intentionally low — start gently with anything
  attached to the shaft.
- A `run_position` command would do a known number of pulses instead;
  see [`drain-funnel.md`](drain-funnel.md) for that pattern.
