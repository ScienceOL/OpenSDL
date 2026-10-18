# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original UV10 sample bay, supported lid and hollow 10mm optical-path cuvette."""
import math
from geometry import box, cylinder, label
from bench_helpers import assembly, knob, front_screen, graph
from author import joint, command, dial


def create_model(p):
    root, parts, m = assembly(
        (
            "body",
            "lid",
            "cuvette",
            "screen",
            "wavelength_screen",
            "wavelength_dial",
            "power_key",
            "blank_key",
            "measure_key",
        )
    )
    body = parts["body"]
    box(body, "Bench base", (0, 0, 0.025), (0.380, 0.320, 0.026), m["trim"], 0.006)
    box(
        body,
        "Optical electronics enclosure",
        (-0.110, 0, 0.109),
        (0.160, 0.320, 0.142),
        m["shell"],
        0.006,
    )
    box(
        body,
        "Sample chamber floor",
        (0.080, 0, 0.052),
        (0.220, 0.320, 0.028),
        m["trim"],
        0.002,
    )
    for x in (-0.025, 0.180):
        box(
            body,
            "Opaque bay side",
            (x, 0, 0.123),
            (0.010, 0.320, 0.114),
            m["shell"],
            0.002,
        )
    box(
        body,
        "Opaque bay rear",
        (0.080, 0.140, 0.123),
        (0.200, 0.040, 0.114),
        m["shell"],
        0.002,
    )
    box(
        body,
        "Front controls enclosure",
        (0.080, -0.112, 0.123),
        (0.200, 0.096, 0.114),
        m["shell"],
        0.003,
    )
    for x in (-0.135, 0.135):
        for y in (-0.110, 0.110):
            cylinder(
                body,
                "Isolator foot",
                (x, y, 0.006),
                0.014,
                0.012,
                m["rubber"],
                vertices=24,
            )
    box(
        body,
        "Cuvette support",
        (0.080, 0.020, 0.071),
        (0.040, 0.040, 0.010),
        m["steel"],
        0.001,
    )
    for dx in (-0.009, 0.009):
        for dy in (-0.009, 0.009):
            box(
                body,
                "Open carrier post",
                (0.080 + dx, 0.020 + dy, 0.091),
                (0.005, 0.005, 0.030),
                m["trim"],
                0.001,
            )
    # Optical path is the actual 10 mm liquid width, between two 1 mm walls.
    cuvette = parts["cuvette"]
    for dx in (-0.0055, 0.0055):
        box(
            cuvette,
            "Optical cell side",
            (0.080 + dx, 0.020, 0.0985),
            (0.001, 0.012, 0.045),
            m["glass"],
            0.00015,
        )
    for dy in (-0.0055, 0.0055):
        box(
            cuvette,
            "Optical cell end",
            (0.080, 0.020 + dy, 0.0985),
            (0.010, 0.001, 0.045),
            m["glass"],
            0.00015,
        )
    box(
        cuvette,
        "Cell bottom",
        (0.080, 0.020, 0.0765),
        (0.010, 0.010, 0.001),
        m["glass"],
        0.0001,
    )
    box(
        cuvette,
        "Synthetic optical solution",
        (0.080, 0.020, 0.092),
        (0.010, 0.010, 0.028),
        m["water"],
        0.0001,
    )
    box(
        cuvette,
        "Cuvette cap",
        (0.080, 0.020, 0.124),
        (0.014, 0.014, 0.006),
        m["white"],
        0.001,
    )
    parts["lid"].location = (0.080, 0.120, 0.183)
    box(
        parts["lid"],
        "Opaque sample cover",
        (0, -0.092, 0.003),
        (0.210, 0.184, 0.006),
        m["shell"],
        0.003,
    )
    box(
        parts["lid"],
        "Cover grip",
        (0, -0.163, 0.017),
        (0.045, 0.018, 0.022),
        m["trim"],
        0.003,
    )
    cylinder(
        parts["lid"],
        "Sample hinge",
        (0, 0, 0),
        0.005,
        0.190,
        m["steel"],
        axis="X",
        vertices=24,
    )
    for x in (-0.010, 0.170):
        box(
            body,
            "Hinge support",
            (x, 0.123, 0.175),
            (0.014, 0.014, 0.022),
            m["steel"],
            0.002,
        )
    front_screen(parts["screen"], m, (-0.105, -0.1605, 0.134), 0.100, 0.030)
    front_screen(parts["wavelength_screen"], m, (0.055, -0.1605, 0.134), 0.080, 0.030)
    knob(parts["wavelength_dial"], m, (0.147, -0.165, 0.125), radius=0.014)
    for name, x in [
        ("power_key", -0.140),
        ("blank_key", -0.065),
        ("measure_key", 0.045),
    ]:
        box(
            parts[name],
            "Optical control key",
            (x, -0.162, 0.083),
            (0.055, 0.004, 0.024),
            m["blue"],
            0.003,
        )
    label(body, "UV 10", (-0.108, -0.1605, 0.170), 0.009, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("lid", "lid_rad", [-1, 0, 0], 0, p["opening_rad"], "revolute"),
            joint(
                "wavelength_dial",
                "wavelength_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
        ],
        readouts=[
            ("screen", "absorbance", 3, "A"),
            ("wavelength_screen", "wavelength_nm", 0, "nm"),
        ],
        controls={
            "lid": command("lid"),
            "power_key": command("power"),
            "blank_key": command("blank"),
            "measure_key": command("measure"),
            "wavelength_dial": dial(
                "set_wavelength",
                "wavelength_nm",
                "wavelength_nm",
                p["wavelength_min_nm"],
                p["wavelength_max_nm"],
                10,
            ),
        },
        visibility=[("cuvette", "cuvette_visible")],
    )
