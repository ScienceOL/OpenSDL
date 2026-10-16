# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original vortex-mixer geometry and declarative bindings, metres, Blender Z-up."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "eccentric",
            "cup",
            "tube",
            "speed_dial",
            "screen",
            "start_key",
            "stop_key",
        )
    )
    enclosure(parts["body"], m, 0.130, 0.165, 0.095)
    cylinder(parts["body"], "Cup bearing", (0, 0.020, 0.101), 0.030, 0.012, m["steel"])
    parts["eccentric"].location = (0, 0.020, 0.107)
    parts["cup"].parent = parts["eccentric"]
    parts["cup"].location = (p["orbit_m"], 0, 0)
    cylinder(
        parts["cup"],
        "Rubber cup",
        (0, 0, 0.013),
        0.027,
        0.026,
        m["rubber"],
        vertices=64,
    )
    cylinder(parts["cup"], "Cup socket", (0, 0, 0.027), 0.008, 0.003, m["trim"])
    parts["tube"].parent = parts["cup"]
    tube = vessel(
        parts["tube"],
        m["glass"],
        {"radius_m": 0.007, "height_m": 0.065, "wall_m": 0.001, "bottom_m": 0.0015},
    )
    tube.location = (0, 0, 0.0285)
    cylinder(
        parts["tube"],
        "Inert liquid fixture",
        (0, 0, 0.0477),
        0.006,
        0.0354,
        m["water"],
        vertices=64,
        bevel=0,
    )
    cylinder(
        parts["tube"],
        "Blue tube cap",
        (0, 0, 0.096),
        0.009,
        0.005,
        m["blue"],
        vertices=32,
    )
    knob(parts["speed_dial"], m, (-0.034, -0.086, 0.054))
    front_screen(parts["screen"], m, (0.021, -0.083, 0.064), 0.050, 0.019)
    box(
        parts["start_key"],
        "Start key",
        (0.015, -0.083, 0.030),
        (0.025, 0.003, 0.014),
        m["blue"],
        0.002,
    )
    box(
        parts["stop_key"],
        "Stop key",
        (0.045, -0.083, 0.030),
        (0.025, 0.003, 0.014),
        m["trim"],
        0.002,
    )
    label(parts["body"], "VX 30", (0, -0.083, 0.084), 0.006, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("cup", "cup_rad", [0, 1, 0], -math.tau, 0, "continuous"),
            joint("eccentric", "orbit_rad", [0, 1, 0], 0, math.tau, "continuous"),
            joint(
                "speed_dial",
                "speed_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                kind="revolute",
            ),
        ],
        readouts=[("screen", "rpm", 0, "rpm")],
        controls={
            "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 250),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("tube", "tube_visible")],
    )
