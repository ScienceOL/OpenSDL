# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original OS250, metre-scale Z-up source with an explicitly counter-rotating platform."""
import math
import bpy
from geometry import box, cylinder, finish, label
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial


def flask(parent, surface):
    # Paired outer/inner profiles form an open neck, hollow cone and closed bottom.
    profile = [
        (0.036, 0.002), (0.039, 0.008), (0.039, 0.022),
        (0.014, 0.097), (0.014, 0.126), (0.016, 0.127),
        (0.016, 0.130), (0.0125, 0.130), (0.0125, 0.097),
        (0.0375, 0.022), (0.0375, 0.008), (0.0345, 0.0035),
        (0.0, 0.0035), (0.0, 0.002),
    ]
    n = 64
    vertices = [
        (r * math.cos(i * math.tau / n), r * math.sin(i * math.tau / n), z)
        for r, z in profile for i in range(n)
    ]
    faces = [
        (j*n+i, j*n+(i+1)%n, ((j+1)%len(profile))*n+(i+1)%n, ((j+1)%len(profile))*n+i)
        for j in range(len(profile)) for i in range(n)
    ]
    mesh = bpy.data.meshes.new("Original hollow flask profile")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("Conical flask", mesh)
    bpy.context.collection.objects.link(obj)
    finish(obj, parent, surface)


def create_model(p):
    root, parts, m = assembly((
        "body", "eccentric", "platform", "flask", "clamp_0", "clamp_1", "clamp_2",
        "speed_dial", "screen", "start_key", "stop_key",
    ))
    enclosure(parts["body"], m, 0.320, 0.300, 0.090)
    cylinder(parts["body"], "Drive bearing", (0, 0.008, 0.096), 0.035, 0.012, m["steel"])
    parts["eccentric"].location = (0, 0.008, 0.102)
    cylinder(parts["eccentric"], "Eccentric drive", (0, 0, 0.004), 0.027, 0.008, m["trim"])
    parts["platform"].parent = parts["eccentric"]
    parts["platform"].location = (p["orbit_m"], 0, 0.012)
    box(parts["platform"], "Platform deck", (0, 0, 0.004), (0.250, 0.250, 0.008), m["steel"], 0.004)
    box(parts["platform"], "Nonslip mat", (0, 0, 0.009), (0.240, 0.240, 0.002), m["rubber"], 0.003)
    for x in (-0.112, 0.112):
        for y in (-0.112, 0.112):
            cylinder(parts["platform"], "Platform fastener", (x, y, 0.0095), 0.003, 0.001, m["trim"], vertices=12)
    parts["flask"].parent = parts["platform"]
    parts["flask"].location = (0, 0, 0.008)
    flask(parts["flask"], m["glass"])
    cylinder(parts["flask"], "Synthetic liquid fixture", (0, 0, 0.014), 0.036, 0.019, m["water"], vertices=64, bevel=0)
    for i in range(3):
        angle = i * math.tau / 3
        part = parts[f"clamp_{i}"]
        part.parent = parts["platform"]
        part.location = (0.045*math.cos(angle), 0.045*math.sin(angle), 0.010)
        part.rotation_euler.z = angle
        cylinder(part, "Clamp hinge", (0, 0, 0.003), 0.004, 0.014, m["steel"], axis="Y", vertices=24)
        box(part, "Spring clamp finger", (-0.004, 0, 0.018), (0.003, 0.012, 0.030), m["steel"], 0.001)
        box(part, "Flask contact pad", (-0.008, 0, 0.032), (0.008, 0.015, 0.009), m["rubber"], 0.002)
    knob(parts["speed_dial"], m, (-0.110, -0.154, 0.048))
    front_screen(parts["screen"], m, (-0.021, -0.1505, 0.052), 0.085, 0.025)
    for key, x, surface in [("start_key", 0.067, m["blue"]), ("stop_key", 0.112, m["trim"])]:
        box(parts[key], "Control key", (x, -0.152, 0.048), (0.029, 0.004, 0.019), surface, 0.003)
    label(parts["body"], "OS 250", (-0.022, -0.1505, 0.078), 0.008, m["trim"])
    label(parts["body"], "ORBITAL SHAKER", (0.085, -0.1505, 0.076), 0.004, m["trim"])
    return root, parts


def model_graph(p, parts):
    # Axes are GLB Y-up; clamp local Blender +Y becomes GLB −Z.
    joints = [
        joint("eccentric", "orbit_rad", [0, 1, 0], 0, math.tau, "continuous"),
        joint("platform", "platform_rad", [0, 1, 0], -math.tau, 0, "continuous"),
        joint("speed_dial", "speed_dial_rad", [0, 0, -1], 0, math.tau*0.75, "revolute"),
    ] + [joint(f"clamp_{i}", "clamp_rad", [0, 0, -1], 0, 0.35, "revolute") for i in range(3)]
    return graph(
        p, parts, joints=joints,
        readouts=[("screen", "rpm", 0, "rpm")],
        controls={
            "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 25),
            "start_key": command("start"), "stop_key": command("stop"),
            **{f"clamp_{i}": command("clamp") for i in range(3)},
        },
        visibility=[("flask", "flask_visible")],
    )
