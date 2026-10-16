# P 200 Single Channel Pipette original design

Geometry, dimensions, materials, motion pivots, display surfaces and operation fixtures are independently authored. No third-party CAD, meshes, textures, product identifiers or reference-model parameters are reused. These are generic educational instruments, not manufacturer replicas.

Rest-pose assembled width/depth/height: 150 / 100 / 266 mm. Moving envelope is checked separately from this resting size.

Original 20-200 uL hand pipette. Finite source, tip and receiver volumes conserve liquid under ideal displacement. No calibration accuracy, compressibility, viscosity or droplet simulation.

Model, model-specific authoring source, parameters and derived files: CC BY-NC 4.0. Attribution: OpenSDL / LiyanLabs. General exporter and runtime software retain their software license. Source identity includes internal nonfunctional casting marks and exact input hashes. These assist provenance comparison and are not unremovable or independent proof of infringement.

Rebuild using pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- pipette --output <new-directory> --proofs <directory>`.

The handheld instrument is 35 / 44.5 / 244 mm; its stand and two fixture vials define the assembled bounds. `move_to` explicitly places it at park/source/receiver; this is operator fixture placement, not a collision-checked robot trajectory. Aspirate and dispense require the appropriate fixture position. Source, tip and receiver liquid heights follow the same conserved volume state.

P200 volume steps are integer microlitres. The runtime stores source, tip and receiver volume as bounded integers; decimal sub-microlitre settings are rejected rather than rounded in a 0-digit display.
