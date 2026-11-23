# Elite ES66 telemetry

Add the controller to the configuration loaded by the OpenSDL worker on the
instrument network. Use the address assigned to your ES66.

```yaml
elite_robots:
  - id: elite-es66
    host: 192.168.42.10
```

The worker identifies the model through the Elite Dashboard on TCP port 29999
and reads an output-only RTSI v1 recipe on TCP port 30004. It registers the
device as `elite.es66` and samples inventory every 30 seconds. An identity
mismatch or failed sample marks the device offline and clears its cached state.

```sh
lab device get elite-es66
lab send elite-es66 status --json
```

`status` accepts an empty object and returns a fresh controller sample. The
command result includes these fields.

| Field | Meaning |
| --- | --- |
| `identity` | Dashboard-verified model and RTSI controller version |
| `state` | Robot mode, safety status, program state, and joint modes. Each includes the vendor `code` and its `name`. An unrecognized code retains its number and gets the name `unknown`. |
| `controller_timestamp_s` | Seconds since the controller started |
| `tcp_pose` | TCP pose in `robot_base`. `position_m` is `[x, y, z]` in metres. `rotation_vector_rad` is `[rx, ry, rz]` in radians. |
| `joint_positions_rad` | Actual joint angles in radians, in J1–J6 order |

## XYZ motion

To expose `move_xyz`, add `motion_task: opensdl-xyz.task` to the robot's worker
configuration. At the teach pendant, create a dedicated task named
`opensdl-xyz.task` with one Script node. Insert the contents of
[`opensdl-xyz.script`](configs/opensdl-xyz.script), save the task, and start it
from the controller. The Script node enters the interpreter and waits for a
command.

Use this task only with the OpenSDL worker. The worker checks the loaded task
name and operating state before sending a move to the controller's existing
TCP port 30020. It does not start the robot, change safety modes, or load tasks.
The worker advertises the action through the existing cluster connection.

`move_xyz` takes absolute `x_m`, `y_m`, and `z_m` in the same base frame as
`status`. It keeps the current TCP orientation and commands a straight-line
move at up to 0.05 m/s with zero blend radius. Read `status` before choosing
your target. Long moves may require increasing the caller's command timeout.

The controller assigns an ID when it queues the move. OpenSDL verifies that a
subsequent instruction has started and that a newer RTSI sample is within
1 mm of the requested XYZ. A timed-out or disconnected move has an unconfirmed
outcome. Check the controller and actual position before issuing another move.

The reference is the bundled Elite CS SDK 1.3.0 under
`cpp/Elite_Robots_CS_SDK` in ScienceOL/Robots at commit
`0797f078dc1ad4909e4a14374a51ae2007666d22`. `DashboardClient.cpp`,
`RtsiClient.cpp`, `RtsiRecipeInternal.cpp`, `RtsiIOInterface.cpp`, and
`DataType.hpp` define the read path. The vendor's [RTSI instructions][rtsi]
confirm the field types and [pose reference][pose] specifies the base frame.
The [Elite interpreter guide][interpreter] defines the acknowledgements and
execution-state queries. The SDK trajectory implementation in
`example/example/external_control.script` demonstrates Cartesian `movel`.

[rtsi]: https://www.elibot.com/service/articles/list/106
[pose]: https://www.elibot.com/service/articles/list/83
[interpreter]: https://www.elibot.com/service/articles/list/113
