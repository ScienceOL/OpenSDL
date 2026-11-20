#!/usr/bin/env python3
"""Print SC9300 identity as an OpenSDL read-only probe."""
import json
from pathlib import Path
import subprocess
import sys


def main():
    executable, port = sys.argv[1:]
    result = subprocess.run(
        [executable, str(Path(port).resolve(strict=True)), "info"],
        capture_output=True,
        text=True,
        timeout=4,
        check=True,
    )
    names = {"device_type", "device_id", "firmware"}
    properties = {}
    for line in result.stdout.splitlines():
        key, _, value = (part.strip() for part in line.partition(":"))
        if key in names and value:
            properties[key] = value
    if properties.keys() != names:
        raise ValueError("SC9300 did not return a complete identity")
    print(json.dumps(properties))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
