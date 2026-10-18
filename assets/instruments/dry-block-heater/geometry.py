# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original DB6 heater: six actual machined wells, tubes and a hinged clear cover."""
import math
from geometry import box, cylinder, label, vessel, bore_world
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "lid",
            "sample",
            "screen",
            "sample_screen",
            "temperature_dial",
            "start_key",
            "stop_key",
        )
    )
    body = parts["body"]
    enclosure(body, m, 0.220, 0.200, 0.070)
    for x in (-0.080, 0.080):
        box(
            body,
            "Rear hinge support",
            (x, 0.070, 0.117),
            (0.012, 0.018, 0.094),
            m["steel"],
            0.002,
        )
        box(
            body,
            "Front cover stop",
            (x, -0.050, 0.116),
            (0.010, 0.010, 0.092),
            m["trim"],
            0.002,
        )
    block = box(
        body,
        "Machined aluminium block",
        (0, 0.015, 0.100),
        (0.145, 0.095, 0.050),
        m["steel"],
        0.002,
    )
    for x in (-0.045, 0, 0.045):
        for y in (-0.007, 0.037):
            bore_world(block, (x, y, 0.1075), 0.007, 0.045, vertices=40)
            tube = vessel(
                parts["sample"],
                m["glass"],
                {
                    "radius_m": 0.006,
                    "height_m": 0.062,
                    "wall_m": 0.0008,
                    "bottom_m": 0.0015,
                },
            )
            tube.location = (x, y, 0.086)
            cylinder(
                parts["sample"],
                "Tube cap",
                (x, y, 0.150),
                0.008,
                0.005,
                m["blue"],
                vertices=32,
            )
            cylinder(
                parts["sample"],
                "Synthetic sample",
                (x, y, 0.107),
                0.0049,
                0.038,
                m["water"],
                vertices=32,
            )
    parts["lid"].location = (0, 0.070, 0.164)
    box(
        parts["lid"],
        "Clear cover",
        (0, -0.055, 0),
        (0.170, 0.140, 0.004),
        m["glass"],
        0.002,
    )
    for x in (-0.085, 0.085):
        box(
            parts["lid"],
            "Cover edge",
            (x, -0.055, 0.002),
            (0.004, 0.140, 0.008),
            m["trim"],
            0.001,
        )
    cylinder(
        parts["lid"],
        "Rear hinge",
        (0, 0, 0),
        0.005,
        0.175,
        m["steel"],
        axis="X",
        vertices=24,
    )
    box(
        parts["lid"],
        "Cover grip",
        (0, -0.126, 0.006),
        (0.035, 0.010, 0.009),
        m["trim"],
        0.002,
    )
    front_screen(parts["screen"], m, (-0.018, -0.1005, 0.045), 0.065, 0.019)
    front_screen(parts["sample_screen"], m, (0.057, -0.1005, 0.045), 0.061, 0.019)
    knob(parts["temperature_dial"], m, (-0.082, -0.105, 0.037), radius=0.012)
    for name, x in [("start_key", 0.005), ("stop_key", 0.057)]:
        box(
            parts[name],
            "Control key",
            (x, -0.102, 0.018),
            (0.031, 0.004, 0.013),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(body, "DB 6", (-0.018, -0.1005, 0.062), 0.005, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [-1, 0, 0], 0, p["opening_rad"], "revolute"),
            joint(
                "temperature_dial",
                "temperature_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
        ],
        readouts=[
            ("screen", "temperature_c", 1, "C"),
            ("sample_screen", "sample_c", 1, "C"),
        ],
        controls={
            "lid": command("lid"),
            "temperature_dial": dial(
                "set_temperature",
                "temperature_c",
                "target_c",
                p["ambient_c"],
                p["max_c"],
                5,
            ),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("sample", "sample_visible")],
    )
