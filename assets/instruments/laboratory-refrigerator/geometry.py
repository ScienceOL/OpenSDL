# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original LR88 insulated cabinet, wire shelves, supported door and vial rack."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, knob, front_screen, graph
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
            "cooling_led",
        )
    )
    body = parts["body"]
    box(body, "Insulated floor", (0, 0.010, 0.115), (0.5, 0.5, 0.07), m["shell"], 0.007)
    box(body, "Insulated roof", (0, 0.010, 0.785), (0.5, 0.5, 0.07), m["shell"], 0.007)
    box(body, "Insulated rear", (0, 0.225, 0.450), (0.5, 0.07, 0.60), m["shell"], 0.005)
    for x in (-0.220, 0.220):
        box(
            body,
            "Insulated side",
            (x, 0.010, 0.450),
            (0.060, 0.430, 0.60),
            m["shell"],
            0.005,
        )
        box(
            body,
            "White cavity liner",
            (x * 0.861, -0.005, 0.450),
            (0.003, 0.380, 0.60),
            m["white"],
            0.001,
        )
    for x in (-0.190, 0.190):
        for y in (-0.180, 0.180):
            cylinder(
                body,
                "Adjustable foot",
                (x, y, 0.040),
                0.022,
                0.080,
                m["rubber"],
                vertices=24,
            )
    box(
        body,
        "Compressor plinth",
        (0, 0.015, 0.055),
        (0.460, 0.460, 0.045),
        m["trim"],
        0.006,
    )
    for z in (0.345, 0.570):
        for x in (-0.180, 0.180):
            box(
                body,
                "Shelf rail",
                (x, -0.005, z - 0.004),
                (0.015, 0.370, 0.012),
                m["steel"],
                0.001,
            )
        for x in [-0.175 + i * 0.025 for i in range(15)]:
            cylinder(
                body,
                "Shelf wire",
                (x, -0.005, z),
                0.002,
                0.365,
                m["steel"],
                axis="Y",
                vertices=8,
            )
        for y in (-0.180, 0.170):
            cylinder(
                body,
                "Shelf crossbar",
                (0, y, z - 0.001),
                0.003,
                0.365,
                m["steel"],
                axis="X",
                vertices=12,
            )
    box(
        body,
        "Rear cooling panel",
        (0, 0.188, 0.450),
        (0.320, 0.008, 0.500),
        m["steel"],
        0.003,
    )
    for i in range(7):
        box(
            body,
            "Plinth ventilation",
            (-0.150 + i * 0.050, -0.216, 0.057),
            (0.027, 0.003, 0.008),
            m["steel"],
            0.001,
        )
    for z in (0.120, 0.740):
        cylinder(
            body,
            "Fixed hinge pin",
            (-0.225, -0.246, z),
            0.009,
            0.040,
            m["steel"],
            vertices=24,
        )
        box(
            body,
            "Fixed hinge bracket",
            (-0.225, -0.225, z),
            (0.030, 0.054, 0.015),
            m["steel"],
            0.002,
        )
    parts["lid"].location = (-0.225, -0.246, 0.430)
    door = parts["lid"]
    box(
        door,
        "Insulated door",
        (0.225, -0.004, 0),
        (0.450, 0.035, 0.680),
        m["shell"],
        0.007,
    )
    for x in (0.015, 0.435):
        box(
            door,
            "Vertical door gasket",
            (x, 0.016, 0),
            (0.012, 0.009, 0.650),
            m["rubber"],
            0.002,
        )
    for z in (-0.320, 0.320):
        box(
            door,
            "Horizontal door gasket",
            (0.225, 0.016, z),
            (0.420, 0.009, 0.012),
            m["rubber"],
            0.002,
        )
    for z in (-0.100, 0.100):
        box(
            door,
            "Handle standoff",
            (0.395, -0.033, z),
            (0.018, 0.024, 0.018),
            m["steel"],
            0.003,
        )
    box(
        door, "Door handle", (0.395, -0.043, 0), (0.020, 0.014, 0.220), m["trim"], 0.004
    )
    for key in (
        "screen",
        "sample_screen",
        "temperature_dial",
        "start_key",
        "stop_key",
        "cooling_led",
    ):
        parts[key].parent = door
    front_screen(parts["screen"], m, (0.135, -0.022, 0.255), 0.100, 0.028)
    front_screen(parts["sample_screen"], m, (0.260, -0.022, 0.255), 0.100, 0.028)
    knob(parts["temperature_dial"], m, (0.360, -0.028, 0.255), radius=0.013)
    for name, x in [("start_key", 0.135), ("stop_key", 0.260)]:
        box(
            parts[name],
            "Cooling key",
            (x, -0.023, 0.205),
            (0.060, 0.004, 0.023),
            m["blue"] if name == "start_key" else m["trim"],
            0.003,
        )
    box(
        parts["cooling_led"],
        "Cooling indicator",
        (0.360, -0.023, 0.205),
        (0.009, 0.002, 0.009),
        m["blue"],
        0.002,
    )
    label(door, "LR 88", (0.100, -0.022, -0.235), 0.018, m["trim"])
    rack = parts["sample"]
    box(
        rack,
        "Removable rack base",
        (0, -0.005, 0.355),
        (0.270, 0.220, 0.016),
        m["blue"],
        0.002,
    )
    for x in (-0.085, 0, 0.085):
        for y in (-0.060, 0.050):
            jar = vessel(
                rack,
                m["glass"],
                dict(radius_m=0.025, height_m=0.100, wall_m=0.002, bottom_m=0.003),
            )
            jar.location = (x, y, 0.363)
            cylinder(
                rack, "Vial cap", (x, y, 0.468), 0.027, 0.010, m["white"], vertices=32
            )
            cylinder(
                rack,
                "Synthetic sample",
                (x, y, 0.398),
                0.021,
                0.060,
                m["water"],
                vertices=32,
            )
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [0, -1, 0], 0, p["opening_rad"], "revolute"),
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
                p["min_c"],
                p["max_c"],
                1,
            ),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[("sample", "sample_visible"), ("cooling_led", "cooling")],
    )
