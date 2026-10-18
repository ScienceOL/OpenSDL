# SPDX-License-Identifier: CC-BY-NC-4.0
"""Original inclined flask, lifted glassware, open bath and hollow condenser coil."""
import math
import bpy
from mathutils import Vector
from geometry import box, cylinder, finish, label, vessel, bore_world
from bench_helpers import assembly, enclosure, knob, front_screen, graph
from author import joint, command, dial
from tubing import smooth_path, sweep


def lathe(parent, name, profile, surface):
    n = 48
    vertices = [
        (r * math.cos(i * math.tau / n), r * math.sin(i * math.tau / n), z)
        for r, z in profile
        for i in range(n)
    ]
    faces = [
        (
            j * n + i,
            j * n + (i + 1) % n,
            ((j + 1) % len(profile)) * n + (i + 1) % n,
            ((j + 1) % len(profile)) * n + i,
        )
        for j in range(len(profile))
        for i in range(n)
    ]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return finish(obj, parent, surface)


def flask(parent, m, r, neck, length):
    wall = 0.002
    theta = math.asin(neck / r)
    inner_theta = math.asin((neck - wall) / (r - wall))
    outer = [
        (r * math.sin(t), r * math.cos(t))
        for t in [math.pi - (math.pi - theta) * i / 20 for i in range(21)]
    ]
    inner = [
        ((r - wall) * math.sin(t), (r - wall) * math.cos(t))
        for t in [inner_theta + (math.pi - inner_theta) * i / 20 for i in range(21)]
    ]
    top = r * math.cos(theta) + length
    return lathe(
        parent,
        "Hollow round-bottom flask",
        outer + [(neck, top), (neck - wall, top)] + inner,
        m["glass"],
    )


def create_model(p):
    names = (
        "body",
        "carriage",
        "drive",
        "flask",
        "liquid",
        "condenser",
        "receiver",
        "clamp",
        "vent",
        "speed_dial",
        "temperature_dial",
        "screen",
        "sample_screen",
        "pressure_screen",
        "start_key",
        "stop_key",
        "heat_key",
        "vacuum_key",
        "lift_key",
    )
    root, parts, m = assembly(names)
    body = parts["body"]
    box(body, "Bench base", (0, 0, 0.006), (0.440, 0.340, 0.012), m["trim"], 0.005)
    cylinder(
        body, "Lift mast", (-0.080, 0.140, 0.310), 0.012, 0.580, m["steel"], vertices=40
    )
    box(
        body,
        "Mast support",
        (-0.080, 0.140, 0.035),
        (0.070, 0.050, 0.046),
        m["blue"],
        0.004,
    )
    cylinder(
        body,
        "Bath heater housing",
        (-0.080, -0.060, 0.030),
        0.105,
        0.036,
        m["shell"],
        vertices=64,
    )
    bath = vessel(
        body,
        m["steel"],
        dict(radius_m=0.095, height_m=0.130, wall_m=0.004, bottom_m=0.005),
    )
    bath.location = (-0.080, -0.060, 0.048)
    cylinder(
        body,
        "Bath water",
        (-0.080, -0.060, 0.093),
        0.089,
        0.080,
        m["water"],
        vertices=64,
        bevel=0,
    )
    box(
        body,
        "Control enclosure",
        (0.135, -0.070, 0.052),
        (0.145, 0.180, 0.080),
        m["shell"],
        0.005,
    )
    carriage = parts["carriage"]
    sweep(
        carriage,
        "Telescopic support sleeve",
        [Vector((-0.080, 0.140, z)) for z in [0.245, 0.550]],
        0.016,
        m["steel"],
        0.0125,
        sides=32,
    )
    collar = box(
        carriage,
        "Sliding mast carriage",
        (-0.080, 0.140, 0.265),
        (0.050, 0.043, 0.090),
        m["blue"],
        0.004,
    )
    bore_world(collar, (-0.080, 0.140, 0.265), 0.0125, 0.120)
    support = box(
        carriage,
        "Drive support arm",
        (-0.080, 0.085, 0.245),
        (0.035, 0.110, 0.030),
        m["steel"],
        0.003,
    )
    bore_world(support, (-0.080, 0.140, 0.245), 0.0125, 0.040)
    for name in ("drive", "flask", "liquid", "condenser", "receiver", "clamp"):
        parts[name].parent = carriage
    centre = Vector((-0.080, -0.060, 0.140))
    axis = Vector((0, 1, 1)).normalized()
    parts["drive"].location = centre
    parts["drive"].rotation_euler.x = -math.pi / 4
    sweep(
        parts["drive"],
        "Annular motor housing",
        [Vector((0, 0, z)) for z in [0.095, 0.165]],
        0.035,
        m["blue"],
        0.013,
        sides=32,
    )
    sweep(
        parts["drive"],
        "Vapor spindle bore",
        [Vector((0, 0, z)) for z in [0.085, 0.175]],
        0.009,
        m["glass"],
        0.007,
        sides=24,
    )
    for z in (0.098, 0.162):
        sweep(
            parts["drive"],
            "Spindle bearing",
            [Vector((0, 0, h)) for h in [z - 0.003, z + 0.003]],
            0.013,
            m["steel"],
            0.012,
            sides=24,
        )
    parts["flask"].location = centre
    parts["flask"].rotation_euler.x = -math.pi / 4
    flask(parts["flask"], m, 0.050, 0.012, 0.040)
    sweep(
        parts["flask"],
        "Rotating neck coupling",
        [Vector((0, 0, z)) for z in [0.075, 0.165]],
        0.012,
        m["steel"],
        0.009,
        sides=24,
    )
    box(
        parts["flask"],
        "Rotation index",
        (0.050, 0, 0),
        (0.001, 0.006, 0.010),
        m["blue"],
        0.0002,
    )
    parts["liquid"].location = centre
    r = 0.047
    if not 0 < p["sample_ml"] * 1e-6 < 4 * math.pi * r**3 / 3:
        raise ValueError("sample volume does not fit the flask bulb")
    lo, hi = 0, 2 * r
    for _ in range(50):
        h = (lo + hi) / 2
        if math.pi * h * h * (r - h / 3) < p["sample_ml"] * 1e-6:
            lo = h
        else:
            hi = h
    height = (lo + hi) / 2
    profile = [
        (math.sqrt(max(0, r * r - (-r + height * i / 32) ** 2)), -r + height * i / 32)
        for i in range(33)
    ] + [(0, -r + height)]
    lathe(parts["liquid"], "Horizontal synthetic liquid", profile, m["water"])
    parts["clamp"].parent = parts["drive"]
    parts["clamp"].location = (0, 0, 0.089)
    sweep(
        parts["clamp"],
        "Neck collet",
        [Vector((0, 0, z)) for z in [-0.004, 0.004]],
        0.021,
        m["white"],
        0.012,
        sides=24,
    )
    box(
        parts["clamp"],
        "Collet release wing",
        (0.022, 0, 0),
        (0.020, 0.010, 0.007),
        m["blue"],
        0.001,
    )
    parts["condenser"].location = (0.065, 0.090, 0)
    jacket = sweep(
        parts["condenser"],
        "Hollow condenser jacket",
        [Vector((0, 0, z)) for z in [0.300, 0.580]],
        0.026,
        m["glass"],
        0.024,
        sides=40,
    )
    vapor = sweep(
        parts["condenser"],
        "Vapor passage",
        [Vector((0, 0, z)) for z in [0.290, 0.590]],
        0.008,
        m["glass"],
        0.006,
        sides=24,
    )
    coil = [
        Vector(
            (
                0.016 * math.cos(math.pi / 2 + i * math.tau / 24),
                0.016 * math.sin(math.pi / 2 + i * math.tau / 24),
                0.315 + i * 0.200 / 192,
            )
        )
        for i in range(193)
    ]
    sweep(
        parts["condenser"],
        "Hollow cooling coil",
        coil,
        0.0025,
        m["glass"],
        0.0015,
        sides=8,
    )
    for z in (0.300, 0.580):
        sweep(
            parts["condenser"],
            "Jacket end seal",
            [Vector((0, 0, h)) for h in [z - 0.002, z + 0.002]],
            0.026,
            m["white"],
            0.008,
            sides=32,
        )
    bore_world(vapor, (0.058, 0.090, 0.298), 0.007, 0.026, "X")
    bore_world(vapor, (0.065, 0.098, 0.587), 0.003, 0.020, "Y")
    cylinder(
        parts["condenser"],
        "Vapor passage cap",
        (0, 0, 0.591),
        0.008,
        0.002,
        m["glass"],
        vertices=24,
    )
    for z in (0.315, 0.515):
        bore_world(jacket, (0.065, 0.115, z), 0.004, 0.020, "Y")
        sweep(
            parts["condenser"],
            "Cooling water port",
            [Vector((0, y, z)) for y in [0.016, 0.050]],
            0.004,
            m["glass"],
            0.0025,
            sides=16,
        )
    parts["receiver"].location = (0.065, 0.090, 0.215)
    flask(parts["receiver"], m, 0.043, 0.010, 0.035)
    sweep(
        parts["condenser"],
        "Receiver neck joint",
        [Vector((0, 0, z)) for z in [0.280, 0.300]],
        0.012,
        m["white"],
        0.010,
        sides=24,
    )
    start = centre + axis * 0.175
    path = smooth_path(
        [
            tuple(start),
            (-0.060, 0.085, 0.285),
            (0.020, 0.090, 0.290),
            (0.065, 0.090, 0.300),
        ]
    )
    sweep(
        carriage,
        "Continuous hollow vapor connector",
        path,
        0.009,
        m["glass"],
        0.007,
        sides=16,
    )
    for z in (0.330, 0.530):
        bracket = box(
            carriage,
            "Condenser support bracket",
            (0.005, 0.140, z),
            (0.190, 0.046, 0.012),
            m["steel"],
            0.002,
        )
        bore_world(bracket, (-0.080, 0.140, z), 0.0125, 0.030)
        sweep(
            carriage,
            "Condenser clamp band",
            [Vector((0.065, 0.090, h)) for h in [z - 0.004, z + 0.004]],
            0.030,
            m["blue"],
            0.026,
            sides=24,
        )
    parts["vent"].parent = carriage
    parts["vent"].location = (0.065, 0.140, 0.587)
    sweep(
        carriage,
        "Vacuum port sidearm",
        [Vector((0.065, y, 0.587)) for y in [0.096, 0.140]],
        0.004,
        m["glass"],
        0.003,
        sides=16,
    )
    sweep(
        carriage,
        "Supported vacuum valve body",
        [Vector((0.065, y, 0.587)) for y in [0.130, 0.155]],
        0.007,
        m["steel"],
        0.003,
        sides=24,
    )
    cylinder(
        parts["vent"], "Vent spindle", (0, 0, 0), 0.005, 0.015, m["steel"], vertices=24
    )
    box(
        parts["vent"],
        "Vent lever",
        (0.012, 0, 0.010),
        (0.035, 0.010, 0.006),
        m["blue"],
        0.001,
    )
    for name, x, z in [
        ("screen", 0.100, 0.068),
        ("sample_screen", 0.160, 0.068),
        ("pressure_screen", 0.130, 0.038),
    ]:
        front_screen(parts[name], m, (x, -0.1605, z), 0.050, 0.018)
    knob(parts["speed_dial"], m, (0.092, -0.165, 0.018), radius=0.009)
    knob(parts["temperature_dial"], m, (0.178, -0.165, 0.018), radius=0.009)
    for name, x, y in [
        ("start_key", 0.090, -0.100),
        ("stop_key", 0.135, -0.100),
        ("heat_key", 0.180, -0.100),
        ("vacuum_key", 0.105, -0.040),
        ("lift_key", 0.165, -0.040),
    ]:
        box(
            parts[name],
            "Top control key",
            (x, y, 0.094),
            (0.030, 0.024, 0.005),
            m["blue"] if name != "stop_key" else m["trim"],
            0.002,
        )
    label(body, "RE 500", (0.130, -0.1605, 0.085), 0.007, m["trim"])
    return root, parts


def model_graph(p, parts):
    return graph(
        p,
        parts,
        joints=[
            joint("carriage", "carriage_m", [0, 1, 0], 0, p["lift_m"], "prismatic"),
            joint("flask", "flask_rad", [0, 1, 0], 0, math.tau, "continuous"),
            joint("clamp", "clamp_rad", [0, 1, 0], 0, 0.6, "revolute"),
            joint("vent", "vent_rad", [0, 1, 0], 0, math.pi / 2, "revolute"),
            joint(
                "speed_dial",
                "speed_dial_rad",
                [0, 0, -1],
                0,
                math.tau * 0.75,
                "revolute",
            ),
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
            ("pressure_screen", "pressure_kpa", 1, "kPa"),
        ],
        controls={
            "clamp": command("clamp"),
            "vent": command("vent"),
            "speed_dial": dial("set_speed", "rpm", "target_rpm", 0, p["max_rpm"], 10),
            "temperature_dial": dial(
                "set_temperature",
                "temperature_c",
                "target_c",
                p["ambient_c"],
                p["max_c"],
                5,
            ),
            "start_key": command("start"),
            "stop_key": command("stop"),
            "heat_key": command("heat"),
            "vacuum_key": command("start_vacuum"),
            "lift_key": command("lift"),
        },
        visibility=[("flask", "flask_visible"), ("liquid", "liquid_visible")],
    )
