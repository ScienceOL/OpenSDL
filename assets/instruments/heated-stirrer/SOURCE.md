# HS 150 original design

This asset is authored from primitive solids and the adjacent dimensioned input. No third-party meshes, CAD, textures, product marks, process scripts or instrument parameters are reused.

The housing, support frame, twin dial layout, vent pattern, sample vessel, probe bracket and motion pivots are original generic engineering fixtures. Width 188 mm, assembled depth 303 mm (housing 292 mm), base height 120 mm; the fitted probe assembly reaches 388 mm. The ceramic plate is 150 mm diameter. Metre dimensions describe this authored assembly, not a replica of a commercial instrument.

The thermal process uses a 360 J/K plate and a 200 g water fixture at an assumed constant 4180 J/(kg K), with declared heat exchange/loss conductances. It excludes boiling, evaporation, viscous flow, magnetic decoupling and calibrated PID. Setpoint controls the plate, and the immersed probe separately observes the liquid. Motor acceleration is 300 rpm/s; the rotating bar is a kinematic pose, not a contact simulation. Power-off preserves heat and ramps rotation down. The water fixture accepts a plate setpoint at most 95 C and cannot be loaded onto a hotter plate, keeping this phase-change-free model within its declared domain. The empty plate allows 150 C; liquid cutoff at 98 C is a simulation rule.

The model, model-specific geometry source, parameters and derived assets are CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General exporter/runtime code retains its repository software license. Build with pinned Blender 4.5.3 and tools/instrument-models/build_stirrer.py.
