"""Package verified committed model inputs and outputs for CI signing, without executing model scripts."""

import argparse
import hashlib
import json
from pathlib import Path
import tarfile

REPO = Path(__file__).resolve().parents[2]


def bundle(output):
    paths = set()
    for folder in sorted((REPO / "assets/instruments").iterdir()):
        if not folder.is_dir():
            continue
        package = folder / "package"
        provenance = json.loads((package / "provenance.json").read_text())
        for name, digest in provenance["inputs"].items():
            path = (REPO / name).resolve()
            if (
                not path.is_relative_to(REPO)
                or hashlib.sha256(path.read_bytes()).hexdigest() != digest
            ):
                raise ValueError(f"Unverified author input: {name}")
            paths.add(path)
        paths.update(p for p in package.rglob("*") if p.is_file())
    paths.add(REPO / "LICENSE")
    paths.add(REPO / "licenses/CC-BY-NC-4.0.txt")
    paths.add(REPO / "tools/instrument-models/LICENSING.md")
    with tarfile.open(output, "w", format=tarfile.PAX_FORMAT) as tar:
        for path in sorted(paths):
            info = tar.gettarinfo(str(path), arcname=str(path.relative_to(REPO)))
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ""
            with path.open("rb") as data:
                tar.addfile(info, data)
    return hashlib.sha256(Path(output).read_bytes()).hexdigest()


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--output", required=True, type=Path)
    a = p.parse_args()
    print(bundle(a.output))
