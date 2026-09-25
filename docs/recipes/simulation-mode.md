# Local simulation mode

OpenSDL can boot a deterministic virtual lab when no physical devices are
connected. The virtual devices use the normal `Device`, `DeviceCommand`,
`DeviceStatus`, event, and gRPC contracts, so a Runner, Agent, or UI can be
developed against them without a hardware-specific code path.

## Quick start

```bash
cargo run --bin lab -- serve --simulation
```

The default world exposes a virtual heater, magnetic stirrer, valve, and
temperature probe. Inspect them from another shell:

```bash
cargo run --bin lab -- device list
cargo run --bin lab -- send sim:lab-sim:heater-1 set_temperature \
  --params '{"temperature":80}'
cargo run --bin lab -- send sim:lab-sim:heater-1 start
```

`--simulation` is equivalent to `OSDL_SIMULATION=true`. It is a local
development mode and does not discover or control physical hardware.

## A custom world

Use [`configs/simulation.yaml`](configs/simulation.yaml) as a starting point:

```bash
cargo run --bin lab -- serve --config docs/recipes/configs/simulation.yaml
```

`engine: kinematic` is the deterministic backend shipped with OpenSDL. The
configuration already reserves the runtime boundary for `rapier`, `bullet`,
`mujoco`, `isaac-sim`, and `gazebo`; those engines must be provided by a
validated local backend adapter before they can be selected. OpenSDL fails
with an explicit error for an unavailable engine instead of silently changing
the experiment's semantics.

The React + Three workbench treats the simulation snapshot as an observer and
action surface. Physics remains authoritative in the local OpenSDL runtime;
the browser does not become a second physics engine. This makes an eventual
desktop Runner integration and a headless lab host share the same world
state.
