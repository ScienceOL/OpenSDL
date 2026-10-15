"""OpenSDL declarations generated from the authored balance parameters."""
import json


def write_config(out, p):
    def action(name, description, properties=None, required=None):
        return {"name": name, "description": description,
                "params": {"type": "object", "properties": properties or {},
                           "required": required or [], "additionalProperties": False}}
    actions = [action("reset", "Reset this simulated instrument and release its fixtures"), action("power", "Toggle balance power"), action("tare", "Tare the stable pan load"),
               action("zero", "Zero a stable unloaded balance"),
               action("set_door", "Move a draft-shield door", {"door": {"type": "string", "enum": ["left", "right", "top"]},
                      "opening_m": {"type": "number", "minimum": 0, "maximum": p["door_travel_m"]}}, ["door", "opening_m"]),
               action("load_sample", "Place a fixture on the pan", {"sample_id": {"type": "string", "enum": list(p["fixtures"])}}, ["sample_id"]),
               action("unload_sample", "Remove a fixture from the pan", {"sample_id": {"type": "string", "enum": list(p["fixtures"])}}, ["sample_id"])]
    world = {"world_id": "ax240", "engine": "kinematic", "tick_hz": 10, "seed": 0,
             "samples": [{"id": name, **{key: fixture[key] for key in ("mass_kg", "position_m", "radius_m")}}
                         for name, fixture in p["fixtures"].items()],
             "devices": [{"id": "balance-1", "device_type": "simulation.analytical_balance", "role": "balance",
                          "description": p["name"], "actions": actions, "position": [0, 0, 0],
                          "asset_ref": {"namespace": "opensdl", "name": "analytical-balance", "version": p["version"]},
                          "model": {"kind": "analytical_balance", "model_id": p["id"],
                                    **{k: p[k] for k in ("capacity_kg", "readability_kg", "settling_s", "door_travel_m", "pan_radius_m")}}}]}
    manifest = {"apiVersion": "assets.opensdl.org/v1alpha1", "kind": "VirtualAsset",
                "metadata": {"name": "analytical-balance", "displayName": p["name"], "version": p["version"],
                             "description": "Original metre-scale analytical balance with three sliding glass doors and a local weighing model.",
                             "tags": ["original", "balance", "metrology"], "license": {"spdx": p["license"], "file": "LICENSE"}},
                "spec": {"type": "instrument", "source": {"format": "openusd", "root": "source/model.usda"},
                         "preview": {"model": "preview.glb", "thumbnail": "thumbnail.png"},
                         "capabilities": ["visual", "simulation", "opensdl-device"],
                         "bindings": {"deviceTypes": ["simulation.analytical_balance"]},
                         "compatibility": {"runtimes": [{"name": "opensdl", "versions": [">=0.3.2"], "status": "expected"}]},
                         "runtime": {"engine": "kinematic", "config": "source/world.yaml"}}}
    for name, value in (("asset.yaml", manifest), ("source/world.yaml", world)):
        (out / name).write_text(json.dumps(value, indent=2) + "\n")
