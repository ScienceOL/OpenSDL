# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original ph-meter geometry and declarative bindings, metres, Blender Z-up."""
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, enclosure, front_screen, graph
from author import joint, command


def create_model(p):
    root, parts, m = assembly(("body", "probe", "screen", "power_key", "calibrate_key"))
    parts["probe"].location.z = p["probe_travel_m"]
    body = parts["body"]
    enclosure(body, m, 0.220, 0.220, 0.060)
    cylinder(
        body,
        "Electrode stand",
        (0.085, 0.070, 0.180),
        0.004,
        0.255,
        m["steel"],
        vertices=32,
    )
    box(
        body,
        "Support arm",
        (0.039, 0.070, 0.307),
        (0.100, 0.012, 0.010),
        m["trim"],
        0.003,
    )
    box(
        body,
        "Probe bracket",
        (-0.008, 0.027, 0.307),
        (0.012, 0.090, 0.010),
        m["trim"],
        0.003,
    )
    cylinder(
        parts["probe"],
        "Electrode stem",
        (-0.008, -0.010, 0.205),
        0.005,
        0.240,
        m["trim"],
        vertices=32,
    )
    cylinder(
        parts["probe"],
        "Glass membrane",
        (-0.008, -0.010, 0.092),
        0.004,
        0.014,
        m["glass"],
        vertices=32,
    )
    cup = vessel(
        body,
        m["glass"],
        {"radius_m": 0.030, "height_m": 0.070, "wall_m": 0.001, "bottom_m": 0.002},
    )
    cup.location = (-0.008, -0.010, 0.060)
    cylinder(
        body,
        "Buffer solution",
        (-0.008, -0.01, 0.088),
        0.029,
        0.05,
        m["water"],
        vertices=64,
        bevel=0,
    )
    front_screen(parts["screen"], m, (0, -0.1105, 0.042), 0.090, 0.025)
    for name, x in [("power_key", -0.077), ("calibrate_key", 0.077)]:
        box(
            parts[name],
            "Key",
            (x, -0.111, 0.035),
            (0.030, 0.003, 0.018),
            m["blue"],
            0.002,
        )
    label(body, "PH 14", (0, -0.111, 0.018), 0.0045, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint(
                "probe", "probe_m", [0, 1, 0], -p["probe_travel_m"], 0, kind="prismatic"
            )
        ],
        readouts=[("screen", "ph", 2, "pH")],
        controls={
            "probe": command("probe"),
            "power_key": command("power"),
            "calibrate_key": command("calibrate"),
        },
    )
