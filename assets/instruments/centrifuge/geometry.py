# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original centrifuge geometry and declarative bindings, metres, Blender Z-up."""
import math
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "lid",
            "rotor",
            "tube_0",
            "tube_1",
            "tube_2",
            "tube_3",
            "screen",
            "speed_dial",
            "start_key",
            "stop_key",
        )
    )
    # Four separate walls expose a genuinely open chamber when the lid opens.
    body = parts["body"]
    box(body, "Bottom pan", (0, 0, 0.036), (0.280, 0.340, 0.050), m["shell"], 0.010)
    for x in (-0.129, 0.129):
        box(body, "Side wall", (x, 0, 0.108), (0.022, 0.340, 0.110), m["shell"], 0.005)
    for y in (-0.158, 0.158):
        box(body, "End wall", (0, y, 0.108), (0.242, 0.024, 0.110), m["shell"], 0.005)
    for x in (-0.100, 0.100):
        for y in (-0.130, 0.130):
            cylinder(
                body,
                "Rubber foot",
                (x, y, 0.008),
                0.016,
                0.016,
                m["rubber"],
                vertices=24,
            )
    cylinder(body, "Rotor spindle", (0, 0.025, 0.086), 0.012, 0.055, m["steel"])
    cylinder(
        body, "Chamber liner", (0, 0.025, 0.068), 0.104, 0.006, m["steel"], vertices=64
    )
    # Pivot is at the rear hinge, not the mesh origin.
    parts["lid"].location = (0, 0.145, 0.165)
    box(
        parts["lid"],
        "Lid panel",
        (0, -0.145, 0.010),
        (0.280, 0.340, 0.020),
        m["shell"],
        0.008,
    )
    box(
        parts["lid"],
        "Lid lock grip",
        (0, -0.300, 0.022),
        (0.075, 0.022, 0.009),
        m["trim"],
        0.003,
    )
    for x in (-0.085, 0.085):
        cylinder(
            body, "Hinge barrel", (x, 0.145, 0.165), 0.008, 0.028, m["steel"], "X", 24
        )
    parts["rotor"].location = (0, 0.025, 0.100)
    cylinder(
        parts["rotor"],
        "Four-place rotor",
        (0, 0, 0),
        0.080,
        0.016,
        m["trim"],
        vertices=64,
    )
    cylinder(
        parts["rotor"],
        "Rotor nut",
        (0, 0, 0.017),
        0.014,
        0.018,
        m["steel"],
        vertices=24,
    )
    for i in range(4):
        a = i * math.tau / 4
        x, y = math.cos(a) * p["rotor_radius_m"], math.sin(a) * p["rotor_radius_m"]
        tube = parts[f"tube_{i}"]
        tube.parent = parts["rotor"]
        cup = vessel(
            tube,
            m["glass"],
            {
                "radius_m": 0.007,
                "height_m": 0.040,
                "wall_m": 0.0005,
                "bottom_m": 0.0012,
            },
        )
        cup.location = (x, y, 0.012)
        cylinder(
            tube,
            "Inert sample",
            (x, y, 0.0202),
            0.0064,
            0.014,
            m["water"],
            vertices=64,
            bevel=0,
        )
        cylinder(tube, "Screw cap", (x, y, 0.055), 0.008, 0.006, m["blue"], vertices=24)
        sleeve = vessel(
            parts["rotor"],
            m["steel"],
            {"radius_m": 0.010, "height_m": 0.024, "wall_m": 0.0015, "bottom_m": 0.002},
        )
        sleeve.location = (x, y, 0.008)
    front_screen(parts["screen"], m, (0, -0.1705, 0.111), 0.088, 0.026)
    knob(parts["speed_dial"], m, (-0.086, -0.175, 0.103))
    for name, x in [("start_key", 0.060), ("stop_key", 0.095)]:
        box(
            parts[name],
            "Key",
            (x, -0.171, 0.103),
            (0.025, 0.003, 0.019),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(body, "CF 4  /  OpenSDL", (0, -0.171, 0.065), 0.007, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [-1, 0, 0], 0, p["lid_open_rad"], kind="revolute"),
            joint("rotor", "rotor_rad", [0, 1, 0], 0, math.tau, "continuous"),
            joint(
                "speed_dial",
                "speed_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                kind="revolute",
            ),
        ],
        readouts=[("screen", "rpm", 0, "rpm")],
        controls={
            "lid": command("lid"),
            "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 250),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
        visibility=[(f"tube_{i}", f"tube_{i}_visible") for i in range(4)],
    )
