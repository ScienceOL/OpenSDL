# UC 3.3 Ultrasonic Cleaner: original design

Independently authored generic instrument, continuing OpenSDL source `33912808a56d7064c016cf8f0ac36ce7c82a8a30` and this branch's original models. No external CAD, mesh, texture, generator or parameter table is an input. Dimensions and dynamic parameters are declared engineering assumptions, not manufacturer measurements or real-device calibration.

The 320 × 260 mm enclosure contains a real open stainless basin, a removable wire basket on feet, two inert sample vials, a supported gasketed cover and a separate drain valve. Full visible water volume is 3358.575 mL; its declared thermal capacity uses the explicit 4.18 J/(mL K) approximation. Filling mixes cold water by heat capacity. A cycle requires sufficient water, a loaded basket and a closed lid. Timer expiry turns off input; residual heat remains. Drain/fill and hot sample removal have explicit interlocks.

Timed bath with 300 W heater and 30 W declared acoustic thermal input. No cavitation, cleaning efficacy, evaporation or temperature-uniformity prediction.

The model, model-specific geometry source, parameters and derived assets use CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General runtime and exporter software retain their repository license. Inert internal geometry identifiers perform no network reporting.

Rebuild with pinned Blender 4.5.3 and `tools/instrument-models/build_catalog.py -- ultrasonic-cleaner --output <new-directory> --proofs <directory>`. The previous signed 0.4.0 runtime does not implement this model. Release compatibility, immutable Hub publication and actual Dev/mobile acceptance remain open.
