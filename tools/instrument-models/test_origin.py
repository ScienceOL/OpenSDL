import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from verify_origin import compare
from attestation_bundle import bundle

REPO = Path(__file__).resolve().parents[2]


class OriginTest(unittest.TestCase):
    def test_geometry_survives_removal_of_all_names_and_metadata(self):
        asset = REPO / "assets/instruments/heated-stirrer"
        design = json.loads((asset / "model.yaml").read_text())
        data = (asset / "package/preview.glb").read_bytes()
        length = struct.unpack_from("<I", data, 12)[0]
        gltf = json.loads(data[20 : 20 + length])
        for objects in ("nodes", "meshes", "materials", "scenes"):
            for obj in gltf.get(objects, []):
                obj.pop("name", None)
                obj.pop("extras", None)
        gltf.pop("asset", None)
        gltf["asset"] = {"version": "2.0"}
        encoded = json.dumps(gltf, separators=(",", ":")).encode()
        encoded += b" " * (-len(encoded) % 4)
        binary = data[20 + length :]
        with tempfile.TemporaryDirectory() as folder:
            suspect = Path(folder) / "stripped.glb"
            suspect.write_bytes(
                struct.pack(
                    "<5I",
                    0x46546C67,
                    2,
                    20 + len(encoded) + len(binary),
                    len(encoded),
                    0x4E4F534A,
                )
                + encoded
                + binary
            )
            self.assertNotEqual(
                hashlib.sha256(data).hexdigest(),
                hashlib.sha256(suspect.read_bytes()).hexdigest(),
            )
            self.assertEqual(compare(suspect, design)["matched_inserts"], [32] * 4)
            wrong = {**design, "version": "unrelated-edition"}
            self.assertLess(sum(compare(suspect, wrong)["matched_inserts"]), 128)

    def test_archive_is_deterministic_and_includes_the_verified_source(self):
        with tempfile.TemporaryDirectory() as folder:
            a = Path(folder) / "a.tar"
            b = Path(folder) / "b.tar"
            self.assertEqual(bundle(a), bundle(b))
            self.assertGreater(a.stat().st_size, 100000)


if __name__ == "__main__":
    unittest.main()
