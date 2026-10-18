# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original EC2 bench meter with a supported lift carriage and open two-electrode cell."""
import math
from geometry import box, cylinder, label, vessel, bore_world
from bench_helpers import assembly, enclosure, front_screen, graph
from author import joint, command


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "meter",
            "probe",
            "sample",
            "screen",
            "temperature_screen",
            "power_key",
            "calibrate_key",
        )
    )
    body = parts["body"]
    box(body, "Kit base", (0, 0, 0.004), (0.360, 0.280, 0.008), m["trim"], 0.004)
    parts["meter"].location = (-0.080, 0, 0.008)
    enclosure(parts["meter"], m, 0.180, 0.160, 0.085)
    cylinder(
        body, "Stand post", (0.140, 0.080, 0.180), 0.005, 0.340, m["steel"], vertices=32
    )
    box(
        body,
        "Stand foot",
        (0.140, 0.080, 0.015),
        (0.045, 0.050, 0.014),
        m["steel"],
        0.003,
    )
    probe = parts["probe"]
    probe.location.z = p["probe_travel_m"]
    clamp = box(
        probe,
        "Sliding clamp",
        (0.140, 0.080, 0.260),
        (0.026, 0.026, 0.024),
        m["trim"],
        0.003,
    )
    bore_world(clamp, (0.140, 0.080, 0.260 + p["probe_travel_m"]), 0.0055, 0.040)
    arm = box(
        probe,
        "Probe support arm",
        (0.120, 0.0275, 0.260),
        (0.065, 0.125, 0.012),
        m["steel"],
        0.002,
    )
    bore_world(arm, (0.140, 0.080, 0.260 + p["probe_travel_m"]), 0.0055, 0.040)
    cylinder(
        probe,
        "Electrode stem",
        (0.100, -0.025, 0.145),
        0.006,
        0.230,
        m["trim"],
        vertices=32,
    )
    cylinder(
        probe,
        "Probe head",
        (0.100, -0.025, 0.260),
        0.011,
        0.020,
        m["blue"],
        vertices=32,
    )
    for z in (0.029, 0.043):
        cylinder(
            probe,
            "Conductivity electrode",
            (0.100, -0.025, z),
            0.0055,
            0.004,
            m["steel"],
            vertices=40,
        )
    cylinder(
        probe,
        "Insulating cell gap",
        (0.100, -0.025, 0.036),
        0.004,
        0.010,
        m["white"],
        vertices=32,
    )
    for dx, dy in [(-0.007, 0), (0.007, 0), (0, -0.007), (0, 0.007)]:
        cylinder(
            probe,
            "Open protective strut",
            (0.100 + dx, -0.025 + dy, 0.037),
            0.001,
            0.030,
            m["trim"],
            vertices=12,
        )
    cup = vessel(
        parts["sample"],
        m["glass"],
        dict(radius_m=0.030, height_m=0.090, wall_m=0.0015, bottom_m=0.002),
    )
    cup.location = (0.100, -0.025, 0.008)
    liquid_height = 100e-6 / (math.pi * 0.0275**2)
    cylinder(
        parts["sample"],
        "Synthetic solution",
        (0.100, -0.025, 0.012 + liquid_height / 2),
        0.0275,
        liquid_height,
        m["water"],
        vertices=64,
        bevel=0,
    )
    front_screen(parts["screen"], m, (-0.097, -0.0805, 0.065), 0.096, 0.023)
    front_screen(parts["temperature_screen"], m, (-0.033, -0.0805, 0.065), 0.027, 0.023)
    for name, x in [("power_key", -0.125), ("calibrate_key", -0.045)]:
        box(
            parts[name],
            "Meter key",
            (x, -0.082, 0.033),
            (0.050, 0.004, 0.018),
            m["blue"],
            0.002,
        )
    label(parts["meter"], "EC 2", (0, -0.0805, 0.080), 0.006, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("probe", "probe_m", [0, 1, 0], -p["probe_travel_m"], 0, "prismatic")
        ],
        readouts=[
            ("screen", "conductivity_25_us_cm", 1, "uS/cm"),
            ("temperature_screen", "temperature_c", 1, "C"),
        ],
        controls={
            "probe": command("probe"),
            "power_key": command("power"),
            "calibrate_key": command("calibrate"),
        },
        visibility=[("sample", "sample_visible")],
    )
