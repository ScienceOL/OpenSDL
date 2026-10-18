# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original TR60 parallel-roller assembly, metres and Blender Z-up."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    names = ("body", "tube", "speed_dial", "screen", "start_key", "stop_key")
    names += tuple(f"roller_{i}" for i in range(5))
    root, parts, m = assembly(names)
    enclosure(parts["body"], m, 0.320, 0.200, 0.055)
    for i in range(5):
        y = (i - 2) * 0.032
        roller = parts[f"roller_{i}"]
        roller.location = (0, y, 0.067)
        cylinder(
            roller,
            "Elastomer roller",
            (0, 0, 0),
            p["roller_radius_m"],
            0.240,
            m["rubber"],
            axis="X",
            vertices=48,
        )
        cylinder(
            roller, "Axle", (0, 0, 0), 0.004, 0.270, m["steel"], axis="X", vertices=24
        )
        # A narrow axial index makes roller motion visible without moving the housing.
        box(
            roller,
            "Roller index",
            (0, 0, p["roller_radius_m"]),
            (0.225, 0.0015, 0.0005),
            m["blue"],
            0.0002,
        )
        for x in (-0.135, 0.135):
            box(
                parts["body"],
                "Bearing pedestal",
                (x, y, 0.063),
                (0.016, 0.024, 0.021),
                m["shell"],
                0.003,
            )
    tube = parts["tube"]
    tube.location = (
        0,
        -0.016,
        0.067 + math.sqrt((p["roller_radius_m"] + p["tube_radius_m"]) ** 2 - 0.016**2),
    )
    glass = vessel(
        tube,
        m["glass"],
        {
            "radius_m": p["tube_radius_m"],
            "height_m": 0.110,
            "wall_m": 0.0012,
            "bottom_m": 0.002,
        },
    )
    glass.rotation_euler.y = math.pi / 2
    glass.location.x = -0.055
    for x in (-0.058, 0.058):
        cylinder(
            tube,
            "Flush screw cap",
            (x, 0, 0),
            p["tube_radius_m"],
            0.008,
            m["blue"],
            axis="X",
            vertices=48,
        )
    cylinder(
        tube,
        "Inert sealed sample",
        (0, 0, 0),
        0.010,
        0.095,
        m["water"],
        axis="X",
        vertices=48,
    )
    box(
        tube,
        "Tube graduation",
        (0, -0.0155, 0),
        (0.070, 0.001, 0.003),
        m["white"],
        0.0003,
    )
    knob(parts["speed_dial"], m, (-0.118, -0.105, 0.030), radius=0.012)
    front_screen(parts["screen"], m, (-0.025, -0.1005, 0.032), 0.080, 0.020)
    for name, x in [("start_key", 0.064), ("stop_key", 0.110)]:
        box(
            parts[name],
            "Control key",
            (x, -0.102, 0.030),
            (0.030, 0.004, 0.017),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(parts["body"], "TR 60", (-0.025, -0.1005, 0.048), 0.005, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint(f"roller_{i}", "roller_rad", [1, 0, 0], 0, math.tau, "continuous")
            for i in range(5)
        ]
        + [
            joint("tube", "tube_rad", [1, 0, 0], 0, math.tau, "continuous"),
            joint(
                "speed_dial",
                "speed_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
        ],
        readouts=[("screen", "rpm", 1, "rpm")],
        controls={
            "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 5),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("tube", "tube_visible")],
    )
