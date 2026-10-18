# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original OH1000 stand, sliding motor carriage and immersed impeller."""
import math
from geometry import box, cylinder, label, vessel, bore_world
from bench_helpers import assembly, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "carriage",
            "shaft",
            "clamp",
            "vessel",
            "speed_dial",
            "screen",
            "start_key",
            "stop_key",
        )
    )
    body = parts["body"]
    box(body, "Stand base", (0, 0, 0.015), (0.260, 0.280, 0.022), m["trim"], 0.008)
    for x in (-0.095, 0.095):
        for y in (-0.100, 0.100):
            cylinder(
                body,
                "Isolator foot",
                (x, y, 0.004),
                0.013,
                0.008,
                m["rubber"],
                vertices=24,
            )
    cylinder(
        body, "Stand column", (0, 0.095, 0.280), 0.010, 0.510, m["steel"], vertices=48
    )
    cylinder(
        body, "Column collar", (0, 0.095, 0.040), 0.022, 0.024, m["steel"], vertices=32
    )
    parts["carriage"].location = (0, 0.060, 0.300)
    box(
        parts["carriage"],
        "Motor housing",
        (0, -0.060, 0.050),
        (0.100, 0.110, 0.100),
        m["shell"],
        0.008,
    )
    collar = box(
        parts["carriage"],
        "Sliding stand collar",
        (0, 0.035, 0),
        (0.052, 0.040, 0.040),
        m["trim"],
        0.004,
    )
    bore_world(collar, (0, 0.095, 0.300), 0.0105, 0.060)
    arm = box(
        parts["carriage"],
        "Support arm",
        (0, 0.0, 0.012),
        (0.032, 0.070, 0.020),
        m["steel"],
        0.003,
    )
    bore_world(arm, (0, 0.095, 0.312), 0.0105, 0.040)
    for i in range(4):
        box(
            parts["carriage"],
            "Motor vent",
            (0.0502, -0.065 + i * 0.012, 0.070),
            (0.001, 0.006, 0.020),
            m["trim"],
            0.0002,
        )
    parts["shaft"].parent = parts["carriage"]
    parts["shaft"].location = (0, -0.060, 0)
    cylinder(
        parts["shaft"], "Chuck", (0, 0, -0.010), 0.012, 0.020, m["trim"], vertices=48
    )
    cylinder(
        parts["shaft"],
        "Stainless shaft",
        (0, 0, -0.095),
        0.003,
        0.170,
        m["steel"],
        vertices=32,
    )
    for angle in (0, math.pi / 2):
        blade = box(
            parts["shaft"],
            "Paddle blade",
            (0, 0, -0.170),
            (0.060, 0.012, 0.003),
            m["steel"],
            0.0008,
        )
        blade.rotation_euler.z = angle
    parts["clamp"].parent = parts["carriage"]
    parts["clamp"].location = (0.044, 0.035, 0)
    cylinder(
        parts["clamp"],
        "Stand locking screw",
        (0, 0, 0),
        0.005,
        0.040,
        m["steel"],
        axis="X",
        vertices=24,
    )
    cylinder(
        parts["clamp"],
        "Clamp handwheel",
        (0.022, 0, 0),
        0.019,
        0.012,
        m["blue"],
        axis="X",
        vertices=40,
    )
    box(
        parts["clamp"],
        "Handwheel index",
        (0.0285, 0, 0.014),
        (0.001, 0.003, 0.006),
        m["white"],
        0.0003,
    )
    parts["vessel"].location = (0, 0, 0.026)
    vessel(
        parts["vessel"],
        m["glass"],
        {"radius_m": 0.044, "height_m": 0.145, "wall_m": 0.002, "bottom_m": 0.003},
    )
    cylinder(
        parts["vessel"],
        "Synthetic liquid fixture",
        (0, 0, 0.062),
        0.041,
        0.116,
        m["water"],
        vertices=64,
    )
    for name in ("screen", "speed_dial", "start_key", "stop_key"):
        parts[name].parent = parts["carriage"]
    front_screen(parts["screen"], m, (0, -0.1155, 0.070), 0.064, 0.019)
    knob(parts["speed_dial"], m, (-0.030, -0.120, 0.026), radius=0.011)
    for name, x in [("start_key", 0.005), ("stop_key", 0.030)]:
        box(
            parts[name],
            "Motor key",
            (x, -0.117, 0.026),
            (0.019, 0.003, 0.014),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(parts["carriage"], "OH 1000", (0, -0.1155, 0.091), 0.006, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("carriage", "carriage_m", [0, 1, 0], 0, p["lift_m"], "prismatic"),
            joint("shaft", "shaft_rad", [0, 1, 0], 0, math.tau, "continuous"),
            joint("clamp", "clamp_rad", [1, 0, 0], 0, 0.6, "revolute"),
            joint(
                "speed_dial",
                "speed_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
        ],
        readouts=[("screen", "rpm", 0, "rpm")],
        controls={
            "carriage": command("lift"),
            "clamp": command("clamp"),
            "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 50),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("vessel", "vessel_visible")],
    )
