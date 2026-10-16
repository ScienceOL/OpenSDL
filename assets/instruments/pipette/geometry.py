# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original pipette geometry and declarative bindings, metres, Blender Z-up."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "carrier_x",
            "carrier_z",
            "pipette_body",
            "source_vial",
            "receiver_vial",
            "source_liquid",
            "receiver_liquid",
            "plunger",
            "volume_dial",
            "tip",
            "liquid",
            "screen",
            "tip_key",
        )
    )
    body = parts["pipette_body"]
    cylinder(body, "Hand grip", (0, 0, 0.153), 0.014, 0.124, m["shell"], vertices=64)
    cylinder(body, "Nose cone", (0, 0, 0.070), 0.006, 0.045, m["trim"], vertices=48)
    box(body, "Finger hook", (0, 0.016, 0.194), (0.018, 0.025, 0.009), m["trim"], 0.003)
    parts["plunger"].location = (0, 0, 0.218)
    cylinder(
        parts["plunger"],
        "Plunger stem",
        (0, 0, 0.010),
        0.004,
        0.020,
        m["steel"],
        vertices=24,
    )
    cylinder(
        parts["plunger"],
        "Thumb pad",
        (0, 0, 0.024),
        0.012,
        0.008,
        m["blue"],
        vertices=48,
    )
    parts["volume_dial"].location = (0, 0, 0.207)
    cylinder(
        parts["volume_dial"],
        "Volume collar",
        (0, 0, 0),
        0.015,
        0.012,
        m["trim"],
        vertices=48,
    )
    box(
        parts["volume_dial"],
        "Collar index",
        (0, -0.0155, 0),
        (0.002, 0.001, 0.007),
        m["white"],
        0.0003,
    )
    # Hollow tapered disposable tip, with explicit wall and open bore.
    rings = [(0.0015, 0.002), (0.0045, 0.049), (0.00125, 0.049), (0.00125, 0.002)]
    import bpy

    n = 40
    verts = [
        (r * math.cos(i * math.tau / n), r * math.sin(i * math.tau / n), z)
        for r, z in rings
        for i in range(n)
    ]
    faces = [
        (
            j * n + i,
            j * n + (i + 1) % n,
            ((j + 1) % 4) * n + (i + 1) % n,
            ((j + 1) % 4) * n + i,
        )
        for j in range(4)
        for i in range(n)
    ]
    mesh = bpy.data.meshes.new("Tip wall")
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("Disposable tip", mesh)
    bpy.context.collection.objects.link(obj)
    obj.parent = parts["tip"]
    obj.data.materials.append(m["glass"])
    parts["liquid"].location = (0, 0, 0.0025)
    height = p["max_ul"] * 1e-9 / (math.pi * 0.00120**2)
    cylinder(
        parts["liquid"],
        "Aspirated liquid",
        (0, 0, height / 2),
        0.0012,
        height,
        m["water"],
        vertices=64,
        bevel=0,
    )
    front_screen(parts["screen"], m, (0, -0.0142, 0.150), 0.021, 0.008)
    box(
        parts["tip_key"],
        "Eject lever",
        (0.017, 0, 0.130),
        (0.006, 0.010, 0.060),
        m["trim"],
        0.002,
    )
    label(body, "P 200", (0, -0.0143, 0.182), 0.004, m["trim"])
    # The pipette parks on an actual support, with its tip clear of the base.
    box(
        parts["body"],
        "Stand base",
        (0, 0.015, 0.004),
        (0.150, 0.100, 0.008),
        m["trim"],
        0.003,
    )
    cylinder(
        parts["body"],
        "Stand upright",
        (0, 0.040, 0.109),
        0.004,
        0.202,
        m["steel"],
        vertices=24,
    )
    box(
        parts["body"],
        "Hook support",
        (0, 0.025, 0.207),
        (0.030, 0.020, 0.005),
        m["trim"],
        0.002,
    )
    for name, x in (
        ("source_vial", p["source_x_m"]),
        ("receiver_vial", p["receiver_x_m"]),
    ):
        glass = vessel(
            parts[name],
            m["glass"],
            {"radius_m": 0.006, "height_m": 0.045, "wall_m": 0.001, "bottom_m": 0.002},
        )
        glass.location = (x, 0, 0.008)
    fill_height = p["reservoir_ul"] * 1e-9 / (math.pi * 0.0049**2)
    for name, x in (
        ("source_liquid", p["source_x_m"]),
        ("receiver_liquid", p["receiver_x_m"]),
    ):
        parts[name].location = (x, 0, 0.010)
        cylinder(
            parts[name],
            "Vial liquid",
            (0, 0, fill_height / 2),
            0.0049,
            fill_height,
            m["water"],
            vertices=64,
            bevel=0,
        )
    parts["carrier_z"].parent = parts["carrier_x"]
    for name in (
        "pipette_body",
        "plunger",
        "volume_dial",
        "tip",
        "liquid",
        "screen",
        "tip_key",
    ):
        parts[name].parent = parts["carrier_z"]
        parts[name].location.z += 0.020
    return root, parts


def model_graph(p, parts):
    result = graph(
        p,
        parts,
        joints=[
            joint(
                "carrier_x",
                "pipette_x_m",
                [1, 0, 0],
                p["source_x_m"],
                p["receiver_x_m"],
                kind="prismatic",
            ),
            joint(
                "carrier_z",
                "pipette_vertical_m",
                [0, 1, 0],
                -p["immersion_m"],
                0,
                kind="prismatic",
            ),
            joint(
                "plunger", "plunger_m", [0, -1, 0], 0, p["stroke_m"], kind="prismatic"
            ),
            joint(
                "volume_dial",
                "volume_dial_rad",
                [0, 1, 0],
                0,
                math.tau * 0.75,
                kind="revolute",
            ),
        ],
        readouts=[("screen", "volume_ul", 0, "uL")],
        controls={
            "plunger": command("stroke"),
            "body": {
                "kind": "command",
                "action": "move_to",
                "params": {"position": "park"},
            },
            "source_vial": {
                "kind": "command",
                "action": "move_to",
                "params": {"position": "source"},
            },
            "receiver_vial": {
                "kind": "command",
                "action": "move_to",
                "params": {"position": "receiver"},
            },
            "volume_dial": dial(
                "set_volume", "volume_ul", "volume_ul", p["min_ul"], p["max_ul"], 10
            ),
            "tip_key": command("tip"),
        },
        visibility=[
            ("tip", "tip_visible"),
            ("liquid", "liquid_visible"),
            ("source_liquid", "source_liquid_visible"),
            ("receiver_liquid", "receiver_liquid_visible"),
        ],
    )
    next(c for c in result["components"] if c["id"] == "liquid")["scale"] = {
        "axis": "y",
        "state": "fill_fraction",
        "min": 0,
        "max": 1,
    }
    for component, state in (
        ("source_liquid", "source_fraction"),
        ("receiver_liquid", "receiver_fraction"),
    ):
        next(c for c in result["components"] if c["id"] == component)["scale"] = {
            "axis": "y",
            "state": state,
            "min": 0,
            "max": 1,
        }
    return result
