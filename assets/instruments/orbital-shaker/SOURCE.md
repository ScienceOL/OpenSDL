# OS 250 original orbital shaker

This is an independently authored generic instrument, not a replica of a commercial product. The starting source is OpenSDL `33912808a56d7064c016cf8f0ac36ce7c82a8a30`; no external meshes, CAD, textures, geometry scripts or parameter tables are inputs.

The enclosure is 320 × 300 × 90 mm, with a 250 × 250 mm platform, a 25 mm orbit diameter and one original conical flask fixture. Dimensions, 500 rpm limit and 120 rpm/s acceleration are design choices for this simulator, not manufacturer measurements or validated operating limits. Geometry uses metres with Blender Z up; generated GLB uses Y up. Platform height and flask seating are authored in the same geometry source as the moving assembly.

The eccentric pivot rotates by θ and carries an offset platform pivot rotating by −θ. Their world rotations cancel, giving a level, nonspinning circular translation. The flask, mat and all three clamp fingers belong to that platform. Each finger has its own hinge; a single clamp action represents securing/releasing the fixture. Loading, unclamping and unloading are blocked while the motor runs or decelerates. This is an explicit fixture workflow, not simulated grasp/contact forces.

No liquid sloshing, mixing efficiency, retention strength, vibration, motor torque or load-dependent speed accuracy is modeled. A visible synthetic liquid is an inert geometric fixture. Clamp status establishes a simulation interlock only.

Models, parameters, model-specific geometry and derived assets are CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General exporter and runtime software keep their repository license. The inert internal source identifier contains no network behavior.

Rebuild with pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- orbital-shaker --output <new-directory> --proofs <directory>`. A generated package is not Hub publication or Dev acceptance; record those separately in the library roadmap.
