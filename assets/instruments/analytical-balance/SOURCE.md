# AX 240 analytical balance

Original OpenSDL geometry and engineering design, licensed under the repository's
MIT license. No third-party meshes, textures, parameters, contract files, or
manufacturer artwork are inputs. The AX 240 is a generic educational instrument,
not a model or certified digital twin of a commercial product.

`model.yaml` uses the JSON subset of YAML so both Blender Python and the Rust
asset pipeline can read the exact same authored parameters without a Blender
runtime package dependency. Coordinates are right-handed metres, X right,
Y back, Z up. The closed instrument is 236 × 354 × 336 mm. The origin is the
centre of its supporting footprint. The three doors slide toward the rear.

The pan is a 43 mm radius support at Z=96 mm. Pan load is derived from explicitly
placed synthetic samples. It is not computed by a contact-force solver. Capacity,
resolution, settling interval, and stability conditions are explicit engineering
fixtures. Resolution is not a claim of measurement accuracy.

The committed builder owns the rounded enclosure, framed hollow draft shield,
three separate glass doors, steel pan, display, keys, feet, connectors, and sample
vessel. Embedded model metadata identifies components and their local motion.
All display text at runtime comes from the selected instance's current state.

Build with official Blender 4.5.3 LTS:

```sh
blender --background --factory-startup --python-exit-code 1 \
  --python tools/instrument-models/build_balance.py -- --output <new-directory> --proofs <proof-directory>
```

The builder emits the OpenSDL asset directory, engine configuration, engineering
USD, GLB, thumbnail, provenance, geometry metrics and independent pose proofs.
Authoring sources remain in this repository; users need no Blender installation.

The package carries optional fixture meshes. Hosts apply the portable visibility bindings
at load time; only fixtures confirmed by the local world are shown on the pan. The Hub's
joint sliders inspect geometry without running the device. A device observation includes
a per-run identity and monotonic revision so restarting a local world cannot reuse stale poses.
