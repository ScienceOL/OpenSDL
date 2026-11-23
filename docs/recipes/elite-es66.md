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

The same observation includes scalar workflow properties `tcp_x_m`,
`tcp_y_m`, `tcp_z_m`, `tcp_rx_rad`, `tcp_ry_rad`, `tcp_rz_rad`, and
`joint_1_rad` through `joint_6_rad`. The properties `tcp_frame` and
`tcp_orientation_representation` report `robot_base` and `rotation_vector`.

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

## Relative joint jog

Enable `joint_jog` only after the operator has verified that the configured
task and controller firmware support Elite's six-angle `movej` instruction
in interpreter mode. This is a capability declaration in the worker
configuration. With the same dedicated task as XYZ motion, it looks like this.

```yaml
elite_robots:
  - id: elite-es66
    host: 192.168.42.10
    motion_task: opensdl-xyz.task
    joint_jog: true
```

`jog_joint` takes `joint` (`J1` through `J6`) and signed `delta_rad` with
a magnitude from 0.005 to 0.10 radians (about 0.29° to 5.73°). It adds the
offset to the selected joint's fresh measured angle and holds the other
five angles as joint targets. The controller executes an unblended joint
move at up to 0.10 rad/s under its configured safeguards. A joint jog can
move the TCP in the robot base frame.

Both motion actions use the same exclusive worker motion slot. The worker
checks the configured task, running interpreter, and controller safety
state. A queue acknowledgement supplies `controller_move_id`. Success
requires a subsequent instruction to start and a newer RTSI sample to
confirm every commanded joint angle within 0.002 rad. The result includes
`target_joint_positions_rad` and the fresh `observed` status snapshot. If
the outcome is unconfirmed, check the controller and run `status` before
issuing another move.

The reference is the bundled Elite CS SDK 1.3.0 under
`cpp/Elite_Robots_CS_SDK` in ScienceOL/Robots at commit
`0797f078dc1ad4909e4a14374a51ae2007666d22`. `DashboardClient.cpp`,
`RtsiClient.cpp`, `RtsiRecipeInternal.cpp`, `RtsiIOInterface.cpp`, and
`DataType.hpp` define the read path. The vendor's [RTSI instructions][rtsi]
confirm the field types and [pose reference][pose] specifies the base frame.
The [Elite interpreter guide][interpreter] defines the acknowledgements and
execution-state queries. The SDK trajectory implementation in
`example/example/external_control.script` executes both six-angle `movej`
and Cartesian `movel`. The vendor's [script command example][script]
shows the joint motion parameters.

[rtsi]: https://www.elibot.com/service/articles/list/106
[pose]: https://www.elibot.com/service/articles/list/83
[interpreter]: https://www.elibot.com/service/articles/list/113
[script]: https://www.elibot.com/service/articles/list/104
