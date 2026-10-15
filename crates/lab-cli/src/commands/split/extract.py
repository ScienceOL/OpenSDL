"""Embedded OpenUSD adapter. Rust owns manifests and atomic publication."""
import glob
import json
from pathlib import Path
import shutil
import sys

try:
    from pxr import Sdf, Usd, UsdUtils
except ImportError:
    sys.exit("OpenUSD Python is missing; install usd-core==25.11 in a Python 3.12 venv and pass lab split --python <venv-python>")


def extract(source, asset_root, output, parts):
    stage = Usd.Stage.Open(str(source))
    if not stage:
        raise ValueError(f"cannot open USD stage: {source}")
    for name, prim in parts:
        path = Sdf.Path(prim)
        if not path.IsAbsolutePath() or not path.IsPrimPath() or not stage.GetPrimAtPath(path):
            raise ValueError(f"USD prim does not exist in the composed stage: {prim}")

    for name, prim in parts:
        directory = output / name / "source"
        directory.mkdir(parents=True)
        mask = Usd.StagePopulationMask()
        mask.Add(Sdf.Path(prim))
        selected = Usd.Stage.OpenMasked(str(source), mask)
        # Keep referenced materials and other relationship targets, not just geometry.
        selected.ExpandPopulationMask()
        layer = selected.Flatten(addSourceFileComment=False)
        layer.defaultPrim = Sdf.Path(prim).GetPrefixes()[0].name
        failures = []

        def copy_asset(authored):
            if not authored:
                return authored
            path = Path(authored)
            if not path.is_absolute():
                path = source.parent / path
            path = path.resolve()
            try:
                relative = path.relative_to(asset_root)
                matches = sorted(glob.glob(str(path).replace("<UDIM>", "[0-9][0-9][0-9][0-9]"))) if "<UDIM>" in str(path) else [str(path)]
                if not matches:
                    raise ValueError("no matching texture tiles")
                for match in matches:
                    file = Path(match).resolve()
                    file_relative = file.relative_to(asset_root)
                    if not file.is_file():
                        raise ValueError("not a regular file")
                    target = directory / "dependencies" / file_relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(file, target)
                return (Path("dependencies") / relative).as_posix()
            except (ValueError, OSError) as error:
                failures.append(f"non-portable USD dependency {authored}: {error}")
                return authored

        UsdUtils.ModifyAssetPaths(layer, copy_asset)
        if failures:
            raise ValueError("; ".join(failures))
        if not layer.Export(str(directory / f"{name}.usda")):
            raise ValueError(f"cannot export component: {name}")


try:
    extract(Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), json.loads(sys.argv[4]))
except Exception as error:
    sys.exit(str(error))
