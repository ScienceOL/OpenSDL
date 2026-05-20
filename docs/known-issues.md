# Known Issues

Things we know are wrong or incomplete and have decided not to fix yet,
with enough context that someone (us, in a few weeks) can pick them up
without re-deriving the problem.

If something is purely a bug we plan to fix shortly, file a GitHub issue
instead. This file is for "deliberate gaps with a reasoning trail."

## Drain valve: position tracking + boundary enforcement

**Symptom.** Sending `motor-5` (drain valve) a `run_position` command
with too many pulses, or with `acceleration=0` / high `speed`, can:

- spin the motor head against the valve seat,
- skip steps loudly (loud rattling, possible mechanical stress),
- leave the valve in an unknown position because the encoder advances
  even when the mechanism doesn't.

Surfaced live on **2026-05-20** when the recipe's default
`-p speed=30 -p acceleration=0` stalled motor-5 against its end stop.

**Operational workaround** (now in `docs/recipes/drain-funnel.md`):

- never `pulses > 800` for motor-5;
- always pair an open with a close so net Δ = 0 per cycle;
- use `speed=5`, `acceleration=10` (the original example values).

**What's actually missing in code.** Three pieces in
`crates/osdl-core/src/driver/builtins/emm.rs`:

1. **Live position tracking.** `get_position` does return the encoder
   value (signed, in steps), but the decoder doesn't update a tracked
   `position` property on writes. So callers can't read a live position
   from `device get` without first issuing `get_position` themselves.
   Either:
   - update the tracked `position` by accumulating the commanded delta
     when a write completes (cheap, but lies if the motor stalled), or
   - automatically issue a `get_position` after each write (truthful,
     adds bus round-trip per command).
2. **Calibrated end stops.** Even with live position, we'd need to
   know the open and closed pulse counts to enforce limits. Today
   "800 pulses ≈ 90°" is just a convention from the original example
   code; there's no calibration step that stamps the actual values for
   a specific physical valve. Bus manifest fields like `open_pulses` /
   `closed_pulses` on valve-roled motors would carry the calibration.
3. **Refusing the command.** Once 1 + 2 exist, `run_position` should
   refuse a command that would carry the encoder past the calibrated
   bounds, returning `failed_precondition` rather than letting the
   firmware skip steps.

**Why we haven't done it.** Each piece has design tradeoffs and the
operational workaround keeps the hardware safe today. Worth doing
properly before a third valve type lands or before unattended
operation; until then, the recipe's "never `pulses > 800`, always
paired Δ=0" is the contract.

## `Shutdown.graceful` not implemented

The proto field has been reserved (see
`crates/osdl-proto/proto/osdl.proto`) so we can re-add it without
breaking the wire format. Implementing it requires per-command
lifecycle tracking — see the next item.

## `SendCommand` returns `PENDING` always

The engine dispatches the encoded bytes to the transport and returns
immediately. It does **not** correlate the device's reply back to the
originating command, so the proto's `Succeeded` / `Failed` /
`Cancelled` states are unreachable.

Treat `PENDING` as "command dispatched without error" and observe
effects via `device_status` events / `device get`. Closing this gap
requires per-codec response correlation: each adapter would need to
match a reply frame to a pending command id, then emit
`OsdlEvent::CommandResult { status: Succeeded | Failed }` with the
matching id. The runze codec is the easiest place to start (replies
are addressed and ordered), the Emm V5.0 codec is harder (binary,
short replies). Until this lands:

- `SendCommand` lies about completion;
- `Shutdown.graceful` cannot be implemented (no in-flight set to drain).

## ESP-NOW raw probes only available as Rust binaries

The `chinwe_scan`, `laiyu_scan`, `espnow_probe`, `emm_motors_probe_via_espnow`,
`test_chinwe`, `test_laiyu` examples bypass the engine and write
arbitrary bytes for hardware bring-up. They're kept under
`crates/{osdl-cli,osdl-core}/examples/` until an `osdl probe` RPC
lands. The recipe README points there. Not urgent — these are bring-up
tools, not user-facing flows.
