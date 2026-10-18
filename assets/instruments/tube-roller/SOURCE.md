# TR 60 Tube Roller: original design

Independently authored generic instrument, based on OpenSDL source `33912808a56d7064c016cf8f0ac36ce7c82a8a30` and this branch's original work. No external mesh, CAD, texture, model generator or parameter table is an input. All metre-scale dimensions, dynamic parameters and synthetic samples are our design choices, not manufacturer measurements.

The 320 × 200 × 55 mm housing carries five 240 mm rollers of radius 12 mm at 32 mm centres. A tube of radius 16 mm rests between two rollers at the geometrically calculated contact height. Tube and roller angles integrate separately so the noninteger ratio does not jump on each motor revolution. Removal is blocked during running and deceleration.

Five parallel rollers, 60 rpm maximum and 30 rpm/s slew. The tube rotates opposite the rollers with a 12:16 radius ratio. Motion is prescribed no-slip kinematics; contact, wobble and mixing efficiency are not solved.

The model, author parameters, model-specific geometry source and derived assets are CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General exporter and runtime software retain their repository license. The inert internal geometry identifier performs no network reporting.

Build using pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- tube-roller --output <new-directory> --proofs <directory>`. The package requires this branch's new runtime model; the previously signed 0.4.0 binary does not implement it. Hub publication, release compatibility and actual Dev/mobile acceptance remain separate gates.
