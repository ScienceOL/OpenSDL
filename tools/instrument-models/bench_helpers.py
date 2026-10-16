"""Reusable metre primitives for instrument authors, independent of device behavior."""

import math
from geometry import box, component, cylinder, label, material
from author import screen


def palette():
    return {
        "shell": material("Warm enamel", (0.73, 0.76, 0.74), roughness=0.30),
        "trim": material("Graphite polymer", (0.025, 0.038, 0.042), roughness=0.42),
        "steel": material("Satin stainless steel", (0.62, 0.68, 0.71), 0.85, 0.26),
        "glass": material(
            "Glass", (0.89, 0.96, 0.98), roughness=0.08, transmission=0.96
        ),
        "rubber": material("Elastomer", (0.018, 0.023, 0.025), roughness=0.8),
        "blue": material("Control accent", (0.08, 0.25, 0.29), 0.35, 0.30),
        "water": material(
            "Water", (0.39, 0.74, 0.82), roughness=0.1, transmission=0.80
        ),
        "white": material("Ceramic and PTFE", (0.91, 0.91, 0.87), roughness=0.28),
    }


def assembly(names):
    root = component("instrument")
    return root, {name: component(name, root) for name in names}, palette()


def enclosure(body, m, width, depth, height):
    box(
        body,
        "Enamel enclosure",
        (0, 0, (height + 0.012) / 2),
        (width, depth, height - 0.012),
        m["shell"],
        0.006,
    )
    box(
        body,
        "Lower seam",
        (0, 0, 0.022),
        (width, depth - 0.002, 0.002),
        m["trim"],
        0.0004,
    )
    for x in (-width * 0.34, width * 0.34):
        for y in (-depth * 0.34, depth * 0.34):
            cylinder(
                body,
                "Isolator foot",
                (x, y, 0.006),
                min(0.012, width * 0.08),
                0.012,
                m["rubber"],
                vertices=24,
            )
    for i in range(5):
        box(
            body,
            "Vent",
            (width / 2 + 0.0002, depth * 0.15 + i * 0.012, height * 0.5),
            (0.0005, 0.007, min(0.020, height * 0.3)),
            m["trim"],
            0.0002,
        )


def knob(part, m, position, radius=0.013):
    part.location = position
    cylinder(part, "Encoder", (0, 0, 0), radius, 0.010, m["trim"], "Y", 32)
    box(
        part,
        "Index",
        (0, -0.0058, radius * 0.6),
        (0.0015, 0.001, 0.005),
        m["white"],
        0.0002,
    )
    for i in range(8):
        angle = i * math.tau / 8
        cylinder(
            part,
            "Grip",
            (math.cos(angle) * radius * 0.92, 0, math.sin(angle) * radius * 0.92),
            0.0007,
            0.009,
            m["steel"],
            "Y",
            6,
        )


def front_screen(part, m, position, width=0.075, height=0.022):
    obj = screen(part, position, (width, height), m["trim"])
    obj.rotation_euler.x = math.pi / 2


def graph(
    p,
    parts,
    joints=None,
    readouts=None,
    controls=None,
    visibility=None,
    placements=None,
):
    controls = controls or {}
    return {
        "schema_version": 1,
        "model_id": p["id"],
        "dimensions_m": [
            p["dimensions_m"][0],
            p["dimensions_m"][2],
            p["dimensions_m"][1],
        ],
        "components": [
            {
                "id": k,
                "label": k.replace("_", " ").title(),
                "parent": v.parent.get("opensdlComponent"),
                **({"control": controls[k]} if k in controls else {}),
            }
            for k, v in parts.items()
        ],
        "joints": joints or [],
        "readouts": [
            {"component": c, "state": s, "digits": d, "unit": u}
            for c, s, d, u in (readouts or [])
        ],
        "visibility": [{"component": c, "state": s} for c, s in (visibility or [])],
        "placements": placements or [],
    }
