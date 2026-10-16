"""Original metre-scale instrument authoring helpers for pinned Blender 4.5.3."""

import math

import bpy
from mathutils import Vector


def material(name, color, metallic=0.0, roughness=0.35, transmission=0.0):
    value = bpy.data.materials.new(name)
    value.use_nodes = True
    shader = value.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*color, 1)
    shader.inputs["Metallic"].default_value = metallic
    shader.inputs["Roughness"].default_value = roughness
    shader.inputs["Transmission Weight"].default_value = transmission
    shader.inputs["IOR"].default_value = 1.46
    return value


def component(identifier, parent=None):
    obj = bpy.data.objects.new(identifier, None)
    obj["opensdlComponent"] = identifier
    bpy.context.collection.objects.link(obj)
    obj.parent = parent
    return obj


def finish(obj, parent, surface, bevel=0):
    obj.parent = parent
    obj.data.materials.append(surface)
    if bevel:
        modifier = obj.modifiers.new("Machined edge", "BEVEL")
        modifier.width = bevel
        modifier.segments = 3
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    if obj.type == "MESH":
        for polygon in obj.data.polygons:
            polygon.use_smooth = True
        normal = obj.modifiers.new("Surface normals", "WEIGHTED_NORMAL")
        normal.keep_sharp = True
    return obj


def box(parent, name, position, dimensions, surface, bevel=0.001):
    bpy.ops.mesh.primitive_cube_add(size=1, location=position)
    obj = bpy.context.object
    obj.name = name
    obj.dimensions = dimensions
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return finish(obj, parent, surface, bevel)


def cylinder(
    parent, name, position, radius, depth, surface, axis="Z", vertices=48, bevel=None
):
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices, radius=radius, depth=depth, location=position
    )
    obj = bpy.context.object
    obj.name = name
    if axis == "X":
        obj.rotation_euler.y = math.pi / 2
    elif axis == "Y":
        obj.rotation_euler.x = math.pi / 2
    return finish(
        obj, parent, surface, min(0.0008, depth / 5) if bevel is None else bevel
    )


def label(parent, text, position, size, surface, rotation=(math.pi / 2, 0, 0)):
    curve = bpy.data.curves.new("Etched legend", "FONT")
    curve.body = text
    curve.size = size
    curve.align_x = "CENTER"
    curve.align_y = "CENTER"
    curve.resolution_u = 3
    obj = bpy.data.objects.new("Legend", curve)
    bpy.context.collection.objects.link(obj)
    obj.parent = parent
    obj.location = position
    obj.rotation_euler = rotation
    obj.data.materials.append(surface)
    return obj


def vessel(parent, surface, fixture):
    """Closed thin glass wall and bottom, not a transparent solid cylinder."""
    r, h, wall, bottom = (
        fixture[k] for k in ("radius_m", "height_m", "wall_m", "bottom_m")
    )
    rings = [(r, 0), (r, h), (r - wall, h), (r - wall, bottom)]
    n = 48
    verts = [
        (radius * math.cos(i * math.tau / n), radius * math.sin(i * math.tau / n), z)
        for radius, z in rings
        for i in range(n)
    ]
    lower = len(verts)
    upper = lower + 1
    verts.extend([(0, 0, 0), (0, 0, bottom)])
    faces = [
        (j * n + i, j * n + (i + 1) % n, (j + 1) * n + (i + 1) % n, (j + 1) * n + i)
        for j in range(3)
        for i in range(n)
    ]
    faces += [(lower, (i + 1) % n, i) for i in range(n)]
    faces += [(upper, 3 * n + i, 3 * n + (i + 1) % n) for i in range(n)]
    mesh = bpy.data.meshes.new("Vessel wall")
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("Sample vessel", mesh)
    bpy.context.collection.objects.link(obj)
    return finish(obj, parent, surface)


def studio():
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 48
    scene.render.resolution_x = 1000
    scene.render.resolution_y = 900
    scene.render.resolution_percentage = 100
    scene.world.color = (0.22, 0.22, 0.22)
    for name, pos, energy, size in [
        ("Key", (0.35, -0.45, 0.7), 28, 0.55),
        ("Rim", (-0.4, 0.3, 0.5), 35, 0.45),
        ("Fill", (-0.35, -0.3, 0.4), 12, 0.4),
    ]:
        light = bpy.data.lights.new(name, "AREA")
        light.energy, light.shape, light.size = energy, "DISK", size
        obj = bpy.data.objects.new(name, light)
        bpy.context.collection.objects.link(obj)
        obj.location = pos
        obj.rotation_euler = (
            (Vector((0, 0, 0.15)) - obj.location).to_track_quat("-Z", "Y").to_euler()
        )
    camera = bpy.data.cameras.new("Inspection camera")
    obj = bpy.data.objects.new("Inspection camera", camera)
    bpy.context.collection.objects.link(obj)
    obj.location = (0.52, -0.71, 0.45)
    obj.rotation_euler = (
        (Vector((0, 0, 0.17)) - obj.location).to_track_quat("-Z", "Y").to_euler()
    )
    camera.type, camera.ortho_scale = "ORTHO", 0.57
    scene.camera = obj
    scene.view_settings.view_transform = "AgX"
    scene.render.image_settings.file_format = "PNG"
    return scene
