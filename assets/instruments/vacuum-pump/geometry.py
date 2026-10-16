# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original vacuum-pump geometry and declarative bindings, metres, Blender Z-up."""
import math
import bpy
from geometry import box, cylinder, label, vessel
from bench_helpers import assembly, enclosure, front_screen, graph
from author import joint, command


def create_model(p):
    root, parts, m = assembly(("body", "vent_dial", "screen", "start_key", "stop_key"))
    body = parts["body"]
    enclosure(body, m, 0.300, 0.210, 0.060)
    cylinder(
        body, "Diaphragm motor", (0, 0.035, 0.110), 0.050, 0.170, m["steel"], "X", 64
    )
    for x in (-0.078, -0.060, -0.042, -0.024, -0.006, 0.012, 0.030, 0.048, 0.066):
        cylinder(
            body, "Cooling fin", (x, 0.035, 0.110), 0.054, 0.003, m["trim"], "X", 48
        )
    cylinder(
        body, "Pump head", (-0.105, 0.035, 0.110), 0.039, 0.040, m["trim"], "X", 48
    )
    cylinder(
        body,
        "Inlet fitting",
        (-0.105, 0.035, 0.157),
        0.006,
        0.022,
        m["steel"],
        vertices=24,
    )
    # Sealed 0.5 L teaching chamber sits beside the pump on the same base.
    cup = vessel(
        body,
        m["glass"],
        {"radius_m": 0.043, "height_m": 0.105, "wall_m": 0.002, "bottom_m": 0.004},
    )
    cup.location = (0.100, -0.040, 0.060)
    cylinder(
        body,
        "Chamber lid",
        (0.100, -0.040, 0.170),
        0.046,
        0.008,
        m["steel"],
        vertices=64,
    )
    cylinder(
        body,
        "Vacuum chamber hose fitting",
        (0.080, -0.045, 0.180),
        0.004,
        0.012,
        m["steel"],
        vertices=24,
    )
    curve = bpy.data.curves.new("Vacuum hose", "CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = 8
    curve.bevel_depth = 0.003
    curve.bevel_resolution = 2
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(4)
    for point, position in zip(
        spline.bezier_points,
        [
            (-0.105, 0.035, 0.168),
            (-0.105, 0.035, 0.190),
            (-0.050, 0.035, 0.208),
            (0.020, -0.045, 0.208),
            (0.080, -0.045, 0.186),
        ],
    ):
        point.co = position
        point.handle_left_type = point.handle_right_type = "AUTO"
    hose = bpy.data.objects.new("Connected vacuum hose", curve)
    bpy.context.collection.objects.link(hose)
    hose.parent = body
    hose.data.materials.append(m["rubber"])
    bpy.ops.object.select_all(action="DESELECT")
    hose.select_set(True)
    bpy.context.view_layer.objects.active = hose
    bpy.ops.object.convert(target="MESH")
    parts["vent_dial"].location = (0.100, -0.040, 0.184)
    cylinder(
        parts["vent_dial"],
        "Vent valve",
        (0, 0, 0),
        0.010,
        0.018,
        m["blue"],
        vertices=32,
    )
    box(
        parts["vent_dial"],
        "Valve index",
        (0, -0.007, 0.010),
        (0.002, 0.008, 0.002),
        m["white"],
        0.0003,
    )
    front_screen(parts["screen"], m, (0, -0.106, 0.040), 0.070, 0.022)
    for name, x in [("start_key", -0.080), ("stop_key", -0.040)]:
        box(
            parts[name],
            "Key",
            (x, -0.106, 0.026),
            (0.026, 0.003, 0.014),
            m["blue"] if name == "start_key" else m["trim"],
            0.002,
        )
    label(body, "VP 20", (0, -0.106, 0.020), 0.0045, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("vent_dial", "vent_rad", [0, 1, 0], 0, math.pi / 2, kind="revolute")
        ],
        readouts=[("screen", "pressure_kpa", 2, "kPa")],
        controls={
            "vent_dial": command("vent"),
            "start_key": command("start"),
            "stop_key": command("stop"),
        },
    )
