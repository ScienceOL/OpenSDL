"""Build one repository-owned instrument author source; no author code is shipped to the runtime."""

import importlib.util
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).parent))
from author import build

if __name__ == "__main__":
    index = sys.argv.index("--") + 1
    slug = sys.argv.pop(index)
    if not re.fullmatch(r"[a-z][a-z0-9-]+", slug):
        raise ValueError("invalid instrument directory")
    source = Path(__file__).resolve().parents[2] / "assets/instruments" / slug
    spec = importlib.util.spec_from_file_location(
        "original_instrument_author", source / "geometry.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    role = json.loads((source / "model.yaml").read_text())["role"]

    def runtime(p):
        return p["actions"], p["runtime"], p.get("samples", [])

    build(source, module.create_model, module.model_graph, runtime, slug, role)
