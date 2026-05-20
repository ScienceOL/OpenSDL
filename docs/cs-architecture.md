# OpenSDL Client/Server Architecture

This document describes the post-refactor C/S architecture: how the
engine, the gRPC server, and the CLI compose, what ships in each crate,
and how the same code runs in three different deployment modes.

If you want a higher-level orientation, start with
[`architecture.md`](architecture.md) (the engine's transport layer +
hardware path) and then come back here.

## Why C/S

The engine has always been a long-running, async, stateful thing —
broker, mDNS, mediamtx, ESP-NOW gateways, SQLite event store, dozens of
in-flight devices. We needed three deployment shapes:

1. **Local box** — engine + client on the same lab machine.
2. **Remote** — client on a workstation, engine on the lab Pi.
3. **Bundled** — desktop app embeds the engine in-process for the agent.

The dominant constraint was that all three modes have to run the same
code paths. So we split the codebase into four crates with the engine
sitting under a thin gRPC adapter:

```
crates/
├── osdl-core/      engine, transports, adapters, registry, store
├── osdl-proto/     protobuf schema + generated client/server stubs
├── osdl-server/    gRPC service wrapping the engine + lifecycle/lockfile/paths
└── osdl-cli/       clap CLI: `serve` boots the engine, others are gRPC clients
```

`osdl-server` is a *library*. The CLI's `osdl serve` calls it; the
desktop bundle in mode 3 also calls it (in-process, against an
EngineHandle the agent already holds). Two consumers, one
implementation.

## Engine: handle + loop

`OsdlEngine` was previously a single `&mut self` thing where `run()`
consumed unique receivers. That doesn't compose with multiple gRPC
subscribers. We split it into:

- **`OsdlEngine`** — owns the loop. Holds the per-loop `mpsc` receivers
  (transport RX, command injection, ESP-NOW REG events). One
  `engine.run().await` per process.
- **`EngineHandle`** — `Clone + Send + Sync`, holds `Arc<RwLock<…>>` for
  the device/node/transport tables, the broadcast event channel, status
  and stop watches, and the inject-command sender. The gRPC service
  stores one of these. So does the orchestrator. So does the CLI's
  command-injection helper.

The events channel is a `tokio::sync::broadcast` (`EVENT_BROADCAST_CAP =
256`). Many subscribers can attach simultaneously; subscribers that fall
too far behind get `RecvError::Lagged(n)` and the gRPC service
synthesizes a `LaggedEvent` so clients learn they missed something.
There is no global head-of-line blocking from a slow consumer.

```
gRPC server  ─┐
CLI inject   ─┼─ EngineHandle.clone() ─→ shared state (RwLock)
orchestrator ─┘                       └→ broadcast::Sender (events)
                                       └→ watch::Sender (status, stop)

OsdlEngine.run() ── consumes ── mpsc::Receiver (transport rx, cmd inject, REG)
                              ── reads ──── EngineHandle's adapters/devices
                              ── writes ─── EngineHandle's tables, broadcast
```

## gRPC surface (osdl-proto)

`crates/osdl-proto/proto/osdl.proto` is the wire contract. Service
methods today:

| RPC | Purpose |
|---|---|
| `Status` | Server identity (instance, pid, sockets, version) + engine state snapshot. Cheap polling. |
| `ListNodes` / `ListDevices` / `GetDevice` | Inspect known nodes/devices. `ListDevices` filters by adapter / device_type / role. |
| `WaitForDevice` | Block until a device matching `device_id` / `device_type` / `role` appears, with a deadline. |
| `SendCommand` | Encode + dispatch a single action via the device's adapter and transport. |
| `StreamEvents` | Server-streaming OsdlEvent feed; supports per-kind filtering. Synthetic `LaggedEvent` emitted when the per-subscriber buffer overflowed. |
| `Shutdown` | Ask the engine to stop. |

Wire types are deliberately *separate* from `osdl_core::protocol::*`.
`osdl-server::convert` translates at the boundary, so internal type
refactors don't break the API contract. Notable conversion behavior:
JSON ↔ `prost_types::Struct` round-tripping coerces whole-number floats
back to integers (matches grpc-gateway), since adapters frequently
branch on `as_u64()` — without this, a CLI `position=3` becomes `3.0`
on the wire and breaks the runze codec.

The build script uses `tonic-prost-build` (tonic 0.14 split prost into a
separate crate) and the `protoc-bin-vendored` binary, so contributors
don't need to install `protoc` separately.

## Server lifecycle (osdl-server)

`osdl_server::serve(handle: EngineHandle, cfg: ServeConfig)` is the
entrypoint. In execution order:

1. **Bind listeners up-front.** TCP and UDS both bind *before* writing
   the lockfile. This way `--listen 127.0.0.1:0` records the
   kernel-assigned port (e.g. `:62968`) — clients reading the lockfile
   get an address they can actually connect to.
2. **Reserve the lockfile.** Write
   `runtime_dir/instances/<NAME>.json`. Refuse to clobber a live
   lockfile (PID alive); overwrite stale ones (PID gone). The guard is
   RAII — it deletes the file on drop.
3. **Build the tonic service.** Two listener tasks (one per kind), both
   notified by a shared `tokio::sync::Notify` on shutdown. Per-listener
   `tonic::transport::Server::builder().serve_with_incoming_shutdown`.
4. **Wait on stop.** Either the engine asks to stop (`engine.request_stop()`)
   or the OS sends Ctrl-C. Whichever fires first triggers `notify_waiters`
   on both listeners.
5. **Drain.** Await each listener's join handle, clean up the UDS file,
   drop the lockfile guard.

UDS files are chmod'd to 0600 so other users on the host can't connect.
The runtime dir itself is 0700 (created by the `paths` module).

## Multi-instance discovery (the VS Code pattern)

Each running `osdl serve` writes a JSON descriptor to a shared runtime
dir. The lockfile contains: instance name, pid, version, started_at,
socket path, listen addr.

Client-side endpoint resolution (in priority order):
1. `--endpoint` flag / `OSDL_ENDPOINT` env (explicit, no discovery).
2. `--instance NAME` / `OSDL_INSTANCE` (lookup by name).
3. Auto-discovery: scan the lockfile dir, prune dead PIDs, pick the
   unique live entry. If there are multiple, error with a list and
   require `--instance`. If there are none, error with a hint.

```
$ osdl status
osdl: multiple osdl servers running (chinwe, dev) — pick one with --instance NAME

$ osdl --instance chinwe status
instance:    chinwe
version:     0.1.0
pid:         12345
socket:      /var/folders/.../osdl-501/chinwe.sock
engine:      CONNECTED (broker=mqtt-disabled)
nodes:       0
devices:     5
```

Stale PIDs (dead processes) are GC'd as a side effect of `list()` /
`find()`. Liveness is `kill(pid, 0)` on Unix.

## Storage paths

Resolved per platform via the `directories` crate:

| Purpose | Linux | macOS |
|---|---|---|
| Config (`OsdlConfig` YAML) | `$XDG_CONFIG_HOME/osdl/` | `~/Library/Application Support/com.scienceol.osdl/` |
| State (SQLite db, logs) | `$XDG_STATE_HOME/osdl/` | same Application Support dir |
| Cache (mediamtx) | `$XDG_CACHE_HOME/osdl/` | `~/Library/Caches/com.scienceol.osdl/` |
| Runtime (sockets, lockfiles) | `$XDG_RUNTIME_DIR/osdl/` | `$TMPDIR/osdl-$UID/` (no Apple analog) |

`Paths::default_socket_path("chinwe")` →
`<runtime_dir>/chinwe.sock`. `Paths::default_db_path("chinwe")` →
`<state_dir>/chinwe.db`. Per-instance, so they don't collide.

## CLI shape (osdl-cli)

```
osdl serve [--instance NAME] [--config PATH] [--listen ADDR]
           [--socket PATH|disabled] [--data-dir PATH] [--log-file PATH]
           [--detach] [--registry PATH] [--espnow-port PORT]
osdl status
osdl device list [--adapter X] [--type Y] [--role Z] [--json]
osdl device get DEVICE_ID [--json]
osdl device wait id:|type:|role:VALUE [--timeout 30s]
osdl send DEVICE_ID ACTION [-p k=v ...] [--params-file FILE]
osdl events [--kinds k1,k2,...] [--json]
osdl stop
```

All commands honor `--endpoint URI` / `OSDL_ENDPOINT` and
`--instance NAME` / `OSDL_INSTANCE` as global flags.

### Foreground vs. detached

Default is foreground (the convention modern services follow — fits
launchd, systemd, docker, container supervisors). `--detach` uses the
`daemonize` crate to fork a Unix daemon: double-fork, setsid, stdio
redirected to `<state_dir>/<instance>.log`, working directory `/`.

Daemonization has to happen *before* the tokio runtime starts —
post-fork tokio is UB because the reactor's epoll/kqueue fds are
inherited but stale. So `osdl serve` routes through a synchronous
`main_entrypoint` that does the fork and only then builds the runtime.
The other CLI subcommands keep their `#[tokio::main]`-equivalent
behavior.

Relative paths in `--config`, `--registry`, `--data-dir`, `--log-file`,
`--socket` are canonicalized to absolute paths in the parent before the
fork — the daemon's chdir(`/`) would otherwise break them.

The lockfile records the post-fork PID, so `osdl stop` works the same
in either mode.

### Connecting to UDS

tonic doesn't have first-class UDS-client support, so `client.rs` builds
a custom `tower::service_fn` connector that opens a `tokio::net::UnixStream`
and wraps it in `hyper_util::rt::TokioIo`. The endpoint URI is a
placeholder (`http://[::]`) since the connector ignores authority.

## Deployment modes (concrete)

### Mode 1 — local

```
[ user ]
  │
  ▼  CLI client over UDS
[ osdl serve --detach --instance lab ]
  │ in-process
  ▼
[ EngineHandle → OsdlEngine.run() → transports → devices ]
```

### Mode 2 — remote

```
[ workstation ]                              [ lab Pi ]
osdl --endpoint http://lab.local:50051 …  ───→  osdl serve --listen 0.0.0.0:50051
                                                  │
                                                  ▼
                                              EngineHandle → OsdlEngine
```

### Mode 3 — bundled (desktop)

```
[ Tauri app ]
  ├── agent (LLM)
  ├── EngineHandle (direct)         ←── no IPC for the agent
  └── osdl_server::serve()  on UDS  ←── for the in-app CLI / external tools
       │
       ▼
   OsdlEngine.run()
```

The agent gets the same `EngineHandle` API the gRPC service uses, so
agent code looks identical to library code. The UDS surface is there
for everything else (debugging, scripting, packaged tools).

## Observability and error handling

- **Event store (SQLite)** — every `OsdlEvent`, every `DeviceCommand`,
  every TX/RX byte stream gets logged. Queryable by event type, device
  id, time range. Used both for forensic replay and as a safety log.
- **Status watch** — `OsdlStatus::Connected { broker, node_count,
  device_count }` updates whenever those counts change. Cheap to read.
  The CLI's `status` and the desktop overlay both consume this.
- **Lagged events** — slow gRPC subscribers don't block the engine; they
  receive a synthetic `LaggedEvent { dropped: n }` and can refresh
  whatever cache they were maintaining.

## Authentication (TCP)

UDS clients aren't authenticated at the protocol level — filesystem perms
(mode 0600 on the socket, mode 0700 on the runtime dir) gate access to
the local user.

TCP gets a bearer-token interceptor:

- `osdl serve --auth-token <T>` (or `OSDL_AUTH_TOKEN=<T>`) enables it.
  Every TCP RPC must carry `authorization: Bearer <T>` in metadata or
  the server returns `Unauthenticated`. UDS skips the check.
- A non-loopback bind (`0.0.0.0:…`, `192.168.x.y:…`, etc.) **without**
  `--auth-token` is refused at startup with a clear error message —
  hardware control on a routable address without auth would be reckless.
- Loopback without a token is allowed (the existing single-host
  workflow keeps working).
- The CLI client mirrors the contract: `--auth-token <T>` / `OSDL_AUTH_TOKEN`
  attaches the bearer header to every outbound RPC.

Comparison is constant-time so timing can't be used to leak the token
byte-by-byte. mTLS is the longer-term answer for hostile networks; the
shared bearer token is the lab-network threat-model minimum.

## Known gaps and open issues

See [`known-issues.md`](known-issues.md) for the running list:

- `SendCommand` returns `PENDING` always (no per-command response
  correlation yet).
- `Shutdown.graceful` is reserved, not implemented (depends on
  per-command lifecycle).
- `SendCommand` bypasses the loop's injection channel and calls
  `EngineHandle::send_command` directly. Both paths share locked
  tables so it's safe today, but the API surface is split.
- Raw-transport probes (`espnow_probe`, `chinwe_scan`, etc.) don't
  have CLI equivalents yet.
- Drain-valve position tracking + boundary enforcement.

## Quick reference

- Server entrypoint: `crates/osdl-server/src/lib.rs:serve`
- Service impl: `crates/osdl-server/src/service.rs:OsdlService`
- Engine handle / loop split: `crates/osdl-core/src/engine.rs`
- Wire schema: `crates/osdl-proto/proto/osdl.proto`
- Recipes (CLI usage): `docs/recipes/`
- Engine + transport architecture: `docs/architecture.md`
