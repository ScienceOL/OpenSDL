"""Compare original casting constellations against an aligned, unskinned GLB.

Metadata/names/indices are not used for the geometric comparison. Editing,
rescaling, unaligned re-export or remeshing may destroy the pattern. A match
is comparative provenance evidence, not a legal determination of infringement.
"""

import argparse
import hashlib
import itertools
import json
from pathlib import Path
import struct


def multiply(a, b):
    return [
        [sum(a[r][i] * b[i][c] for i in range(4)) for c in range(4)] for r in range(4)
    ]


def transform(node):
    if "matrix" in node:
        return [[node["matrix"][c * 4 + r] for c in range(4)] for r in range(4)]
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    s = node.get("scale", [1, 1, 1])
    t = node.get("translation", [0, 0, 0])
    rotation = [
        [1 - 2 * y * y - 2 * z * z, 2 * x * y - 2 * z * w, 2 * x * z + 2 * y * w],
        [2 * x * y + 2 * z * w, 1 - 2 * x * x - 2 * z * z, 2 * y * z - 2 * x * w],
        [2 * x * z - 2 * y * w, 2 * y * z + 2 * x * w, 1 - 2 * x * x - 2 * y * y],
    ]
    return [[rotation[r][c] * s[c] for c in range(3)] + [t[r]] for r in range(3)] + [
        [0, 0, 0, 1]
    ]


def vertices(path):
    data = path.read_bytes()
    if data[:4] != b"glTF":
        raise ValueError("expected GLB")
    length, kind = struct.unpack_from("<II", data, 12)
    if kind != 0x4E4F534A:
        raise ValueError("expected JSON chunk")
    gltf = json.loads(data[20 : 20 + length])
    binary = data[28 + length :]
    points = set()

    def visit(index, parent):
        node = gltf["nodes"][index]
        matrix = multiply(parent, transform(node))
        if "mesh" in node:
            for primitive in gltf["meshes"][node["mesh"]]["primitives"]:
                accessor = gltf["accessors"][primitive["attributes"]["POSITION"]]
                if (
                    accessor["componentType"] != 5126
                    or accessor["type"] != "VEC3"
                    or "sparse" in accessor
                ):
                    raise ValueError("requires ordinary float VEC3 positions")
                view = gltf["bufferViews"][accessor["bufferView"]]
                start = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
                if view.get("buffer", 0) != 0:
                    raise ValueError("external buffers are unsupported")
                for i in range(accessor["count"]):
                    v = struct.unpack_from(
                        "<3f", binary, start + i * view.get("byteStride", 12)
                    ) + (1,)
                    points.add(
                        tuple(
                            round(sum(matrix[r][c] * v[c] for c in range(4)), 6)
                            for r in range(3)
                        )
                    )
        for child in node.get("children", []):
            visit(child, matrix)

    identity = [[float(r == c) for c in range(4)] for r in range(4)]
    for node in gltf["scenes"][gltf.get("scene", 0)]["nodes"]:
        visit(node, identity)
    return points


def compare(path, design):
    points = vertices(path)
    token = hashlib.sha256(
        f'OpenSDL / LiyanLabs | {design["id"]} | {design["version"]}'.encode()
    ).digest()[:16]
    anchor = design["origin_anchor_m"]
    groups = []
    for group in range(4):
        matches = 0
        for i in range(32):
            bit = (token[group * 4 + i // 8] >> (i % 8)) & 1
            center = (
                anchor[0] + (group - 1.5) * 0.004 + (i % 4 - 1.5) * 0.0008,
                anchor[1] + (i // 4 - 3.5) * 0.0008,
                anchor[2] + bit * 0.00045,
            )
            corners = [
                (
                    round(center[0] + x, 6),
                    round(center[2] + z, 6),
                    round(-center[1] - y, 6),
                )
                for x, y, z in itertools.product([-0.00015, 0.00015], repeat=3)
            ]
            matches += all(corner in points for corner in corners)
        groups.append(matches)
    return {
        "file_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "compared_model": design["id"],
        "edition": design["version"],
        "matched_inserts": groups,
        "expected_inserts_per_group": 32,
        "alignment": "authored rest frame, metres, Y-up",
        "interpretation": "Comparative geometry evidence only. A non-match is inconclusive; a match alone does not establish authorship or infringement.",
    }


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("glb", type=Path)
    p.add_argument("--design", required=True, type=Path)
    a = p.parse_args()
    print(json.dumps(compare(a.glb, json.loads(a.design.read_text())), indent=2))
