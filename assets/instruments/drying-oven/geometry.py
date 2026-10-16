# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original drying-oven geometry and declarative bindings, metres, Blender Z-up."""
import math
from geometry import box, cylinder, label
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
    box(body, "Roof", (0, 0, h - 0.01), (w, d, 0.02), m["shell"], 0.005)
    for z in (0.09, 0.18):
        for x in (-0.09, -0.06, -0.03, 0, 0.03, 0.06, 0.09):
            box(
                body,
                "Ventilated rack",
                (x, 0.012, z),
                (0.003, d - 0.06, 0.003),
                m["steel"],
                0.0004,
            )
    parts["lid"].location = (-w / 2, -d / 2, 0.04)
    box(
        parts["lid"],
        "Insulated hinged door",
        (w / 2, -0.01, (h - 0.04) / 2),
        (w, 0.02, h - 0.04),
        m["shell"],
        0.005,
    )
    box(
        parts["lid"],
        "Door handle",
        (w - 0.035, -0.027, h * 0.55),
        (0.012, 0.018, 0.1),
        m["trim"],
        0.004,
    )
    box(
        parts["sample"],
        "Drying tray",
        (0, 0.015, 0.0965),
        (0.13, 0.14, 0.01),
        m["steel"],
        0.002,
    )
    box(
        parts["sample"],
        "Inert wet fixture",
        (0, 0.015, 0.1055),
        (0.09, 0.095, 0.008),
        m["white"],
        0.002,
    )
    fascia = (0, -d / 2 - 0.022, 0.33)
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
            joint("lid", "lid_rad", [0, -1, 0], 0, p["opening_rad"], kind="revolute"),
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
