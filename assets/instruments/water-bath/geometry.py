# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original water-bath geometry and declarative bindings, metres, Blender Z-up."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, graph, front_screen, knob
from author import joint, command, dial


def create_model(p):
    (root, parts, m) = assembly(
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
    (w, d, h) = p["housing_m"]
    box(body, "Chassis floor", (0, 0, 0.035), (w, d, 0.05), m["shell"], 0.008)
    for x in (-w / 2 + 0.01, w / 2 - 0.01):
        box(
            body,
            "Insulated side",
            (x, 0, h / 2),
            (0.02, d, h - 0.04),
            m["shell"],
            0.004,
        )
    box(
        body,
        "Rear wall",
        (0, d / 2 - 0.012, h / 2),
        (w - 0.03, 0.024, h - 0.04),
        m["shell"],
        0.004,
    )
    box(
        body,
        "Front wall",
        (0, -d / 2 + 0.012, h / 2),
        (w - 0.03, 0.024, h - 0.04),
        m["shell"],
        0.004,
    )
    box(
        body,
        "Bath floor",
        (0, 0, 0.072),
        (w - 0.05, d - 0.05, 0.005),
        m["steel"],
        0.002,
    )
    water_height = 0.002 / ((w - 0.052) * (d - 0.052))
    box(
        body,
        "Water charge",
        (0, 0, 0.0745 + water_height / 2),
        (w - 0.052, d - 0.052, water_height),
        m["water"],
        0.001,
    )
    parts["lid"].location = (0, d / 2 - 0.01, h)
    box(
        parts["lid"],
        "Lift-up bath lid",
        (0, -d / 2 + 0.01, 0.004),
        (w, d, 0.008),
        m["steel"],
        0.003,
    )
    box(
        parts["lid"],
        "Lid handle",
        (0, -d / 2 + 0.01, 0.018),
        (0.065, 0.017, 0.02),
        m["trim"],
        0.003,
    )
    vial = vessel(
        parts["sample"],
        m["glass"],
        {"radius_m": 0.014, "height_m": 0.065, "wall_m": 0.001, "bottom_m": 0.002},
    )
    vial.location = (0, 0, 0.0875)
    cylinder(
        parts["sample"],
        "Inert sample charge",
        (0, 0, 0.1145),
        0.013,
        0.05,
        m["water"],
        vertices=64,
        bevel=0,
    )
    cylinder(
        parts["sample"], "Vial cap", (0, 0, 0.155), 0.016, 0.006, m["blue"], vertices=32
    )
    box(body, "Vial rack", (0, 0, 0.086), (0.070, 0.070, 0.003), m["steel"], 0.001)
    for x in (-0.025, 0.025):
        for y in (-0.025, 0.025):
            cylinder(
                body,
                "Rack foot",
                (x, y, 0.08025),
                0.002,
                0.0115,
                m["steel"],
                vertices=16,
            )
    fascia = (0, -d / 2 - 0.002, 0.1)
    front_screen(parts["screen"], m, fascia, 0.08, 0.026)
    front_screen(
        parts["sample_screen"],
        m,
        (fascia[0] + 0.085, fascia[1], fascia[2]),
        0.065,
        0.021,
    )
    knob(parts["temperature_dial"], m, (-0.088, fascia[1] - 0.005, fascia[2]))
    for name, x in [("start_key", -0.025), ("stop_key", 0.025)]:
        box(
            parts[name],
            "Key",
            (x, fascia[1], fascia[2] - 0.033),
            (0.031, 0.003, 0.016),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    for x in (-w * 0.35, w * 0.35):
        for y in (-d * 0.35, d * 0.35):
            cylinder(
                body, "Foot", (x, y, 0.006), 0.014, 0.012, m["rubber"], vertices=24
            )
    label(body, p["short_name"], (0, -d / 2 - 0.002, 0.04), 0.007, m["trim"])
    return (root, parts)


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [-1, 0, 0], 0, p["opening_rad"], kind="revolute"),
            joint(
                "temperature_dial",
                "temperature_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                kind="revolute",
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
