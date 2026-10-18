# DB 6 Dry Block Heater: original design

Independently authored generic instrument, based on OpenSDL source `33912808a56d7064c016cf8f0ac36ce7c82a8a30` and this branch's original work. No external mesh, CAD, texture, model generator or parameter table is an input. All metre-scale dimensions, dynamic parameters and synthetic samples are our design choices, not manufacturer measurements.

The 220 × 200 mm housing carries an original 145 × 95 × 50 mm machined block. Six blind wells leave a real bottom and radial clearance around 12 mm tubes. Tubes are a declared group fixture, with one aggregate thermal node; their separate temperatures are not claimed. The cover has its own rear hinge and stays above all caps. Opening stops heating, and unloading hot samples is rejected. The same thermal transition implementation serves live commands and controlled tests.

Six 14 mm open wells, a hinged clear cover, 150 W maximum heater, 600 J/K block and 60 J/K synthetic sample node. Block/sample lag and residual heat use the canonical thermal chamber model. No assay kinetics, real sample calibration or temperature-uniformity claim.

The model, author parameters, model-specific geometry source and derived assets are CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General exporter and runtime software retain their repository license. The inert internal geometry identifier performs no network reporting.

Build using pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- dry-block-heater --output <new-directory> --proofs <directory>`. The package requires this branch's new runtime model; the previously signed 0.4.0 binary does not implement it. Hub publication, release compatibility and actual Dev/mobile acceptance remain separate gates.
