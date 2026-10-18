# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original TC8, eight real blind wells, tube strip, heated cover and latch."""
import math
from geometry import box, cylinder, label, vessel, bore_world
from bench_helpers import assembly, enclosure, front_screen, graph
from author import joint, command


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "lid",
            "latch",
            "sample",
            "screen",
            "sample_screen",
            "lid_screen",
            "start_key",
            "stop_key",
        )
    )
    body = parts["body"]
    enclosure(body, m, 0.260, 0.260, 0.130)
    block = box(
        body,
        "Thermoelectric block",
        (0, 0.020, 0.145),
        (0.160, 0.100, 0.030),
        m["steel"],
        0.002,
    )
    for x in (-0.0525, -0.0175, 0.0175, 0.0525):
        for y in (-0.0025, 0.0425):
            bore_world(block, (x, y, 0.151), 0.0035, 0.024)
            tube = vessel(
                parts["sample"],
                m["glass"],
                dict(radius_m=0.003, height_m=0.024, wall_m=0.0006, bottom_m=0.001),
            )
            tube.location = (x, y, 0.139)
            cylinder(
                parts["sample"],
                "Tube cap",
                (x, y, 0.165),
                0.004,
                0.004,
                m["white"],
                vertices=24,
            )
            cylinder(
                parts["sample"],
                "Synthetic tube content",
                (x, y, 0.148),
                0.002,
                0.015,
                m["water"],
                vertices=24,
            )
    for y in (-0.0025, 0.0425):
        box(
            parts["sample"],
            "Tube strip bridge",
            (0, y, 0.164),
            (0.112, 0.003, 0.002),
            m["white"],
            0.0003,
        )
    for x in (-0.085, 0.085):
        box(
            body,
            "Hinge tower",
            (x, 0.100, 0.161),
            (0.020, 0.025, 0.062),
            m["steel"],
            0.003,
        )
        box(
            body,
            "Cover stop",
            (x, -0.080, 0.157),
            (0.016, 0.016, 0.054),
            m["trim"],
            0.002,
        )
    parts["lid"].location = (0, 0.100, 0.187)
    box(
        parts["lid"],
        "Thermal cover shell",
        (0, -0.080, 0.012),
        (0.200, 0.195, 0.030),
        m["shell"],
        0.005,
    )
    box(
        parts["lid"],
        "Heated underside plate",
        (0, -0.080, -0.016),
        (0.170, 0.120, 0.008),
        m["steel"],
        0.002,
    )
    box(
        parts["lid"],
        "Thermal insulation backing",
        (0, -0.080, -0.0075),
        (0.170, 0.120, 0.009),
        m["white"],
        0.001,
    )
    cylinder(
        parts["lid"],
        "Rear hinge pin",
        (0, 0, 0),
        0.006,
        0.194,
        m["steel"],
        axis="X",
        vertices=24,
    )
    box(
        parts["lid"],
        "Cover handle",
        (0, -0.164, 0.028),
        (0.075, 0.020, 0.014),
        m["trim"],
        0.003,
    )
    parts["latch"].location = (0, -0.130, 0.160)
    box(
        parts["latch"],
        "Latch strap",
        (0, 0, 0.030),
        (0.025, 0.010, 0.060),
        m["blue"],
        0.002,
    )
    box(
        parts["latch"],
        "Latch catch",
        (0, 0.0275, 0.056),
        (0.025, 0.060, 0.004),
        m["steel"],
        0.001,
    )
    box(
        parts["latch"],
        "Latch grip",
        (0, -0.014, 0),
        (0.040, 0.028, 0.012),
        m["trim"],
        0.003,
    )
    for name, x in [("screen", -0.075), ("sample_screen", 0), ("lid_screen", 0.075)]:
        front_screen(parts[name], m, (x, -0.1305, 0.090), 0.058, 0.023)
    for name, x in [("start_key", -0.045), ("stop_key", 0.045)]:
        box(
            parts[name],
            "Program key",
            (x, -0.132, 0.047),
            (0.055, 0.004, 0.023),
            m["blue"] if name == "start_key" else m["trim"],
            0.003,
        )
    label(body, "TC 8", (0, -0.1305, 0.117), 0.007, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [-1, 0, 0], 0, p["opening_rad"], "revolute"),
            joint("latch", "latch_rad", [1, 0, 0], 0, math.pi / 2, "revolute"),
        ],
        readouts=[
            ("screen", "temperature_c", 1, "C"),
            ("sample_screen", "sample_c", 1, "C"),
            ("lid_screen", "lid_temperature_c", 1, "C"),
        ],
        controls={
            "lid": command("lid"),
            "latch": command("latch"),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("sample", "sample_visible")],
    )
