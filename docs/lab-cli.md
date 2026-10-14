# `lab` resource and instance workflow

This guide is self-contained for an agent that needs to discover an OpenSDL
asset, cache an immutable package, and run the supported local simulation.
Public Hub browsing and pulls do not require Docker, a Hub account, lab
hardware, or a running `lab serve` process.

## Install a runnable release

Install the pre-built release. It contains the `lab` binary, the local asset
daemon, the OpenUSD asset commands, and the kinematic runtime in one
executable; Docker, Rust, and a source checkout are not required:

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
and managed instance records. The asset daemon owns this cache and serializes
build, import, pull, push, tag, inspect, and garbage-collection operations. Use
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
directory, `catalog.sqlite`, or `daemon.json`.

## Discover and pull from Hub

The Test catalog is at <https://hub.test.sciol.ac.cn/>. Its catalog API is
separate from the OCI Registry used by `lab pull`:

```bash
curl --fail --silent --show-error \
  'https://hub.test.sciol.ac.cn/api/v1/assets?q=robot&limit=20'
curl --fail --silent --show-error \
  'https://hub.test.sciol.ac.cn/api/v1/assets/scienceol/elevrail-1-5m-v1-5167d813ec'
curl --fail --silent --show-error \
  'https://hub.test.sciol.ac.cn/api/v1/assets/scienceol/elevrail-1-5m-v1-5167d813ec/versions/1.0.0'
```

Copy the version detail's `pull_reference` in full. The web catalog host is
`hub.test.sciol.ac.cn`; the public OCI reference normally uses
`hub.sciol.ac.cn`. Save a digest reference for reproducible automation:

```bash
asset_ref='hub.sciol.ac.cn/scienceol/elevrail-1-5m-v1-5167d813ec@sha256:2f5734742ca8694602faead5517ef6cf68f0db399ea9bb0e9c160d58f2efc1f2'
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

`lab split` uses `usdcat` from the OpenUSD tools. It follows the LabReady
component workflow: each selected USD prim is extracted from the composed stage,
given its own `asset.yaml`, license, preview, thumbnail, and lineage pointing to
the parent package digest. The generated children are ordinary asset
directories, so every child goes through the same validate/build/push path:

```bash
lab split ./robot \
  --part arm=/Robot/Geometry/Arm \
  --part base=/Robot/Geometry/Base \
  --output ./robot-components

lab validate ./robot-components/arm
lab build ./robot-components/arm \
  --tag hub.sciol.ac.cn/acme/robot-arm:1.0.0
lab push hub.sciol.ac.cn/acme/robot-arm:1.0.0 \
  hub.sciol.ac.cn/acme/robot-arm:1.0.0
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
