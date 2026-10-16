# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original HS150 authoring. Pinned Blender 4.5.3, metre units, Z-up source."""
import json
import math
from pathlib import Path
import sys
import bpy

sys.path.insert(0, str(Path(__file__).parent))
from geometry import box, component, cylinder, label, material, vessel
from author import build, dial, command, joint, screen

SOURCE = Path(__file__).resolve().parents[2] / "assets/instruments/heated-stirrer"


def create_model(p):
    root = component("instrument")
    parts = {
        k: component(k, root)
        for k in (
            "body",
            "plate",
            "heat_dial",
            "speed_dial",
            "power_key",
            "heat_key",
            "stir_key",
            "plate_screen",
            "liquid_screen",
            "rpm_screen",
            "vessel",
            "probe",
            "bar",
            "hot_lamp",
        )
    }
    shell = material("Ivory enamel", (0.72, 0.76, 0.75), roughness=0.28)
    trim = material("Graphite casing", (0.028, 0.040, 0.045), roughness=0.40)
    steel = material("Brushed steel", (0.6, 0.66, 0.69), 0.85, 0.25)
    ceramic = material("Ceramic plate", (0.91, 0.91, 0.87), roughness=0.25)
    rubber = material("Vibration isolators", (0.018, 0.022, 0.024), roughness=0.8)
    glass = material(
        "Borosilicate vessel", (0.88, 0.96, 0.98), roughness=0.06, transmission=0.97
    )
    water = material(
        "Water fixture", (0.42, 0.78, 0.86), roughness=0.12, transmission=0.8
    )
    accent = material("Amber hot-surface indicator", (0.9, 0.22, 0.015), roughness=0.22)
    body = parts["body"]
    box(body, "Lower shell", (0, 0, 0.047), (0.188, 0.292, 0.070), shell, 0.010)
    box(
        body,
        "Upper sloped deck",
        (0, 0.018, 0.087),
        (0.180, 0.248, 0.026),
        shell,
        0.006,
    )
    box(body, "Ventilation seam", (0, 0.018, 0.078), (0.182, 0.250, 0.003), trim)
    for x in (-0.071, 0.071):
        for y in (-0.110, 0.110):
            cylinder(body, "Rubber foot", (x, y, 0.008), 0.013, 0.016, rubber)
    for x in (-0.088, 0.088):
        for y in (0.025, 0.045, 0.065, 0.085, 0.105):
            box(
                body,
                "Recessed vent",
                (x, y, 0.047),
                (0.002, 0.012, 0.018),
                trim,
                0.0007,
            )
    cylinder(
        parts["plate"],
        "Thermal shield",
        (0, 0.028, 0.105),
        0.079,
        0.008,
        steel,
        vertices=72,
    )
    cylinder(
        parts["plate"],
        "Ceramic hotplate",
        (0, 0.028, 0.116),
        p["plate_radius_m"],
        0.008,
        ceramic,
        vertices=96,
    )
    box(body, "Control fascia", (0, -0.123, 0.081), (0.170, 0.039, 0.015), trim, 0.003)
    for identifier, x in (("heat_dial", -0.060), ("speed_dial", 0.060)):
        parent = parts[identifier]
        parent.location = (x, -0.148, 0.055)
        cylinder(parent, "Rotary encoder", (0, 0, 0), 0.016, 0.014, trim, "Y", 64)
        box(
            parent,
            "Dial index",
            (0, -0.0075, 0.009),
            (0.002, 0.0015, 0.008),
            shell,
            0.0004,
        )
        for i in range(8):
            a = i * math.tau / 8
            cylinder(
                parent,
                "Knurled grip",
                (math.cos(a) * 0.015, -0.001, math.sin(a) * 0.015),
                0.0012,
                0.011,
                steel,
                "Y",
                6,
            )
    for identifier, x in (("power_key", -0.033), ("heat_key", 0), ("stir_key", 0.033)):
        box(
            parts[identifier],
            "Membrane key",
            (x, -0.147, 0.030),
            (0.023, 0.003, 0.013),
            trim,
            0.002,
        )
        label(
            parts[identifier],
            {"power_key": "PWR", "heat_key": "HEAT", "stir_key": "STIR"}[identifier],
            (x, -0.149, 0.030),
            0.0038,
            shell,
        )
    for identifier, x in (
        ("plate_screen", -0.056),
        ("liquid_screen", 0),
        ("rpm_screen", 0.056),
    ):
        screen(parts[identifier], (x, -0.123, 0.0895), (0.047, 0.019), trim)
    label(body, "HS 150  /  OpenSDL", (0, -0.147, 0.094), 0.0048, trim)
    label(body, "TEMP", (-0.060, -0.147, 0.077), 0.0038, shell)
    label(body, "RPM", (0.060, -0.147, 0.077), 0.0038, shell)
    cylinder(
        parts["hot_lamp"], "HOT lamp", (0.079, -0.087, 0.103), 0.004, 0.002, accent
    )
    # Side probe stand and bracket; no hidden intersection through the liquid body.
    cylinder(
        body, "Probe stand", (0.077, 0.105, 0.245), 0.003, 0.286, steel, vertices=24
    )
    box(body, "Probe arm", (0.0385, 0.105, 0.385), (0.083, 0.009, 0.006), trim, 0.002)
    box(body, "Probe bracket", (0, 0.071, 0.381), (0.009, 0.074, 0.010), trim, 0.002)
    cylinder(
        parts["probe"],
        "Immersion probe",
        (0, 0.028, 0.265),
        0.002,
        0.240,
        steel,
        vertices=24,
    )
    fixture = {
        "radius_m": 0.040,
        "height_m": 0.110,
        "wall_m": 0.0012,
        "bottom_m": 0.002,
    }
    cup = vessel(parts["vessel"], glass, fixture)
    cup.location = (0, 0.028, 0.120)
    water_height = 0.0002 / (math.pi * 0.0388**2)
    cylinder(
        parts["vessel"],
        "200 ml water",
        (0, 0.028, 0.122 + water_height / 2),
        0.0388,
        water_height,
        water,
        vertices=72,
        bevel=0,
    )
    parts["bar"].location = (0, 0.028, 0.125)
    box(parts["bar"], "PTFE stir bar", (0, 0, 0), (0.030, 0.007, 0.006), ceramic, 0.003)
    return root, parts


def graph(p, parts):
    components = [
        {"id": k, "label": label, "parent": "instrument"}
        for k, label in (
            ("body", "Enclosure and probe stand"),
            ("plate", "150 mm ceramic plate"),
            ("heat_dial", "Plate temperature dial"),
            ("speed_dial", "Stir speed dial"),
            ("power_key", "Power"),
            ("heat_key", "Heat enable"),
            ("stir_key", "Stir enable"),
            ("plate_screen", "Plate temperature"),
            ("liquid_screen", "Immersed probe temperature"),
            ("rpm_screen", "Stir speed"),
            ("vessel", "200 ml water vessel"),
            ("probe", "Immersion probe"),
            ("bar", "PTFE stir bar"),
            ("hot_lamp", "Hot surface warning"),
        )
    ]
    controls = {
        "heat_dial": dial(
            "set_temperature",
            "temperature_c",
            "target_c",
            p["ambient_c"],
            p["max_plate_c"],
            5,
        ),
        "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 100),
        "power_key": command("power"),
        "heat_key": command("heat"),
        "stir_key": command("stir"),
    }
    for c in components:
        if c["id"] in controls:
            c["control"] = controls[c["id"]]
    return {
        "schema_version": 1,
        "model_id": p["id"],
        "dimensions_m": [
            p["dimensions_m"][0],
            p["dimensions_m"][2],
            p["dimensions_m"][1],
        ],
        "components": components,
        "joints": [
            joint(
                "heat_dial",
                "heat_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                kind="revolute",
            ),
            joint(
                "speed_dial",
                "speed_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                kind="revolute",
            ),
            joint("bar", "bar_rad", [0, 1, 0], 0, math.tau, "continuous"),
        ],
        "readouts": [
            {"component": c, "state": s, "digits": d, "unit": u}
            for c, s, d, u in [
                ("plate_screen", "plate_c", 1, "C"),
                ("liquid_screen", "probe_c", 1, "C"),
                ("rpm_screen", "rpm", 0, "rpm"),
            ]
        ],
        "visibility": [
            {"component": c, "state": s}
            for c, s in [
                ("vessel", "vessel_visible"),
                ("bar", "vessel_visible"),
                ("probe", "probe_visible"),
                ("hot_lamp", "hot"),
            ]
        ],
        "placements": [
            {
                "id": "plate",
                "component": "plate",
                "position": [0, 0.120, -0.028],
                "radius_m": p["vessel_radius_m"],
            }
        ],
    }


def runtime(p):
    from author import action, numeric

    actions = [
        action(k, k.replace("_", " ").capitalize())
        for k in (
            "power",
            "heat",
            "stir",
            "stop",
            "reset",
            "load_vessel",
            "unload_vessel",
            "probe",
        )
    ]
    actions += [
        action(
            "set_temperature",
            "Set plate target, not liquid temperature",
            {"temperature_c": numeric(p["ambient_c"], p["max_plate_c"])},
        ),
        action("set_speed", "Set stir target speed", {"rpm": numeric(0, p["max_rpm"])}),
    ]
    keys = (
        "ambient_c",
        "max_plate_c",
        "max_loaded_plate_c",
        "max_rpm",
        "heater_w",
        "plate_capacity_j_k",
        "liquid_capacity_j_k",
        "plate_liquid_w_k",
        "plate_air_w_k",
        "liquid_air_w_k",
        "slew_rpm_s",
        "vessel_radius_m",
    )
    return (
        actions,
        {"kind": "heated_stirrer", "model_id": p["id"], **{k: p[k] for k in keys}},
        [
            {
                "id": "water_vessel",
                "mass_kg": 0.25,
                "position_m": [0, 0],
                "radius_m": 0.04,
            }
        ],
    )


if __name__ == "__main__":
    build(SOURCE, create_model, graph, runtime, "heated-stirrer", "heater")
