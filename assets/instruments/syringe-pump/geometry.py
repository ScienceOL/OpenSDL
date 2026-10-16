# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original syringe-pump geometry and declarative bindings, metres, Blender Z-up."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "plunger",
            "liquid",
            "flow_dial",
            "screen",
            "volume_screen",
            "start_key",
            "stop_key",
        )
    )
    body = parts["body"]
    enclosure(body, m, 0.260, 0.150, 0.070)
    for y in (-0.030, 0.030):
        cylinder(body, "Guide rail", (0, y, 0.115), 0.003, 0.225, m["steel"], "X", 32)
    # Barrel bore and displacement follow the exact authored area.
    cup = vessel(
        body,
        m["glass"],
        {"radius_m": 0.0082, "height_m": 0.070, "wall_m": 0.0012, "bottom_m": 0.002},
    )
    cup.rotation_euler.y = math.pi / 2
    cup.location = (-0.045, 0, 0.120)
    for x in (-0.050, 0.030):
        box(
            body,
            "Barrel support",
            (x, 0, 0.092),
            (0.018, 0.040, 0.045),
            m["trim"],
            0.003,
        )
    cylinder(
        body, "Luer nozzle", (-0.056, 0, 0.120), 0.0025, 0.012, m["white"], "X", 24
    )
    parts["liquid"].location = (-0.043, 0, 0.120)
    length = p["capacity_ml"] * 1e-6 / (math.pi * p["bore_radius_m"] ** 2)
    cylinder(
        parts["liquid"],
        "Syringe liquid",
        (length / 2, 0, 0),
        p["bore_radius_m"],
        length,
        m["water"],
        "X",
        64,
        bevel=0,
    )
    parts["plunger"].location = (0.100, 0, 0.120)
    cylinder(
        parts["plunger"],
        "Piston rod",
        (-0.040, 0, 0),
        0.003,
        0.080,
        m["white"],
        "X",
        24,
    )
    box(
        parts["plunger"],
        "Drive carriage",
        (0, 0, -0.005),
        (0.020, 0.085, 0.040),
        m["trim"],
        0.004,
    )
    cylinder(
        parts["plunger"],
        "Piston seal",
        (-0.143 + length + 0.0015, 0, 0),
        p["bore_radius_m"],
        0.003,
        m["rubber"],
        "X",
        32,
    )
    cylinder(body, "Lead screw", (0, 0.050, 0.110), 0.0035, 0.230, m["steel"], "X", 32)
    front_screen(parts["screen"], m, (-0.055, -0.0755, 0.045), 0.070, 0.022)
    front_screen(parts["volume_screen"], m, (0.024, -0.0755, 0.045), 0.070, 0.022)
    knob(parts["flow_dial"], m, (0.095, -0.081, 0.045), 0.012)
    for name, x in [("start_key", -0.030), ("stop_key", 0.010)]:
        box(
            parts[name],
            "Key",
            (x, -0.076, 0.020),
            (0.028, 0.002, 0.013),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(body, "SP 10  /  OpenSDL", (0, -0.0755, 0.064), 0.0045, m["trim"])
    return root, parts


def model_graph(p, parts):
    result = graph(
        p,
        parts,
        joints=[
            joint(
                "plunger", "plunger_m", [-1, 0, 0], 0, p["stroke_m"], kind="prismatic"
            ),
            joint(
                "flow_dial",
                "flow_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                kind="revolute",
            ),
        ],
        readouts=[
            ("screen", "flow_ml_min", 2, "mL/min"),
            ("volume_screen", "remaining_ml", 3, "mL"),
        ],
        controls={
            "flow_dial": dial(
                "set_flow", "flow_ml_min", "flow_ml_min", 0, p["max_flow_ml_min"], 0.5
            ),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("liquid", "liquid_visible")],
    )
    next(c for c in result["components"] if c["id"] == "liquid")["scale"] = {
        "axis": "x",
        "state": "fill_fraction",
        "min": 0,
        "max": 1,
    }
    return result
