# SPDX-License-Identifier: MIT
"""Reusable original swept annular tubing; ends retain an open lumen."""
import math
import bpy
from mathutils import Vector
from geometry import finish


def smooth_path(points):
    points = [Vector(v) for v in points]
    result = []
    for i in range(len(points) - 1):
        a, b, c, d = (
            points[max(0, i - 1)],
            points[i],
            points[i + 1],
            points[min(len(points) - 1, i + 2)],
        )
        for j in range(6):
            t = j / 6
            result.append(
                (
                    2 * b
                    + (-a + c) * t
                    + (2 * a - 5 * b + 4 * c - d) * t * t
                    + (-a + 3 * b - 3 * c + d) * t * t * t
                )
                * 0.5
            )
    return result + [points[-1]]


def sweep(parent, name, points, outer, surface, inner=None, sides=16):
    vertices = []
    rings = [outer] if inner is None else [outer, inner]
    for radius in rings:
        for i, point in enumerate(points):
            tangent = (
                points[min(i + 1, len(points) - 1)] - points[max(0, i - 1)]
            ).normalized()
            reference = Vector((0, 1, 0)) if abs(tangent.y) < 0.9 else Vector((0, 0, 1))
            normal = (
                tangent.cross(reference).normalized()
                if i == 0
                else previous - tangent * previous.dot(tangent)
            )
            if normal.length < 1e-8:
                normal = tangent.cross(reference)
            normal.normalize()
            previous = normal.copy()
            binormal = tangent.cross(normal)
            vertices += [
                point
                + radius
                * (
                    normal * math.cos(k * math.tau / sides)
                    + binormal * math.sin(k * math.tau / sides)
                )
                for k in range(sides)
            ]
    faces = []
    stride = len(points) * sides
    for layer in range(len(rings)):
        offset = layer * stride
        for i in range(len(points) - 1):
            for k in range(sides):
                face = (
                    offset + i * sides + k,
                    offset + i * sides + (k + 1) % sides,
                    offset + (i + 1) * sides + (k + 1) % sides,
                    offset + (i + 1) * sides + k,
                )
                faces.append(face if layer == 0 else tuple(reversed(face)))
    for i in (0, len(points) - 1):
        if inner is None:
            centre = len(vertices)
            vertices.append(points[i])
            for k in range(sides):
                face = (centre, i * sides + k, i * sides + (k + 1) % sides)
                faces.append(tuple(reversed(face)) if i == 0 else face)
        else:
            for k in range(sides):
                face = (
                    i * sides + k,
                    stride + i * sides + k,
                    stride + i * sides + (k + 1) % sides,
                    i * sides + (k + 1) % sides,
                )
                faces.append(face if i == 0 else tuple(reversed(face)))
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return finish(obj, parent, surface)
