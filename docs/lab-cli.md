# `lab` resource and instance workflow

This guide is self-contained for an agent that needs to discover an OpenSDL
asset, cache an immutable package, and run the supported local simulation.
Public Hub browsing and pulls do not require Docker, a Hub account, lab
hardware, or a running `lab serve` process.

## Install a runnable release

Install the pre-built release. It contains the `lab` binary, the local asset
daemon, the OpenUSD asset commands, and the kinematic runtime in one
executable; Docker, Rust, and a source checkout are not required. USD splitting
additionally uses the OpenUSD Python runtime installed below:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ScienceOL/OpenSDL/releases/latest/download/lab-cli-installer.sh | sh
```

On Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/ScienceOL/OpenSDL/releases/latest/download/lab-cli-installer.ps1 | iex"
```

Check the installed command before starting a workflow:

```bash
lab --version
lab --help
lab daemon --help
lab build --help
lab split --help
lab run --help
lab ps --help
```

If `daemon`, `build`, or `split` are missing, run the installer again. A source
build is only needed for development:

```bash
git clone https://github.com/ScienceOL/OpenSDL.git
cd OpenSDL
cargo install --locked --path crates/lab-cli
```

Rust 1.89 or newer and the platform's C/C++ build tools are required for a
source build. The installer installs the executable only; device schemas and
firmware remain separate hardware assets.

## Start the local asset daemon

The resource cache contains content-addressed package blobs, local references,
and managed instance records. The asset daemon accepts build, import, pull, push, tag, inspect, and
garbage-collection operations. Only store transactions hold the cross-process
cache lease; uploads run outside it. Use
the same cache for every command in one workflow:

```bash
mkdir -p opensdl-demo
cd opensdl-demo
export LAB_ASSET_CACHE="$PWD/lab-cache"

lab daemon start
lab daemon status --json
```

An explicit `--cache-dir` overrides `LAB_ASSET_CACHE` for one command. `lab
daemon start` detaches a loopback daemon and `lab daemon stop` stops it.
Commands fall back to direct cache access when the daemon is not running, so an
offline script remains usable. Do not manually delete the cache's `instances`
directory, `catalog.sqlite`, or `daemon.json`. The daemon authenticates local
RPC using a private per-start token, keeps diagnostics in `daemon.log`, and
drains accepted operations before acknowledging stop. An unresponsive daemon
returns an error instead of silently switching execution paths. It is a
per-user background process, not an automatically installed boot service.

Before upgrading, run `lab daemon stop` with the old executable, update with
the installer, then run `lab daemon start` with the new executable. Stop any
managed instances before replacing the binary on Windows.

## Discover and pull from Hub

The Test catalog is at <https://hub.test.liyanlabs.com/>. Its catalog API is
separate from the OCI Registry used by `lab pull`:

```bash
curl --fail --silent --show-error \
  'https://hub.test.liyanlabs.com/api/v1/assets?q=robot&limit=20'
curl --fail --silent --show-error \
  'https://hub.test.liyanlabs.com/api/v1/assets/scienceol/elevrail-1-5m-v1-5167d813ec'
curl --fail --silent --show-error \
  'https://hub.test.liyanlabs.com/api/v1/assets/scienceol/elevrail-1-5m-v1-5167d813ec/versions/1.0.0'
```

Copy the version detail's `pull_reference` in full. The web catalog host is
`hub.test.liyanlabs.com`; the public OCI reference normally uses
`hub.liyanlabs.com`. New publications use this primary registry host; existing stored references retain their recorded authority. Save a digest reference for reproducible automation:

```bash
asset_ref='hub.liyanlabs.com/scienceol/elevrail-1-5m-v1-5167d813ec@sha256:2f5734742ca8694602faead5517ef6cf68f0db399ea9bb0e9c160d58f2efc1f2'
lab pull "$asset_ref" --output ./assets/elevrail
lab images --json
lab inspect ./assets/elevrail --json
lab validate ./assets/elevrail
```

`lab pull` verifies the manifest digest, config, and all blobs before the
package is admitted to the cache. `--output` must name a new directory and is
never an overwrite operation. Omit `--output` to cache only. A complete cache
can be reused without network access:

```bash
lab pull "$asset_ref" --offline --output ./assets/elevrail-offline
```

An offline failure identifies the missing blob; repeat one online pull with
the same cache and retry.

## Build an asset image

The public Hub catalog currently contains visual assets. The repository's
`examples/kinematic-lab` is the supported runnable package for validating the
instance lifecycle:

```bash
cd /path/to/OpenSDL
lab validate examples/kinematic-lab
lab build examples/kinematic-lab \
  --tag localhost/lab/kinematic:1.0.0 \
  --output "$OLDPWD/kinematic-layout"
lab images --json
```

`lab build` is the Docker-like boundary: it validates the directory, creates a
deterministic OCI image layout, imports all blobs into the daemon's local
content-addressed store, and assigns the optional tag. `lab pack` remains
available when an explicit OCI layout directory is required.

The package must declare `simulation` and the `kinematic` runtime in
`asset.yaml`:

```yaml
spec:
  capabilities: [visual, scene, simulation]
  runtime:
    engine: kinematic
    config: world.yaml
```

Visual-only packages are valid assets but are rejected as managed instances.
The first runtime is deterministic state evolution; it does not provide OS
container isolation, arbitrary process execution, rigid-body physics, or
automatic cloud/device fallback.

## Split an OpenUSD scene into child assets

`lab split` uses the OpenUSD Python API to resolve complete prim paths, flatten
selected subtrees and their relationship targets, and copy/rebase referenced
textures inside each child. It writes all children atomically; a missing prim
or non-portable dependency leaves the output absent, so a corrected command
can reuse the same destination. Names use lowercase letters, digits and
interior hyphens. The source asset must contain all of its dependencies.

Install Python 3.12, then prepare this optional runtime once (the path can be
anywhere on the local device). macOS/Linux:

```bash
python3.12 -m venv "$HOME/.local/share/opensdl-usd"
"$HOME/.local/share/opensdl-usd/bin/python" -m pip install usd-core==25.11
export LAB_USD_PYTHON="$HOME/.local/share/opensdl-usd/bin/python"
```

Windows PowerShell:

```powershell
py -3.12 -m venv "$env:LOCALAPPDATA/opensdl-usd"
& "$env:LOCALAPPDATA/opensdl-usd/Scripts/python.exe" -m pip install usd-core==25.11
$env:LAB_USD_PYTHON = "$env:LOCALAPPDATA/opensdl-usd/Scripts/python.exe"
```

Alternatively pass `--python /absolute/path/to/python` on each split command.
Build, push, pull, daemon and kinematic instances do not need Python. The
adapter is embedded in the release binary; no OpenSDL checkout is needed.

The component mapping follows the explicit semantic extraction pattern in
[ScienceOL/LabReady at 3080711](https://github.com/ScienceOL/LabReady/blob/30807110528fcc959dca20d54c860242bf01e31b/experiments/centrifuge_03/build_labready.py),
whose `build_layers` and `_copy` resolve and validate exact prim paths. This is
an OpenUSD implementation, not a call to an assumed LabReady split API.
Children carry their own `asset.yaml`, license, inherited preview/thumbnail
and parent-digest lineage. Replace the example paths below with prim paths
that exist in your own source stage:

```bash
lab split ./robot \
  --part arm=/Robot/Geometry/Arm \
  --part base=/Robot/Geometry/Base \
  --output ./robot-components

lab validate ./robot-components/arm
lab build ./robot-components/arm \
  --tag hub.liyanlabs.com/acme/robot-arm:1.0.0
lab push hub.liyanlabs.com/acme/robot-arm:1.0.0 \
  hub.liyanlabs.com/acme/robot-arm:1.0.0
```

The first argument to `lab push` can be a local tag or digest, or an OCI layout
directory. The second argument is always the remote tag. `lab split` currently
requires an OpenUSD source and an explicit component-to-prim mapping. It does
not guess semantic parts or generate a new GLB preview; provide a reviewed
preview asset when a child needs a component-specific online preview.

## Run and operate an instance

Top-level commands are shortcuts over the `instance` and `asset` command
groups. They share one catalog, digest, lease, and recovery implementation:

```bash
lab run localhost/lab/kinematic:1.0.0 --name demo --json
lab ps --json
lab logs demo --tail 20
lab instance devices demo --json
lab instance send demo sim:example-lab:heater-1 start --json
lab stop demo --json
lab start demo --json
lab stop demo --json
lab rm demo
```

Creation freezes the resolved manifest digest and `world.yaml`. Each instance
has separate runtime data, logs, loopback gRPC endpoint, and authentication
token. A stopped instance still leases its package, so stopping alone does not
make the package eligible for GC. A crash is retained as a `failed` record;
inspect its logs, then restart a valid prepared instance with `lab start`.

`lab stop NAME` addresses a managed instance. `lab stop` without a name keeps
the server command's existing behavior and shuts down a standalone `lab serve`
process.

## Diagnose and reclaim

```bash
lab ps --all --json
lab instance inspect demo --json
lab logs demo --tail 200
```

Stop before removing a running instance. If deletion is interrupted, the
`removing` record remains; repeat `lab rm demo` to continue. Once the instance
is removed, remove local tags and inspect the reclaim plan:

```bash
lab asset remove localhost/lab/kinematic:1.0.0
lab asset prune --dry-run --json
lab asset prune --json
```

Prune deletes unreferenced and unoccupied packages and only removes a shared
blob when no retained package still needs it. Digest references, active tags,
and instance leases prevent accidental reclamation.
