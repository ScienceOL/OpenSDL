# SPDX-License-Identifier: CC-BY-NC-4.0
"""Build the original AX 240; run inside official Blender 4.5.3."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from geometry import box, component, cylinder, label, material, studio, vessel

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO / "assets/instruments/analytical-balance"


def create_model(p):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    palette = {
        "shell": material("Warm ceramic enamel", (0.72, 0.75, 0.76), roughness=0.28),
        "trim": material("Graphite polymer", (0.035, 0.048, 0.056), roughness=0.38),
        "steel": material("Satin stainless steel", (0.65, 0.70, 0.74), 0.9, 0.24),
        "glass": material(
            "Draft shield glass", (0.88, 0.96, 0.98), roughness=0.08, transmission=0.96
        ),
        "rubber": material("Elastomer", (0.018, 0.022, 0.025), roughness=0.8),
        "screen": material("Inactive display", (0.008, 0.018, 0.021), roughness=0.3),
        "ink": material("Legends", (0.48, 0.58, 0.62), roughness=0.6),
        "accent": material("Anodised dial", (0.10, 0.25, 0.30), 0.65, 0.28),
    }
    root = component("instrument")
    parts = {
        name: component(name, root)
        for name in (
            "body",
            "draft_shield",
            "door_left",
            "door_right",
            "door_top",
            "pan",
            "screen",
            "power_key",
            "tare_key",
            "zero_key",
            "vessel",
            "mass_5g",
            "mass_20g",
            "overload",
        )
    }
    shell, trim, steel, glass = (
        palette[k] for k in ("shell", "trim", "steel", "glass")
    )
    body = parts["body"]
    box(body, "Lower enclosure", (0, 0, 0.033), (0.236, 0.354, 0.046), shell, 0.009)
    box(body, "Upper enclosure", (0, 0.043, 0.061), (0.228, 0.260, 0.028), shell, 0.006)
    box(body, "Enclosure seam", (0, 0, 0.033), (0.236, 0.350, 0.0012), trim, 0.0004)
    for x in (-0.089, 0.089):
        for y in (-0.132, 0.132):
            cylinder(
                body, "Levelling foot", (x, y, 0.008), 0.016, 0.016, palette["rubber"]
            )
            cylinder(body, "Foot collar", (x, y, 0.019), 0.012, 0.008, steel)
    # The real support cavity is open above this deck.
    box(body, "Weighing deck", (0, 0.041, 0.079), (0.207, 0.225, 0.008), steel, 0.002)
    cylinder(parts["pan"], "Pan support", (0, 0.041, 0.087), 0.014, 0.012, trim)
    cylinder(
        parts["pan"],
        "Weighing pan",
        (0, 0.041, p["pan_height_m"] - 0.002),
        p["pan_radius_m"],
        0.004,
        steel,
        vertices=96,
    )
    frame = parts["draft_shield"]
    for x in (-0.108, 0.108):
        for y in (-0.068, 0.151):
            box(
                frame,
                "Frame upright",
                (x, y, 0.207),
                (0.006, 0.006, 0.254),
                shell,
                0.001,
            )
        for z in (0.083, 0.331):
            box(frame, "Door rail", (x, 0.042, z), (0.008, 0.230, 0.007), steel, 0.001)
    for y in (-0.068, 0.151):
        box(frame, "Upper crossmember", (0, y, 0.326), (0.222, 0.006, 0.006), shell)
    box(
        frame,
        "Front fixed glass",
        (0, -0.068, 0.205),
        (0.209, 0.0028, 0.238),
        glass,
        0.0005,
    )
    box(
        frame,
        "Rear fixed glass",
        (0, 0.151, 0.205),
        (0.209, 0.0028, 0.238),
        glass,
        0.0005,
    )
    for side, x in (("left", -0.1135), ("right", 0.1135)):
        parent = parts[f"door_{side}"]
        box(
            parent,
            "Sliding glass",
            (x, 0.041, 0.205),
            (0.0028, 0.212, 0.238),
            glass,
            0.0005,
        )
        box(
            parent,
            "Door leading edge",
            (x + math.copysign(0.0005, x), -0.066, 0.205),
            (0.0035, 0.0035, 0.239),
            steel,
            0.0005,
        )
        box(
            parent,
            "Door handle",
            (math.copysign(0.1155, x), -0.048, 0.194),
            (0.004, 0.018, 0.038),
            trim,
            0.0015,
        )
    top = parts["door_top"]
    box(
        top,
        "Top sliding glass",
        (0, 0.041, 0.331),
        (0.205, 0.209, 0.0028),
        glass,
        0.0005,
    )
    box(top, "Top handle", (0, -0.052, 0.3342), (0.037, 0.012, 0.0036), trim, 0.001)
    # Tilted display plane with its own UV surface for live, instance-owned text.
    bezel = box(
        body, "Display bezel", (0, -0.121, 0.059), (0.182, 0.064, 0.010), trim, 0.004
    )
    bezel.rotation_euler.x = math.radians(24)
    bpy.ops.mesh.primitive_plane_add(size=1, location=(-0.005, -0.1236, 0.0645))
    screen = bpy.context.object
    screen.name = "Live display"
    screen.dimensions = (0.139, 0.040, 0)
    screen.rotation_euler.x = math.radians(24)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    screen.parent = parts["screen"]
    screen.data.materials.append(palette["screen"])
    for identifier, x, text in (
        ("power_key", -0.066, "POWER"),
        ("zero_key", 0, "ZERO"),
        ("tare_key", 0.066, "TARE"),
    ):
        key = box(
            parts[identifier],
            "Membrane key",
            (x, -0.159, 0.058),
            (0.047, 0.020, 0.004),
            palette["accent"] if identifier == "tare_key" else trim,
            0.002,
        )
        key.rotation_euler.x = math.radians(12)
        label(
            parts[identifier],
            text,
            (x, -0.1595, 0.0605),
            0.0045,
            shell,
            (math.radians(12), 0, 0),
        )
    label(body, "OpenSDL  /  AX 240", (0, -0.1771, 0.030), 0.006, palette["ink"])
    label(
        frame, "ORIGINAL INSTRUMENT MODEL", (0, -0.0712, 0.319), 0.004, palette["ink"]
    )
    for x in (-0.060, -0.035):
        cylinder(body, "Rear connector", (x, 0.171, 0.047), 0.006, 0.006, trim, "Y", 20)
    box(
        body,
        "USB connector",
        (0.025, 0.173, 0.046),
        (0.013, 0.004, 0.006),
        trim,
        0.0005,
    )
    for y in (0.082, 0.096, 0.110, 0.124):
        box(
            body,
            "Rear ventilation",
            (0.1141, y, 0.058),
            (0.0005, 0.008, 0.010),
            trim,
            0,
        )
    fixture = p["fixtures"]["vessel"]
    sample = vessel(parts["vessel"], glass, fixture)
    sample.location = (
        fixture["position_m"][0],
        0.041 + fixture["position_m"][1],
        p["pan_height_m"],
    )
    for identifier, fixture in p["fixtures"].items():
        if identifier == "vessel":
            continue
        radius = fixture["radius_m"]
        height = fixture["mass_kg"] / (
            fixture["density_kg_m3"] * math.pi * radius * radius
        )
        x, y = fixture["position_m"]
        cylinder(
            parts[identifier],
            "Reference weight",
            (x, 0.041 + y, p["pan_height_m"] + height / 2),
            radius,
            height,
            steel,
        )
    return root, parts


def model_graph(p):
    labels = {
        "body": "Enclosure",
        "draft_shield": "Draft shield",
        "door_left": "Left sliding door",
        "door_right": "Right sliding door",
        "door_top": "Top sliding door",
        "pan": "Weighing pan",
        "screen": "Mass display",
        "power_key": "Power",
        "tare_key": "Tare",
        "zero_key": "Zero",
        "vessel": "Sample vessel",
        "mass_5g": "5 g fixture",
        "mass_20g": "20 g fixture",
        "overload": "Overload fixture",
    }
    components = [
        {"id": k, "label": label, "parent": "instrument"} for k, label in labels.items()
    ]
    for c in components:
        if c["id"].startswith("door_"):
            c["control"] = {
                "kind": "toggle",
                "action": "set_door",
                "params": {"door": c["id"][5:]},
                "parameter": "opening_m",
                "state": c["id"] + "_m",
                "closed": 0,
                "open": p["door_travel_m"],
            }
        elif c["id"] in ("tare_key", "zero_key", "power_key"):
            c["control"] = {
                "kind": "command",
                "action": c["id"].replace("_key", ""),
                "params": {},
            }
    return {
        "schema_version": 1,
        "model_id": p["id"],
        "dimensions_m": [
            p["dimensions_m"][0],
            p["dimensions_m"][2],
            p["dimensions_m"][1],
        ],
        "components": components,
        "joints": [
            {
                "id": side + "_slide",
                "component": "door_" + side,
                "kind": "prismatic",
                "axis": [0, 0, -1],
                "min": 0,
                "max": p["door_travel_m"],
                "state": "door_" + side + "_m",
            }
            for side in ("left", "right", "top")
        ],
        "readouts": [
            {"component": "screen", "state": "mass_g", "digits": 4, "unit": "g"}
        ],
        "visibility": [{"component": "vessel", "state": "vessel_visible"}]
        + [
            {"component": name, "state": name + "_visible"}
            for name in ("mass_5g", "mass_20g", "overload")
        ],
        "placements": [
            {
                "id": "pan",
                "component": "pan",
                "position": [0, p["pan_height_m"], -0.041],
                "radius_m": p["pan_radius_m"],
            }
        ],
    }


def inject_graph(path, graph):
    data = path.read_bytes()
    length = struct.unpack_from("<I", data, 12)[0]
    gltf = json.loads(data[20 : 20 + length])
    node = next(
        n
        for n in gltf["nodes"]
        if n.get("extras", {}).get("opensdlComponent") == "instrument"
    )
    node.setdefault("extras", {})["opensdlModel"] = graph
    encoded = json.dumps(gltf, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    binary = data[20 + length :]
    path.write_bytes(
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
    return gltf


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--proofs", type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    if bpy.app.version[:3] != (4, 5, 3):
        raise RuntimeError("Use pinned Blender 4.5.3")
    p = json.loads((SOURCE / "model.yaml").read_text())
    out = args.output.resolve()
    args.proofs.mkdir(parents=True, exist_ok=True)
    if out.exists():
        raise RuntimeError("Output must be a new directory")
    (out / "source").mkdir(parents=True)
    root, parts = create_model(p)
    from origin import mark

    mark(parts["body"], p)
    graph = model_graph(p)
    bpy.ops.object.select_all(action="DESELECT")
    for obj in root.children_recursive + [root]:
        obj.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=str(out / "preview.glb"),
        export_format="GLB",
        use_selection=True,
        export_extras=True,
        export_yup=True,
        export_animations=False,
    )
    gltf = inject_graph(out / "preview.glb", graph)
    bpy.ops.wm.usd_export(
        filepath=str(out / "source/model.usda"),
        selected_objects_only=True,
        export_textures=False,
        export_materials=True,
    )
    usd = out / "source/model.usda"
    usd.write_text(usd.read_text().rstrip() + "\n")
    (out / "source/model.graph.json").write_text(json.dumps(graph, indent=2) + "\n")
    (out / "LICENSE").write_text(
        f"{p['name']} / {p['id']} / {p['version']}\n"
        "Copyright 2026 OpenSDL / LiyanLabs. Attribution: OpenSDL / LiyanLabs.\n"
        "License: CC BY-NC 4.0. Source: https://github.com/ScienceOL/OpenSDL/tree/main/assets/instruments/analytical-balance\n"
        "Earlier edition 1.0.1 remains available under its existing MIT grant.\n"
        "License URL: https://creativecommons.org/licenses/by-nc/4.0/\n\n"
        + (REPO / "licenses/CC-BY-NC-4.0.txt").read_text()
    )
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "balance_package", Path(__file__).with_name("package.py")
    )
    package = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(package)
    package.write_config(out, p)
    metrics = {
        "preview_bytes": (out / "preview.glb").stat().st_size,
        "triangles": sum(
            gltf["accessors"][pr["indices"]]["count"] // 3
            for m in gltf["meshes"]
            for pr in m["primitives"]
        ),
        "model_json_bytes": len(json.dumps(graph, separators=(",", ":")).encode()),
    }
    (out / "validation.json").write_text(json.dumps(metrics, indent=2) + "\n")
    sources = [
        SOURCE / "model.yaml",
        SOURCE / "SOURCE.md",
        Path(__file__),
        Path(__file__).with_name("geometry.py"),
        Path(__file__).with_name("package.py"),
        Path(__file__).with_name("origin.py"),
    ]
    provenance = {
        "model": p["id"],
        "license": p["license"],
        "creator": "OpenSDL / LiyanLabs",
        "evidence": p["evidence"],
        "blender": bpy.app.version_string,
        "build_hash": bpy.app.build_hash.decode(),
        "inputs": {
            str(f.relative_to(REPO)): hashlib.sha256(f.read_bytes()).hexdigest()
            for f in sources
        },
    }
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    scene = studio()
    for name in ("mass_5g", "mass_20g", "overload"):
        parts[name].hide_render = True
        for child in parts[name].children_recursive:
            child.hide_render = True
    for name, opening in (
        ("closed", 0),
        ("half-open", p["door_travel_m"] / 2),
        ("open", p["door_travel_m"]),
    ):
        for side in ("left", "right", "top"):
            parts["door_" + side].location.y = opening
        scene.render.filepath = str(
            out / "thumbnail.png" if name == "closed" else args.proofs / (name + ".png")
        )
        bpy.ops.render.render(write_still=True)
    if metrics["triangles"] > 30000 or metrics["preview_bytes"] > 4 * 1024 * 1024:
        raise RuntimeError(f"Model exceeds geometry budget: {metrics}")
    print("MODEL_METRICS", json.dumps(metrics), flush=True)


if __name__ == "__main__":
    main()
