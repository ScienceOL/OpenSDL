# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original VF100 receiver, open vacuum sidearm, perforated funnel and collet seal."""
import math
import bpy
from mathutils import Vector
from geometry import box, cylinder, label, vessel, bore_world
from bench_helpers import assembly, enclosure, front_screen, graph
from author import joint, command
from tubing import smooth_path, sweep


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "pump",
            "funnel",
            "filter",
            "source_liquid",
            "receiver_liquid",
            "seal",
            "vent",
            "screen",
            "source_screen",
            "receiver_screen",
            "start_key",
            "stop_key",
        )
    )
    body = parts["body"]
    box(body, "Kit base", (0, 0, 0.004), (0.36, 0.28, 0.008), m["trim"], 0.004)
    parts["pump"].location = (0.105, 0.025, 0.008)
    enclosure(parts["pump"], m, 0.100, 0.140, 0.130)
    receiver = vessel(
        body,
        m["glass"],
        dict(radius_m=0.045, height_m=0.140, wall_m=0.003, bottom_m=0.004),
    )
    receiver.location = (-0.065, 0.020, 0.008)
    bore_world(receiver, (-0.020, 0.020, 0.120), 0.005, 0.025, "X")
    sweep(
        body,
        "Hollow fused vacuum sidearm",
        [Vector((x, 0.020, 0.120)) for x in [-0.023, 0.010]],
        0.006,
        m["glass"],
        0.004,
    )
    hose = smooth_path(
        [
            (0.010, 0.020, 0.120),
            (0.035, 0.020, 0.120),
            (0.060, 0.050, 0.180),
            (0.105, 0.070, 0.180),
            (0.105, 0.070, 0.142),
        ]
    )
    sweep(body, "Continuous vacuum hose", hose, 0.006, m["rubber"], 0.004)
    stopper = cylinder(
        body,
        "Receiver stopper",
        (-0.065, 0.020, 0.153),
        0.046,
        0.010,
        m["rubber"],
        vertices=48,
    )
    bore_world(stopper, (-0.065, 0.020, 0.153), 0.008, 0.030)
    # The removable stem is open at both ends and meets the funnel drain.
    stem = [Vector((-0.065, 0.020, z)) for z in [0.142, 0.236]]
    sweep(parts["funnel"], "Open funnel stem", stem, 0.008, m["white"], 0.006)
    bowl = vessel(
        parts["funnel"],
        m["white"],
        dict(radius_m=0.040, height_m=0.060, wall_m=0.002, bottom_m=0.002),
    )
    bowl.location = (-0.065, 0.020, 0.234)
    bore_world(bowl, (-0.065, 0.020, 0.235), 0.006, 0.020)
    sweep(
        parts["funnel"],
        "Support seating ring",
        [Vector((-0.065, 0.020, z)) for z in [0.236, 0.238]],
        0.038,
        m["white"],
        0.031,
    )
    sweep(
        body,
        "Pump hose fitting",
        [Vector((0.105, 0.070, z)) for z in [0.130, 0.150]],
        0.006,
        m["steel"],
        0.004,
    )
    support = cylinder(
        parts["funnel"],
        "Perforated support",
        (-0.065, 0.020, 0.239),
        0.035,
        0.002,
        m["steel"],
        vertices=48,
    )
    for x, y in [(0, 0)] + [
        (r * math.cos(i * math.tau / n), r * math.sin(i * math.tau / n))
        for r, n in [(0.012, 6), (0.025, 12)]
        for i in range(n)
    ]:
        bore_world(support, (-0.065 + x, 0.020 + y, 0.239), 0.0015, 0.012)
    cylinder(
        parts["filter"],
        "Synthetic filter paper",
        (-0.065, 0.020, 0.2405),
        0.035,
        0.001,
        m["white"],
        vertices=48,
        bevel=0,
    )
    parts["filter"].parent = parts["funnel"]
    parts["source_liquid"].parent = parts["funnel"]
    parts["source_liquid"].location = (-0.065, 0.020, 0.241)
    height = p["source_ml"] * 1e-6 / (math.pi * 0.035**2)
    cylinder(
        parts["source_liquid"],
        "Synthetic source liquid",
        (0, 0, height / 2),
        0.035,
        height,
        m["water"],
        vertices=64,
        bevel=0,
    )
    parts["receiver_liquid"].location = (-0.065, 0.020, 0.012)
    height = p["receiver_capacity_ml"] * 1e-6 / (math.pi * 0.041**2)
    cylinder(
        parts["receiver_liquid"],
        "Receiver liquid",
        (0, 0, height / 2),
        0.041,
        height,
        m["water"],
        vertices=64,
        bevel=0,
    )
    parts["seal"].location = (-0.065, 0.020, 0.161)
    sweep(
        parts["seal"],
        "Seal collet",
        [Vector((0, 0, z)) for z in [-0.003, 0.003]],
        0.014,
        m["blue"],
        0.008,
    )
    for x in (-0.022, 0.022):
        box(
            parts["seal"],
            "Seal grip",
            (x, 0, 0),
            (0.022, 0.008, 0.006),
            m["blue"],
            0.001,
        )
    parts["vent"].location = (0.140, 0.095, 0.140)
    cylinder(
        parts["vent"], "Vent spindle", (0, 0, 0), 0.005, 0.014, m["steel"], vertices=24
    )
    box(
        parts["vent"],
        "Vent lever",
        (0.012, 0, 0.010),
        (0.035, 0.008, 0.006),
        m["blue"],
        0.001,
    )
    for name, x in [
        ("screen", 0.105),
        ("source_screen", 0.080),
        ("receiver_screen", 0.130),
    ]:
        front_screen(
            parts[name],
            m,
            (x, -0.0455, 0.101 if name == "screen" else 0.068),
            0.078 if name == "screen" else 0.040,
            0.019,
        )
    for name, x in [("start_key", 0.080), ("stop_key", 0.130)]:
        box(
            parts[name],
            "Vacuum key",
            (x, -0.047, 0.034),
            (0.038, 0.004, 0.020),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(body, "VF 100", (-0.065, -0.026, 0.075), 0.008, m["trim"])
    return root, parts


def model_graph(p, parts):
    result = graph(
        p,
        parts,
        joints=[
            joint("seal", "seal_rad", [0, 1, 0], 0, math.pi / 3, "revolute"),
            joint("vent", "vent_rad", [0, 1, 0], 0, math.pi / 2, "revolute"),
        ],
        readouts=[
            ("screen", "pressure_kpa", 1, "kPa"),
            ("source_screen", "source_ml", 1, "mL"),
            ("receiver_screen", "receiver_ml", 1, "mL"),
        ],
        controls={
            "seal": command("seal"),
            "vent": command("vent"),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[
            ("funnel", "funnel_visible"),
            ("filter", "filter_visible"),
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
