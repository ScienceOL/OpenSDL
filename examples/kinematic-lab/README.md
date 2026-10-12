# Kinematic Lab example

This runnable package uses the built-in OpenSDL kinematic world. Build the
current source CLI; these managed commands are not in the public release yet.
Run from the OpenSDL repository root and choose output paths that do not exist:

```bash
lab validate examples/kinematic-lab
lab pack examples/kinematic-lab --output ./kinematic-layout
lab asset import ./kinematic-layout --tag localhost/lab/kinematic:1.0.0 --cache-dir ./lab-store
lab instance run localhost/lab/kinematic:1.0.0 --name demo --cache-dir ./lab-store
lab instance devices demo --json --cache-dir ./lab-store
lab instance send demo sim:example-lab:heater-1 start --cache-dir ./lab-store
lab instance logs demo --tail 10 --cache-dir ./lab-store
lab instance stop demo --cache-dir ./lab-store
lab instance start demo --cache-dir ./lab-store
lab instance stop demo --cache-dir ./lab-store
lab instance remove demo --cache-dir ./lab-store
lab asset remove localhost/lab/kinematic:1.0.0 --cache-dir ./lab-store
lab asset prune --dry-run --json --cache-dir ./lab-store
```

The OpenUSD/GLB cube is a preview. Device behavior comes from `world.yaml` and
the built-in kinematic backend; no rigid-body or robot physics is implied.
