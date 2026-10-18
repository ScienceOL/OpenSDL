# UV 10 spectrophotometer: original design

Independent generic design continuing OpenSDL 33912808a56d7064c016cf8f0ac36ce7c82a8a30. No external meshes, CAD, textures, generator, dimensions or spectral dataset are used. Original metre-scale housing encloses a working sample cavity with a supported opaque cover, four-post carrier and hollow rectangular 10 mm optical-path cuvette.

The synthetic sample has A(lambda) = 0.05 + 1.15 exp(-0.5 ((lambda - 520)/45)^2) at a 10 mm path. Transmittance is 10^(-A). These are declared educational parameters, not measured spectra or chemical identification. Lamp readiness takes a declared 3 s. A closed blank must be acquired at the current wavelength before measuring the synthetic sample; changing wavelength or power invalidates the reference. Opening the sample bay invalidates the displayed result. No lamp spectrum, noise, stray light, detector physics, optical accuracy or real assay calibration is claimed.

CC BY-NC 4.0 for model-specific geometry, parameters and assets, attribution OpenSDL / LiyanLabs. Generic runtime/exporter retains its repository license. Internal geometry identifiers are inert. Rebuild with pinned Blender 4.5.3 and tools/instrument-models/build_catalog.py. Release compatibility, Hub publication and actual Dev/mobile acceptance remain open.
