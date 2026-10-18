# PP 56 Peristaltic Pump: original design

Independently authored generic instrument, continuing OpenSDL source `33912808a56d7064c016cf8f0ac36ce7c82a8a30` and this branch's original models. No external CAD, mesh, texture, generator or parameter table is an input. Dimensions and dynamic parameters are declared engineering assumptions, not manufacturer measurements or real-device calibration.

The original 440 × 250 mm kit includes two 100 mL synthetic reservoirs and continuous hollow tubing. The actual path length and the same declared 7 mL line volume determine the lumen radius. Roller centres orbit at 45 mm and followers have 15 mm radius; their prescribed counterrotation follows the stator rolling constraint. Nominal 2.8 mL/rev displacement is an original simulation parameter, not inferred or calibrated from elasticity. Opening/removal is blocked through deceleration. Priming debits the source; unloading accounts for retained liquid as removed volume. The model stops at source/receiver capacity and integrates the remaining coast time; it assumes flow ceases at that hard boundary without solving pressure or subsequent air/liquid displacement.

Three-roller head with 56 mL/min nominal maximum, priming and explicit source/receiver/line/removed liquid inventories. No validated tube deformation, pressure curve, occlusion forces or measured flow calibration.

The model, model-specific geometry source, parameters and derived assets use CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General runtime and exporter software retain their repository license. Inert internal geometry identifiers perform no network reporting.

Rebuild with pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- peristaltic-pump --output <new-directory> --proofs <directory>`. The previous signed 0.4.0 runtime does not implement this model. Release compatibility, immutable Hub publication and actual Dev/mobile acceptance remain open.
