# EC 2 conductivity meter: original design

Independent generic design continuing OpenSDL 33912808a56d7064c016cf8f0ac36ce7c82a8a30. No external CAD, mesh, texture, generator, calibration table or manufacturer dimensions are inputs. The metre-scale base, stand, lift carriage, separated cell electrodes and open guard are original geometry. Raising the cell clears the vessel rim before a solution can be changed or removed.

Both 1413 uS/cm standard and 800 uS/cm sample are explicitly synthetic values at 25 C. The declared linear coefficient is 0.02/K, probe gain 1.025 and first-order response time 2 s. Calibration estimates a multiplicative correction only after the standard settles. The readout compensates to 25 C; dry or powered-off cells have unavailable readings. This is an educational response model, not real sample identification, validated cell constant or laboratory accuracy.

Model geometry, parameters and assets: CC BY-NC 4.0, attribution OpenSDL / LiyanLabs. General runtime/exporter retains its license. Internal geometry identifiers are inert. Rebuild with pinned Blender 4.5.3 and tools/instrument-models/build_catalog.py. Runtime release compatibility, Hub publication and actual Dev/mobile acceptance remain open.
