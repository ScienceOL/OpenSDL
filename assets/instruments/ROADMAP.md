# Original instrument library: 10 + 12

Updated 2026-10-18. The first ten instruments are the starting batch, not a limit on the library. Expand by twelve distinct devices to a total of twenty-two. A listing, generated mesh, successful unit test, or Hub thumbnail is not end-to-end acceptance.

## Existing batch

Analytical balance, heated magnetic stirrer, pipette, centrifuge, vortex mixer, water bath, drying oven, vacuum pump, syringe pump, and pH meter have authored sources and immutable Hub assets. Desktop operations were exercised on 2026-10-16. Revalidation through the current independent OpenSDL plugin and mobile phone/tablet acceptance remain open; see the [Liyan acceptance record](https://github.com/ScienceOL/Liyan/blob/codex/common-instruments/docs/acceptance/2026-10-16-original-instruments.md).

## Expansion queue

The order below prioritizes distinct mechanical structures and reusable model capabilities. Slugs identify planned original designs, not existing published assets. All twelve start unaccepted.

| # | Device / asset slug | Required original geometry and interaction | Executable task and honest process boundary |
| --- | --- | --- | --- |
| 11 | Orbital shaker / `orbital-shaker` | Counter-rotating eccentric drive, level platform, nonslip mat, independent flask clamps, speed dial | Load and secure a flask, set speed, accelerate, orbit without spinning the platform, decelerate before unloading. Prescribed motion; no claim of fluid mixing efficiency or validated retention forces. |
| 12 | Tube roller / `tube-roller` | Parallel rollers, end bearings, removable tube, roller/tube opposing rotations | Load, set speed, run and stop before removal. Kinematic rolling with declared geometry; no contact solver or mixing prediction. |
| 13 | Overhead stirrer / `overhead-stirrer` | Stand, adjustable motor carriage, clamp, shaft, impeller, vessel and guard | Raise, clamp, lower the impeller into a vessel, set speed, run and stop before lifting. No fluid torque, shear-rate or reaction prediction. |
| 14 | Dry block heater / `dry-block-heater` | Machined block with actual open wells, individual tubes, insulation, screen and encoder | Load tubes, set temperature, heat, stop and retain residual heat. Separate block/sample temperatures with declared lumped parameters; no assay kinetics. |
| 15 | Ultrasonic cleaner / `ultrasonic-cleaner` | Open tank, removable basket, draining port, lid, control panel | Fill to an allowed level, load basket, set time/temperature, sonicate, stop and drain only when permitted. Timed power and thermal model; no fabricated cavitation or cleaning efficacy. |
| 16 | Peristaltic pump / `peristaltic-pump` | Hinged head, rollers, curved continuous tube, inlet/outlet and flow control | Open head, install tubing, close, prime, pump, stop. Declared displacement and source/receiver inventory; no validated tube deformation or pressure curve. |
| 17 | Rotary evaporator / `rotary-evaporator` | Lift, motor and rotating flask, bath, condenser, receiving flask and vacuum ports | Mount flask, lower into bath, rotate, heat, reduce pressure, stop, vent and lift. Thermal/pressure states first; evaporation and recovery stay unavailable until a separately validated mass-transfer model exists. |
| 18 | Vacuum filtration assembly / `vacuum-filtration` | Funnel, filter support, seal, receiving flask, tubing and valve | Assemble and seal, load a synthetic slurry, evacuate, vent before opening. Pressure and explicit synthetic liquid transfer; no unsupported separation efficiency. |
| 19 | Conductivity meter / `conductivity-meter` | Stand, liftable probe, cell, sample vessel, screen and keys | Power, immerse, settle, calibrate against declared synthetic standards, lift to unavailable. Declared probe response and temperature correction; no claim of real sample identification. |
| 20 | UV–visible spectrophotometer / `uv-vis-spectrophotometer` | Sample compartment, lid, cuvette carrier, removable cuvettes, display | Insert blank, close, zero, insert synthetic sample and read. Explicit synthetic spectrum; no measured analytical accuracy or chemical identification. |
| 21 | PCR thermal cycler / `thermal-cycler` | Heated hinged lid, latch, real well block, tube strip and controls | Load strip, latch, execute a bounded temperature/hold program, finish and cool. Thermal schedule and block/sample lag; no DNA amplification or yield prediction. |
| 22 | Laboratory refrigerator / `laboratory-refrigerator` | Insulated cabinet, hinged gasketed door, shelves, sample rack and thermostat | Open, load, close, set temperature, cool and observe door-open recovery. Declared compartment/sample thermal model; no cold-chain qualification. |

### Current implementation evidence

OS 250 has its first original author source, generated GLB/USD, model graph and executable world fixture. The pinned Blender 4.5.3 build measured 320.45 × 310.30 × 252.00 mm (source X/Y/Z), 469,748 preview bytes, 15,025 triangles and 24 draw primitives. The rendered front perspective has been inspected. The SDK tests load the actual exported mesh, check the full 25 mm orbit without platform rotation, confirm the flask/clamp parent assembly and exercise the clamp and speed controls. Runtime tests cover clamp interlocks through deceleration, rejection atomicity, inventory release and step-partition-invariant ramp angles. The complete SDK suite passes 25 tests; the `osdl-core` suite passes with the `espnow` feature used by the CLI build.

This is source and prototype evidence only. Rear/open/moving visual inspection, release runtime compatibility, Hub publication with an immutable digest, current plugin packaging and actual Dev/mobile operations remain open. The previously signed 0.4.0 binary does not implement `orbital_shaker`; do not publish this candidate against that binary or count it as the eleventh accepted device.

## Per-device completion gate

1. Commit the original parameter source, geometry builder and evidence statement. Design dimensions are metre-scale engineering choices; they are not manufacturer measurements. Do not import third-party meshes, textures, parameter tables, generator templates or test expectations.
2. Rebuild GLB, USD, graph, world configuration, provenance and validation from those inputs with the pinned author toolchain. Inspect the rendered front, rear, open/loaded and moving states. Check support-plane origin, declared bounds, motion envelope, pivots, clearances and visible working cavities.
3. Implement authoritative state transitions in the selected Runner's OpenSDL runtime. Reject invalid commands without state changes; test time evolution, stopping, loading ownership and observable quantities. Keep numerical steps independent of accumulated history.
4. Publish a new immutable OCI version and confirm Hub 3D structure preview, scale and part controls. Record the exact digest. No mutable replacement of an accepted version.
5. Install the corresponding plugin build and execute the task in the actual local Dev desktop client. Repeat representative operations in Android/iOS phone and tablet clients, including landscape, narrow split view and parameter keyboard. Record the exact versions and observations; shared source is not mobile acceptance.
6. Measure model bytes, cold/warm load, render resource reuse and current-state command costs. Append evidence to the acceptance record before marking the device accepted.

## Licensing and attribution

New original models, textures, authored model parameters and model-specific geometry sources use CC BY-NC 4.0 with OpenSDL / LiyanLabs attribution. General software retains its repository license. Keep source/output hashes and inert internal geometry identifiers; no hidden network telemetry or claims that identifiers are irremovable legal proof. Previously granted licenses remain valid.
