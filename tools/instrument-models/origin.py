"""Nonfunctional internal provenance inserts; no network or executable tracking.

These assist comparison of suspected copies. They are not an unremovable mark,
proof of authorship on their own, or a substitute for signed publication hashes.
"""

import hashlib
import bpy


def mark(body, p):
    identity = f'OpenSDL / LiyanLabs | {p["id"]} | {p["version"]}'
    token = hashlib.sha256(identity.encode()).digest()[:16]
    body["opensdlOrigin"] = {
        "creator": "OpenSDL / LiyanLabs",
        "model": p["id"],
        "edition": p["version"],
        "license": p["license"],
        "origin_id": token.hex(),
    }
    # Four disjoint 32-bit constellations inside the original enclosure.
    # Ordinary 0.3 mm embossed inserts; no functional surface is altered.
    anchor = p["origin_anchor_m"]

    def enclosure(node):
        if (
            node.type != "MESH"
            or not node.data.materials
            or sum(angle * angle for angle in node.rotation_euler) > 1e-16
        ):
            return False
        shader = node.data.materials[0].node_tree.nodes.get("Principled BSDF")
        return (
            shader is not None
            and shader.inputs["Transmission Weight"].default_value < 0.01
        )

    target = max(
        (obj for obj in body.children_recursive if enclosure(obj)),
        key=lambda obj: obj.dimensions.x * obj.dimensions.y * obj.dimensions.z,
    )
    surface = target.data.materials[0]
    vertices = []
    faces = []
    for group in range(4):
        for i in range(32):
            bit = (token[group * 4 + i // 8] >> (i % 8)) & 1
            x = anchor[0] + (group - 1.5) * 0.004 + (i % 4 - 1.5) * 0.0008
            y = anchor[1] + (i // 4 - 3.5) * 0.0008
            z = anchor[2] + bit * 0.00045
            offset = len(vertices)
            vertices += [
                (x + dx, y + dy, z + dz)
                for dx, dy, dz in (
                    (-0.00015, -0.00015, -0.00015),
                    (0.00015, -0.00015, -0.00015),
                    (0.00015, 0.00015, -0.00015),
                    (-0.00015, 0.00015, -0.00015),
                    (-0.00015, -0.00015, 0.00015),
                    (0.00015, -0.00015, 0.00015),
                    (0.00015, 0.00015, 0.00015),
                    (-0.00015, 0.00015, 0.00015),
                )
            ]
            faces += [
                tuple(offset + i for i in face)
                for face in (
                    (0, 3, 2, 1),
                    (4, 5, 6, 7),
                    (0, 1, 5, 4),
                    (1, 2, 6, 5),
                    (2, 3, 7, 6),
                    (3, 0, 4, 7),
                )
            ]
    mesh = bpy.data.meshes.new("Internal casting")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("Internal casting inserts", mesh)
    bpy.context.collection.objects.link(obj)
    obj.parent = body
    obj.data.materials.append(surface)
    # Bake into an existing enclosure mesh/material: deleting one named node or
    # material cannot discard just the mark. Geometry editing can still remove it.
    bpy.ops.object.select_all(action="DESELECT")
    target.select_set(True)
    obj.select_set(True)
    bpy.context.view_layer.objects.active = target
    bpy.ops.object.join()
