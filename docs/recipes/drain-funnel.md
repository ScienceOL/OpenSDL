# Recipe — Drain the separatory funnel

Replaces `drain_funnel_via_espnow.rs` and the follow-up
`drain_funnel_nudge_more.rs`. Toggles the drain valve (`motor-5`) by
800 pulses (~90°) per direction, holds for 10 seconds so the funnel
drains, then closes back.

Assumes a ChinWe server is already running under `--instance chinwe`.

## Direction convention

The valve has two stable positions, 90° apart, and toggles between them
by reversing direction:

- **`direction=0`** → open the funnel (drain flows)
- **`direction=1`** → close the funnel

The valve is normally **closed at rest**. So a typical drain cycle starts
with `direction=0` (open), waits, then issues `direction=1` (close).
800 pulses ≈ 90° rotation between the two positions on the current
gearing.

## Open / hold / close

```sh
M5='espnow:30EDA0B65B38:motor-5'

osdl --instance chinwe send "$M5" enable -p enable=true
sleep 0.3

# Open the funnel.
osdl --instance chinwe send "$M5" run_position \
  -p pulses=800 -p speed=5 -p direction=0 -p acceleration=10 -p absolute=false

sleep 10                       # let the funnel drain

# Close the funnel — opposite direction, same pulse count returns the
# valve to the seat. Net Δposition = 0.
osdl --instance chinwe send "$M5" run_position \
  -p pulses=800 -p speed=5 -p direction=1 -p acceleration=10 -p absolute=false
```

## If 800 pulses isn't enough — incremental nudges

Replaces `drain_funnel_nudge_more.rs`. Run this between drains until you
find the right open angle. Each invocation is reversible (Δ=0):

```sh
NUDGE=100
osdl --instance chinwe send "$M5" run_position \
  -p pulses=$NUDGE -p speed=5 -p direction=0 -p acceleration=10 -p absolute=false
sleep 10
osdl --instance chinwe send "$M5" run_position \
  -p pulses=$NUDGE -p speed=5 -p direction=1 -p acceleration=10 -p absolute=false
```

## Common stalls and gotchas

These are real things that happen on this motor. Read before you change
the parameters.

### Don't set `acceleration=0`

`acceleration=0` tells the Emm V5.0 firmware to **skip the accel ramp
entirely** — it kicks straight to target RPM with no soft start. Under
the valve's mechanical resistance the motor will stall and (since
holding torque stays applied) draw current against the bind, sometimes
making a loud rattle. Always use `acceleration=10` for the drain valve.

### Don't push `speed` past 5 RPM on the valve

The original code used 5 RPM precisely because higher speeds stall when
the valve hits its mechanical seat. The stirrer (`motor-4`) tolerates
60 RPM because it spins freely in air or fluid. **The valve does not.**

### No software boundary check yet

There is no software interlock against driving past the valve's
mechanical end stops. The current codec sends the requested pulse count
verbatim — if you pass `pulses=8000` (10× a quarter turn), the firmware
will *try* to rotate 10× the open angle. With `absolute=false` and a
valve that's already at one stop, this will:

- spin the motor head against the valve seat,
- skip steps loudly,
- leave the valve in an unknown position (because the encoder advances
  even when the mechanism doesn't).

Treat 800 pulses as the maximum safe move per direction until a
boundary check lands in code. Several issues need to be solved before
that's possible:

1. **Position tracking.** `get_position` does return the encoder value
   (signed, in steps), but the decoder doesn't yet update a tracked
   `position` property on writes — so command-side software can't know
   where the valve is without polling. The `os` codec emits
   `{"status": "ack"}` for writes and `{"position": N}` only on explicit
   `get_position`.
2. **Calibrated end stops.** Even with live position, we'd need to know
   the open and closed pulse counts to enforce limits. Today those are
   "800 pulses ≈ 90°" by convention — there's no calibration step that
   stamps the actual values for this physical valve.
3. **Refusing the command.** Once both pieces exist, the codec
   (`emm.rs`) should refuse a `run_position` that would carry the
   encoder past the calibrated bounds, returning a `failed_precondition`
   gRPC status rather than letting the firmware skip steps.

If you need this sooner, the operational workaround is: never run
`run_position` for motor-5 with `pulses > 800` and always pair an open
with a close (Δ=0 per cycle). That keeps the encoder roughly aligned
with reality even without software bounds.

### Other safety notes

- Don't run with `enable=false` first; the driver needs hold torque or
  the valve back-drives under fluid pressure.
- If a command appears to stall (motor humming but not turning), stop
  immediately:
  ```sh
  osdl --instance chinwe send "$M5" stop
  osdl --instance chinwe send "$M5" enable -p enable=false
  ```
  This cuts holding current so the motor isn't drawing power against
  the bind. Inspect the valve mechanically before re-energizing.
