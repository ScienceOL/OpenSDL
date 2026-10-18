# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original UC3.3 open steel tank, removable wire basket, cover and drain valve."""
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
            "water",
            "drain",
            "screen",
            "timer_screen",
            "temperature_dial",
            "duration_dial",
            "start_key",
            "stop_key",
            "sonic_led",
        )
    )
    body = parts["body"]
    box(body, "Lower housing", (0, 0, 0.040), (0.320, 0.260, 0.068), m["shell"], 0.006)
    for x in (-0.150, 0.150):
        box(
            body,
            "Housing side",
            (x, 0, 0.126),
            (0.020, 0.260, 0.112),
            m["shell"],
            0.004,
        )
    for y in (-0.120, 0.120):
        box(
            body, "Housing end", (0, y, 0.126), (0.280, 0.020, 0.112), m["shell"], 0.004
        )
    for x in (-0.149, 0.149):
        box(
            body,
            "Cover gasket",
            (x, 0, 0.1845),
            (0.014, 0.240, 0.005),
            m["rubber"],
            0.001,
        )
    box(
        body,
        "Front cover gasket",
        (0, -0.119, 0.1845),
        (0.284, 0.012, 0.005),
        m["rubber"],
        0.001,
    )
    box(body, "Tank floor", (0, 0, 0.081), (0.260, 0.184, 0.002), m["steel"], 0.001)
    for x in (-0.129, 0.129):
        box(
            body, "Basin wall", (x, 0, 0.131), (0.002, 0.184, 0.100), m["steel"], 0.0006
        )
    for y in (-0.091, 0.091):
        box(body, "Basin end", (0, y, 0.131), (0.256, 0.002, 0.100), m["steel"], 0.0006)
    for x in (-0.124, 0.124):
        for y in (-0.095, 0.095):
            cylinder(
                body,
                "Isolator foot",
                (x, y, 0.004),
                0.014,
                0.008,
                m["rubber"],
                vertices=24,
            )
    for i in range(5):
        box(
            body,
            "Cooling vent",
            (0.1603, 0.010 + i * 0.017, 0.046),
            (0.0006, 0.008, 0.025),
            m["trim"],
            0.0002,
        )
    parts["water"].location = (0, 0, 0.082)
    box(
        parts["water"],
        "Water fixture",
        (0, 0, 0.0375),
        (0.253, 0.177, 0.075),
        m["water"],
        0.0005,
    )
    basket = parts["sample"]
    for x in (-0.108, 0.108):
        for y in (-0.072, 0.072):
            cylinder(
                basket,
                "Basket support leg",
                (x, y, 0.091),
                0.002,
                0.018,
                m["steel"],
                vertices=12,
            )
    for x in [-0.108 + i * 0.012 for i in range(19)]:
        cylinder(
            basket,
            "Basket floor wire",
            (x, 0, 0.101),
            0.001,
            0.150,
            m["steel"],
            axis="Y",
            vertices=8,
        )
    for y in [-0.072 + i * 0.012 for i in range(13)]:
        cylinder(
            basket,
            "Cross wire",
            (0, y, 0.102),
            0.001,
            0.220,
            m["steel"],
            axis="X",
            vertices=8,
        )
    for x in (-0.110, 0.110):
        cylinder(
            basket,
            "Basket rim",
            (x, 0, 0.157),
            0.002,
            0.150,
            m["steel"],
            axis="Y",
            vertices=12,
        )
        for y in (-0.075, 0.075):
            cylinder(
                basket,
                "Basket corner",
                (x, y, 0.129),
                0.0015,
                0.056,
                m["steel"],
                vertices=12,
            )
    for y in (-0.075, 0.075):
        cylinder(
            basket,
            "Basket rim",
            (0, y, 0.157),
            0.002,
            0.220,
            m["steel"],
            axis="X",
            vertices=12,
        )
    for x in (-0.045, 0.045):
        vial = vessel(
            basket,
            m["glass"],
            {"radius_m": 0.016, "height_m": 0.056, "wall_m": 0.0012, "bottom_m": 0.002},
        )
        vial.location = (x, 0, 0.103)
        cylinder(
            basket,
            "Original inert sample",
            (x, 0, 0.119),
            0.013,
            0.027,
            m["white"],
            vertices=32,
        )
    parts["lid"].location = (0, 0.125, 0.187)
    box(
        parts["lid"],
        "Steel cover",
        (0, -0.125, 0.004),
        (0.316, 0.250, 0.008),
        m["steel"],
        0.004,
    )
    box(
        parts["lid"],
        "Cover grip",
        (0, -0.125, 0.018),
        (0.072, 0.022, 0.020),
        m["trim"],
        0.004,
    )
    cylinder(
        parts["lid"],
        "Hinge pin",
        (0, 0, 0),
        0.006,
        0.280,
        m["steel"],
        axis="X",
        vertices=24,
    )
    for x in (-0.135, 0.135):
        box(
            body,
            "Hinge support",
            (x, 0.124, 0.177),
            (0.018, 0.012, 0.022),
            m["steel"],
            0.002,
        )
    cylinder(
        body,
        "Drain fitting",
        (0.171, 0.060, 0.066),
        0.009,
        0.030,
        m["steel"],
        axis="X",
        vertices=32,
    )
    parts["drain"].location = (0.186, 0.060, 0.066)
    cylinder(
        parts["drain"],
        "Valve spindle",
        (0, 0, 0),
        0.005,
        0.014,
        m["steel"],
        vertices=24,
    )
    box(
        parts["drain"],
        "Drain lever",
        (0.014, 0, 0.009),
        (0.040, 0.012, 0.006),
        m["blue"],
        0.002,
    )
    front_screen(parts["screen"], m, (-0.040, -0.1305, 0.123), 0.069, 0.023)
    front_screen(parts["timer_screen"], m, (0.047, -0.1305, 0.123), 0.069, 0.023)
    knob(parts["temperature_dial"], m, (-0.101, -0.135, 0.060))
    knob(parts["duration_dial"], m, (-0.040, -0.135, 0.060))
    for name, x in [("start_key", 0.035), ("stop_key", 0.092)]:
        box(
            parts[name],
            "Cycle key",
            (x, -0.132, 0.060),
            (0.040, 0.004, 0.021),
            m["blue"] if name == "start_key" else m["trim"],
            0.003,
        )
    label(body, "UC 3.3", (0, -0.1305, 0.164), 0.009, m["trim"])
    box(
        body,
        "Indicator bezel",
        (-0.105, -0.132, 0.164),
        (0.010, 0.003, 0.010),
        m["trim"],
        0.002,
    )
    box(
        parts["sonic_led"],
        "Cycle indicator",
        (-0.105, -0.134, 0.164),
        (0.006, 0.001, 0.006),
        m["blue"],
        0.001,
    )
    return root, parts


def model_graph(p, parts):
    result = graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [-1, 0, 0], 0, p["opening_rad"], "revolute"),
            joint("drain", "drain_rad", [0, 1, 0], 0, math.pi / 2, "revolute"),
            joint(
                "temperature_dial",
                "temperature_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
            joint(
                "duration_dial",
                "duration_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
        ],
        readouts=[
            ("screen", "temperature_c", 1, "C"),
            ("timer_screen", "remaining_s", 0, "s"),
        ],
        controls={
            "lid": command("lid"),
            "drain": command("drain"),
            "temperature_dial": dial(
                "set_temperature",
                "temperature_c",
                "target_c",
                p["ambient_c"],
                p["max_c"],
                5,
            ),
            "duration_dial": dial(
                "set_duration", "seconds", "duration_s", 1, p["max_duration_s"], 10
            ),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[
            ("sample", "sample_visible"),
            ("water", "water_visible"),
            ("sonic_led", "running"),
        ],
    )
    next(c for c in result["components"] if c["id"] == "water")["scale"] = {
        "axis": "y",
        "state": "fill_fraction",
        "min": 0,
        "max": 1,
    }
    return result
