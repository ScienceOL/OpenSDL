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

Motion requires a dedicated controller task with exclusive ownership of the
arm and a verifiable terminal outcome for each request. The ScienceOL ES66
reported RTSI firmware `2.14.5.1236` with the task stopped and no task file
loaded. The teleoperation program runs a continuous reverse-control session.
The SDK trajectory script also reports success after a timed-out point read.
The current driver exposes `status` as its sole action.

The reference is the bundled Elite CS SDK 1.3.0 under
`cpp/Elite_Robots_CS_SDK` in ScienceOL/Robots at commit
`0797f078dc1ad4909e4a14374a51ae2007666d22`. `DashboardClient.cpp`,
`RtsiClient.cpp`, `RtsiRecipeInternal.cpp`, `RtsiIOInterface.cpp`, and
`DataType.hpp` define the read path. The vendor's [RTSI instructions][rtsi]
confirm the field types and [pose reference][pose] specifies the base frame.

[rtsi]: https://www.elibot.com/service/articles/list/106
[pose]: https://www.elibot.com/service/articles/list/83
