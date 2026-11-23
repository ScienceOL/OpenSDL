use serde::Serialize;

#[derive(Serialize)]
struct Mode<T> {
    code: T,
    name: &'static str,
}

#[derive(Serialize)]
pub(super) struct RobotState {
    robot_mode: Mode<i32>,
    joint_modes: [Mode<i32>; 6],
    safety_status: Mode<i32>,
    program_state: Mode<u32>,
}

impl RobotState {
    pub(super) fn permits_xyz(&self) -> bool {
        self.robot_mode.code == 7
            && matches!(self.safety_status.code, 1 | 2)
            && self.program_state.code == 1
    }

    // Code mappings come from the SDK's RobotMode, JointMode, SafetyMode,
    // and TaskStatus enumerations. Preserve unrecognized controller codes.
    pub(super) fn from_codes(codes: [i32; 9]) -> Self {
        let [robot, j1, j2, j3, j4, j5, j6, safety, program] = codes;
        let program = program as u32;
        Self {
            robot_mode: Mode {
                code: robot,
                name: match robot {
                    -1 => "no_controller",
                    0 => "disconnected",
                    1 => "confirm_safety",
                    2 => "booting",
                    3 => "power_off",
                    4 => "power_on",
                    5 => "idle",
                    6 => "backdrive",
                    7 => "running",
                    8 => "updating_firmware",
                    9 => "waiting_calibration",
                    _ => "unknown",
                },
            },
            joint_modes: [j1, j2, j3, j4, j5, j6].map(|code| Mode {
                code,
                name: match code {
                    235 => "reset",
                    236 => "shutting_down",
                    238 => "backdrive",
                    239 => "power_off",
                    240 => "ready_for_power_off",
                    245 => "not_responding",
                    246 => "motor_initialization",
                    247 => "booting",
                    249 => "bootloader",
                    251 => "violation",
                    252 => "fault",
                    253 => "running",
                    255 => "idle",
                    _ => "unknown",
                },
            }),
            safety_status: Mode {
                code: safety,
                name: match safety {
                    1 => "normal",
                    2 => "reduced",
                    3 => "protective_stop",
                    4 => "recovery",
                    5 => "safeguard_stop",
                    6 => "system_emergency_stop",
                    7 => "robot_emergency_stop",
                    8 => "violation",
                    9 => "fault",
                    10 => "validate_joint_id",
                    11 => "undefined_safety_mode",
                    12 => "automatic_mode_safeguard_stop",
                    13 => "system_three_position_enabling_stop",
                    14 => "tp_three_position_enabling_stop",
                    _ => "unknown",
                },
            },
            program_state: Mode {
                code: program,
                name: match program {
                    1 => "running",
                    2 => "paused",
                    3 => "stopped",
                    _ => "unknown",
                },
            },
        }
    }
}
