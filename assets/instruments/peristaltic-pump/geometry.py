# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original PP56: hinged head, three freely rolling followers and continuous hollow tubing."""
import math
import bpy
from mathutils import Vector
from geometry import box, cylinder, finish, label, vessel
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial


def smooth_path(points):
    points = [Vector(v) for v in points]
    result = []
    for i in range(len(points) - 1):
        a, b, c, d = (
            points[max(0, i - 1)],
            points[i],
            points[i + 1],
            points[min(len(points) - 1, i + 2)],
        )
        for j in range(6):
            t = j / 6
            result.append(
                (
                    2 * b
                    + (-a + c) * t
                    + (2 * a - 5 * b + 4 * c - d) * t * t
                    + (-a + 3 * b - 3 * c + d) * t * t * t
                )
                * 0.5
            )
    return result + [points[-1]]


def sweep(parent, name, points, outer, surface, inner=None, sides=16):
    vertices = []
    rings = [outer] if inner is None else [outer, inner]
    for radius in rings:
        for i, point in enumerate(points):
            tangent = (
                points[min(i + 1, len(points) - 1)] - points[max(0, i - 1)]
            ).normalized()
            normal = tangent.cross(Vector((0, 1, 0))).normalized()
            binormal = tangent.cross(normal)
            vertices += [
                point
                + radius
                * (
                    normal * math.cos(k * math.tau / sides)
                    + binormal * math.sin(k * math.tau / sides)
                )
                for k in range(sides)
            ]
    faces = []
    stride = len(points) * sides
    for layer in range(len(rings)):
        offset = layer * stride
        for i in range(len(points) - 1):
            for k in range(sides):
                face = (
                    offset + i * sides + k,
                    offset + i * sides + (k + 1) % sides,
                    offset + (i + 1) * sides + (k + 1) % sides,
                    offset + (i + 1) * sides + k,
                )
                faces.append(face if layer == 0 else tuple(reversed(face)))
    for i in (0, len(points) - 1):
        if inner is None:
            centre = len(vertices)
            vertices.append(points[i])
            for k in range(sides):
                face = (centre, i * sides + k, i * sides + (k + 1) % sides)
                faces.append(tuple(reversed(face)) if i == 0 else face)
        else:
            for k in range(sides):
                face = (
                    i * sides + k,
                    stride + i * sides + k,
                    stride + i * sides + (k + 1) % sides,
                    i * sides + (k + 1) % sides,
                )
                faces.append(face if i == 0 else tuple(reversed(face)))
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return finish(obj, parent, surface)


def create_model(p):
    names = (
        "body",
        "housing",
        "rotor",
        "lid",
        "tubing",
        "liquid",
        "source_vessel",
        "receiver_vessel",
        "source_liquid",
        "receiver_liquid",
        "flow_dial",
        "screen",
        "source_screen",
        "receiver_screen",
        "start_key",
        "stop_key",
        "prime_key",
    )
    root, parts, m = assembly(names + tuple(f"roller_{i}" for i in range(3)))
    body = parts["body"]
    box(
        body,
        "Kit support plate",
        (0, 0, 0.004),
        (0.440, 0.250, 0.008),
        m["trim"],
        0.005,
    )
    parts["housing"].location.z = 0.008
    enclosure(parts["housing"], m, 0.220, 0.160, 0.300)
    h = p["head_height_m"]
    cylinder(
        body,
        "Drive bushing",
        (0, -0.080, h),
        0.020,
        0.025,
        m["steel"],
        axis="Y",
        vertices=40,
    )
    cylinder(
        body,
        "Head backplate",
        (0, -0.090, h),
        0.080,
        0.018,
        m["steel"],
        axis="Y",
        vertices=64,
    )
    parts["rotor"].location = (0, -0.109, h)
    cylinder(
        parts["rotor"],
        "Rotor carrier",
        (0, 0, 0),
        0.045,
        0.006,
        m["blue"],
        axis="Y",
        vertices=48,
    )
    cylinder(
        parts["rotor"],
        "Rotor hub",
        (0, -0.005, 0),
        0.012,
        0.014,
        m["steel"],
        axis="Y",
        vertices=32,
    )
    for i in range(3):
        roller = parts[f"roller_{i}"]
        roller.parent = parts["rotor"]
        angle = i * math.tau / 3
        roller.location = (
            p["roller_orbit_m"] * math.cos(angle),
            0,
            p["roller_orbit_m"] * math.sin(angle),
        )
        cylinder(
            roller,
            "Follower roller",
            (0, 0, 0),
            p["roller_radius_m"],
            0.016,
            m["white"],
            axis="Y",
            vertices=40,
        )
        cylinder(
            roller,
            "Bearing cap",
            (0, -0.009, 0),
            0.006,
            0.003,
            m["steel"],
            axis="Y",
            vertices=24,
        )
        box(
            roller,
            "Follower index",
            (0, -0.011, 0.009),
            (0.002, 0.001, 0.007),
            m["trim"],
            0.0003,
        )
    arc_radius = p["roller_orbit_m"] + p["roller_radius_m"] + p["tube_outer_radius_m"]
    points = [
        (-0.17, -0.035, 0.030),
        (-0.17, -0.035, 0.190),
        (-0.12, -0.109, h - 0.068),
    ]
    points += [
        (arc_radius * math.cos(t), -0.109, h + arc_radius * math.sin(t))
        for t in [7 * math.pi / 6 - i * 4 * math.pi / 3 / 18 for i in range(19)]
    ]
    points += [(0.12, -0.109, h - 0.068), (0.17, -0.035, 0.190), (0.17, -0.035, 0.030)]
    points = smooth_path(points)
    length = sum((b - a).length for a, b in zip(points, points[1:]))
    # The same declared line volume drives the actual lumen rather than an unrelated decorative curve.
    area_factor = 16 * math.sin(math.tau / 16) / 2
    inner = math.sqrt(p["dead_volume_ml"] * 1e-6 / (area_factor * length))
    if not 0 < inner < p["tube_outer_radius_m"]:
        raise ValueError("line volume does not fit the declared tube wall")
    sweep(
        parts["tubing"],
        "Continuous hollow silicone tube",
        points,
        p["tube_outer_radius_m"],
        m["glass"],
        inner,
    )
    sweep(parts["liquid"], "Primed liquid column", points, inner, m["water"])
    race_radius = arc_radius + p["tube_outer_radius_m"] + 0.007
    race = [
        Vector((race_radius * math.cos(t), -0.109, h + race_radius * math.sin(t)))
        for t in [-math.pi / 18 + i * 10 * math.pi / 9 / 40 for i in range(41)]
    ]
    sweep(body, "Open-bottom stator race", race, 0.007, m["trim"], sides=12)
    parts["lid"].location = (-0.092, -0.130, h)
    cylinder(
        parts["lid"],
        "Clear head cover",
        (0.092, 0, 0),
        0.080,
        0.003,
        m["glass"],
        axis="Y",
        vertices=64,
    )
    bpy.ops.mesh.primitive_torus_add(
        major_segments=64,
        minor_segments=8,
        location=(0.092, 0, 0),
        major_radius=0.082,
        minor_radius=0.003,
    )
    rim = bpy.context.object
    rim.rotation_euler.x = math.pi / 2
    finish(rim, parts["lid"], m["trim"])
    cylinder(
        parts["lid"], "Cover hinge", (0, 0, 0), 0.005, 0.112, m["steel"], vertices=24
    )
    for z in (h - 0.048, h + 0.048):
        box(
            body,
            "Hinge bracket",
            (-0.092, -0.105, z),
            (0.014, 0.050, 0.012),
            m["steel"],
            0.002,
        )
    for name, x in [("source_vessel", -0.170), ("receiver_vessel", 0.170)]:
        glass = vessel(
            parts[name],
            m["glass"],
            {"radius_m": 0.027, "height_m": 0.140, "wall_m": 0.0015, "bottom_m": 0.002},
        )
        glass.location = (x, -0.035, 0.008)
        label(
            parts[name], "IN" if x < 0 else "OUT", (x, -0.0625, 0.098), 0.007, m["trim"]
        )
    for name, x, volume in [
        ("source_liquid", -0.170, p["source_ml"]),
        ("receiver_liquid", 0.170, p["receiver_capacity_ml"]),
    ]:
        parts[name].location = (x, -0.035, 0.0105)
        height = volume * 1e-6 / (math.pi * 0.0245**2)
        cylinder(
            parts[name],
            "Reservoir liquid",
            (0, 0, height / 2),
            0.0245,
            height,
            m["water"],
            vertices=64,
            bevel=0,
        )
    for name, x in [
        ("screen", -0.070),
        ("source_screen", 0),
        ("receiver_screen", 0.070),
    ]:
        front_screen(parts[name], m, (x, -0.0805, 0.289), 0.052, 0.017)
    knob(parts["flow_dial"], m, (0.084, -0.085, 0.254), radius=0.011)
    for name, x in [("start_key", -0.070), ("stop_key", -0.032), ("prime_key", 0.014)]:
        box(
            parts[name],
            "Pump key",
            (x, -0.082, 0.269),
            (0.030, 0.003, 0.013),
            m["blue"] if name != "stop_key" else m["trim"],
            0.002,
        )
    label(parts["housing"], "PP 56", (0, -0.0805, 0.298), 0.006, m["trim"])
    return root, parts


def model_graph(p, parts):
    result = graph(
        p,
        parts,
        joints=[
            joint("rotor", "rotor_rad", [0, 0, -1], 0, math.tau, "continuous"),
            *[
                joint(
                    f"roller_{i}", "roller_rad", [0, 0, -1], 0, math.tau, "continuous"
                )
                for i in range(3)
            ],
            joint("lid", "lid_rad", [0, -1, 0], 0, p["opening_rad"], "revolute"),
            joint(
                "flow_dial", "flow_dial_rad", [0, 0, -1], 0, math.tau * 0.75, "revolute"
            ),
        ],
        readouts=[
            ("screen", "flow_ml_min", 1, "mL/min"),
            ("source_screen", "source_ml", 1, "mL"),
            ("receiver_screen", "receiver_ml", 1, "mL"),
        ],
        controls={
            "lid": command("lid"),
            "flow_dial": dial(
                "set_flow",
                "flow_ml_min",
                "target_flow_ml_min",
                0,
                p["max_rpm"] * p["displacement_ml_rev"],
                2,
            ),
            "start_key": command("start"),
            "stop_key": command("stop"),
            "prime_key": command("prime"),
        },
        visibility=[
            ("tubing", "tube_visible"),
            ("liquid", "liquid_visible"),
            ("source_liquid", "source_liquid_visible"),
            ("receiver_liquid", "receiver_liquid_visible"),
        ],
    )
    for name, state in [
        ("source_liquid", "source_fraction"),
        ("receiver_liquid", "receiver_fraction"),
    ]:
        next(c for c in result["components"] if c["id"] == name)["scale"] = {
            "axis": "y",
            "state": state,
            "min": 0,
            "max": 1,
        }
    return result
