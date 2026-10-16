"""Shared exporter for original instrument builders; all geometry remains in author modules."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys
import bpy
from geometry import studio


def dial(action, parameter, state, low, high, step):
    return {
        "kind": "dial",
        "action": action,
        "params": {},
        "parameter": parameter,
        "state": state,
        "min": low,
        "max": high,
        "step": step,
    }


def command(action):
    return {"kind": "command", "action": action, "params": {}}


def joint(component, state, axis, low, high, kind):
    return {
        "id": component + "_joint",
        "component": component,
        "kind": kind,
        "axis": axis,
        "min": low,
        "max": high,
        "state": state,
        **({"velocity_state": state + "_s"} if kind == "continuous" else {}),
    }


def screen(parent, position, dimensions, surface):
    bpy.ops.mesh.primitive_plane_add(size=1, location=position)
    obj = bpy.context.object
    obj.name = "Live LCD surface"
    obj.dimensions = (*dimensions, 0)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.parent = parent
    obj.data.materials.append(surface)
    return obj


def numeric(low, high):
    return {"type": "number", "minimum": low, "maximum": high}


def action(name, description, properties=None):
    properties = properties or {}
    return {
        "name": name,
        "description": description,
        "params": {
            "type": "object",
            "properties": properties,
            "required": list(properties),
            "additionalProperties": False,
        },
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


def build(source, create_model, model_graph, runtime, slug, role, *, preview=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--proofs", type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    if bpy.app.version[:3] != (4, 5, 3):
        raise RuntimeError("Use pinned Blender 4.5.3")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    (out / "source").mkdir()
    args.proofs.mkdir(parents=True, exist_ok=True)
    p = json.loads((source / "model.yaml").read_text())
    p = {**p, **p.get("runtime", {})}
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    root, parts = create_model(p)
    graph = model_graph(p, parts)
    from origin import mark

    mark(parts["body"], p)
    from optimize import consolidate

    consolidate(root)
    bpy.context.scene.unit_settings.system = "METRIC"
    bpy.context.scene.unit_settings.scale_length = 1.0
    bpy.context.view_layer.update()
    from mathutils import Vector

    points = [
        obj.matrix_world @ Vector(corner)
        for obj in root.children_recursive
        if obj.type == "MESH"
        for corner in obj.bound_box
    ]
    size = [max(v[i] for v in points) - min(v[i] for v in points) for i in range(3)]
    if any(
        abs(a - b) > max(0.0002, b * 0.005) for a, b in zip(size, p["dimensions_m"])
    ):
        raise RuntimeError(
            f'Authored bounds differ: actual={size}, declared={p["dimensions_m"]}'
        )
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
    repo = source.parents[2]
    (out / "LICENSE").write_text(
        f"{p['name']} / {p['id']} / {p['version']}\n"
        "Copyright 2026 OpenSDL / LiyanLabs.\n"
        "Attribution: OpenSDL / LiyanLabs. License: CC BY-NC 4.0.\n"
        f"Source: https://github.com/ScienceOL/OpenSDL/tree/main/assets/instruments/{slug}\n"
        "License URL: https://creativecommons.org/licenses/by-nc/4.0/\n"
        + p.get("license_history", "")
        + "\n"
        + (repo / "licenses/CC-BY-NC-4.0.txt").read_text()
    )
    actions, model, samples = runtime(p)
    world = {
        "world_id": p["world_id"],
        "engine": "kinematic",
        "tick_hz": 10,
        "seed": 0,
        "samples": samples,
        "devices": [
            {
                "id": p["device_id"],
                "device_type": "simulation." + model["kind"],
                "role": role,
                "description": p["name"],
                "actions": actions,
                "position": [0, 0, 0],
                "asset_ref": {
                    "namespace": "opensdl",
                    "name": slug,
                    "version": p["version"],
                },
                "model": model,
            }
        ],
    }
    manifest = {
        "apiVersion": "assets.opensdl.org/v1alpha1",
        "kind": "VirtualAsset",
        "metadata": {
            "name": slug,
            "displayName": p["name"],
            "version": p["version"],
            "description": p["evidence"],
            "tags": ["original", role],
            "license": {"spdx": p["license"], "file": "LICENSE"},
        },
        "spec": {
            "type": "instrument",
            "source": {"format": "openusd", "root": "source/model.usda"},
            "preview": {"model": "preview.glb", "thumbnail": "thumbnail.png"},
            "capabilities": ["visual", "simulation", "opensdl-device"],
            "bindings": {"deviceTypes": ["simulation." + model["kind"]]},
            "compatibility": {
                "runtimes": [
                    {"name": "opensdl", "versions": [">=0.4.0"], "status": "expected"}
                ]
            },
            "runtime": {"engine": "kinematic", "config": "source/world.yaml"},
        },
    }
    for name, value in [("asset.yaml", manifest), ("source/world.yaml", world)]:
        (out / name).write_text(json.dumps(value, indent=2) + "\n")
    metrics = {
        "preview_bytes": (out / "preview.glb").stat().st_size,
        "triangles": sum(
            gltf["accessors"][pr["indices"]]["count"] // 3
            for m in gltf["meshes"]
            for pr in m["primitives"]
        ),
        "draw_primitives": sum(len(m["primitives"]) for m in gltf["meshes"]),
        "model_json_bytes": len(json.dumps(graph, separators=(",", ":")).encode()),
        "measured_dimensions_m": size,
    }
    if metrics["triangles"] > 40000 or metrics["preview_bytes"] > 4 * 1024 * 1024:
        raise RuntimeError(f"Geometry budget exceeded: {metrics}")
    (out / "validation.json").write_text(json.dumps(metrics, indent=2) + "\n")
    inputs = [
        source / "model.yaml",
        source / "SOURCE.md",
        Path(create_model.__code__.co_filename),
        Path(__file__),
        Path(__file__).with_name("geometry.py"),
        Path(__file__).with_name("origin.py"),
    ]
    inputs += [
        Path(module.__file__).resolve()
        for module in list(sys.modules.values())
        if getattr(module, "__file__", None)
        and Path(module.__file__).resolve().parent == Path(__file__).resolve().parent
    ]
    inputs = list(dict.fromkeys(inputs))
    provenance = {
        "model": p["id"],
        "license": p["license"],
        "creator": "OpenSDL / LiyanLabs",
        "evidence": p["evidence"],
        "blender": bpy.app.version_string,
        "build_hash": bpy.app.build_hash.decode(),
        "inputs": {
            str(f.relative_to(repo)): hashlib.sha256(f.read_bytes()).hexdigest()
            for f in inputs
        },
    }
    (out / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    scene = studio()
    height = p["dimensions_m"][2]
    scene.camera.data.ortho_scale = max(p["dimensions_m"]) * 1.65
    from mathutils import Vector

    scene.camera.rotation_euler = (
        (Vector((0, 0, height * 0.45)) - scene.camera.location)
        .to_track_quat("-Z", "Y")
        .to_euler()
    )
    if preview is None:
        scene.render.filepath = str(out / "thumbnail.png")
        bpy.ops.render.render(write_still=True)
    else:
        preview(p, parts, scene, out, args.proofs)
    print("MODEL_METRICS", json.dumps(metrics), flush=True)
