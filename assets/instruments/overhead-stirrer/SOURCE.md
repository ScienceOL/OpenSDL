# OH 1000 Overhead Stirrer: original design

Independently authored generic instrument, based on OpenSDL source `33912808a56d7064c016cf8f0ac36ce7c82a8a30` and this branch's original work. No external mesh, CAD, texture, model generator or parameter table is an input. All metre-scale dimensions, dynamic parameters and synthetic samples are our design choices, not manufacturer measurements.

The 260 × 280 mm base supports a 20 mm diameter upright and a sliding carriage. The shaft and crossed paddles belong to the carriage; the motor display and encoder move with it. The minimum shaft tip is above the vessel floor, and the declared immersion window keeps the paddle below the synthetic liquid surface. Load/unload requires the fully raised shaft; motion requires a loaded vessel, locked stand and immersed paddle. Height and clamp operations are blocked through deceleration.

100 mm carriage travel, explicit stand clamp, immersed paddle and 1000 rpm maximum. No fluid torque, viscosity, shear rate, sloshing or reaction is predicted.

The model, author parameters, model-specific geometry source and derived assets are CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General exporter and runtime software retain their repository license. The inert internal geometry identifier performs no network reporting.

Build using pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- overhead-stirrer --output <new-directory> --proofs <directory>`. The package requires this branch's new runtime model; the previously signed 0.4.0 binary does not implement it. Hub publication, release compatibility and actual Dev/mobile acceptance remain separate gates.
