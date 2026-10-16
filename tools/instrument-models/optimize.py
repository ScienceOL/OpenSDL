"""Merge rigid meshes by actual component/material; animated parents and LCD ownership stay separate."""

from collections import defaultdict
import bpy


def consolidate(root):
    groups = defaultdict(list)
    for obj in root.children_recursive:
        if obj.type == "MESH" and len(obj.data.materials) == 1:
            groups[(obj.parent, obj.data.materials[0])].append(obj)
    for (parent, _), objects in groups.items():
        if len(objects) < 2:
            continue
        bpy.ops.object.select_all(action="DESELECT")
        for obj in objects:
            obj.select_set(True)
        bpy.context.view_layer.objects.active = objects[0]
        bpy.ops.object.convert(target="MESH")
        bpy.ops.object.join()
        bpy.context.object.parent = parent
