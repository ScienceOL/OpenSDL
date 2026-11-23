//! Elite CS interpreter protocol for a dedicated, operator-started task.

use std::{net::Ipv4Addr, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time,
};

use super::wire::{self, Snapshot};
use crate::protocol::ActionSchema;

const MOVE_XYZ: &str = "move_xyz";
const JOG_JOINT: &str = "jog_joint";
const INTERPRETER_PORT: u16 = 30020;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MOVE_TIMEOUT: Duration = Duration::from_secs(75);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const POSITION_TOLERANCE_M: f64 = 0.001;
const XYZ_SPEED_M_S: f64 = 0.05;
const XYZ_ACCEL_M_S2: f64 = 0.1;
const MAX_LINE: u64 = 512;
const MIN_JOG_RAD: f64 = 0.005;
const MAX_JOG_RAD: f64 = 0.1;
const JOINT_TOLERANCE_RAD: f64 = 0.002;
const JOINT_SPEED_RAD_S: f64 = 0.1;
const JOINT_ACCEL_RAD_S2: f64 = 0.2;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Command {
    MoveXyz(Xyz),
    JogJoint(Jog),
}

#[derive(Clone, Copy, Deserialize, Serialize)]
enum Joint {
    J1,
    J2,
    J3,
    J4,
    J5,
    J6,
}

impl Joint {
    const ALL: [Self; 6] = [Self::J1, Self::J2, Self::J3, Self::J4, Self::J5, Self::J6];
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Jog {
    joint: Joint,
    delta_rad: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Xyz {
    x_m: f64,
    y_m: f64,
    z_m: f64,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Target {
    Xyz {
        target_position_m: [f64; 3],
    },
    Joint {
        target_joint_positions_rad: [f64; 6],
    },
}

#[derive(Serialize)]
pub(super) struct Outcome {
    #[serde(flatten)]
    target: Target,
    controller_move_id: u64,
    pub(super) observed: Snapshot,
}

pub(super) fn valid_task_name(name: &str) -> bool {
    name.strip_suffix(".task").is_some_and(|stem| {
        !stem.is_empty()
            && stem.len() <= 96
            && stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    })
}

pub(super) fn xyz_schema(task: &str) -> ActionSchema {
    ActionSchema {
        name: MOVE_XYZ.into(),
        description: format!(
            "Move the TCP in a straight line to absolute robot_base XYZ in metres, keeping its orientation. Requires the dedicated {task} task to be started at the controller."
        ),
        params: json!({
            "type": "object",
            "properties": {
                "x_m": {"type":"number","description":"Absolute base-frame X in metres"},
                "y_m": {"type":"number","description":"Absolute base-frame Y in metres"},
                "z_m": {"type":"number","description":"Absolute base-frame Z in metres"}
            },
            "required": ["x_m","y_m","z_m"],
            "additionalProperties": false
        }),
    }
}

pub(super) fn jog_schema(task: &str) -> ActionSchema {
    ActionSchema {
        name: JOG_JOINT.into(),
        description: format!(
            "Move one joint relative to its measured angle and hold the other joint targets. Requires the dedicated {task} task to be started at the controller."
        ),
        params: json!({
            "type": "object",
            "properties": {
                "joint": {"type": "string", "enum": Joint::ALL},
                "delta_rad": {
                    "type": "number",
                    "description": format!("Signed change in radians, with magnitude from {MIN_JOG_RAD} to {MAX_JOG_RAD} inclusive"),
                    "minimum": -MAX_JOG_RAD,
                    "maximum": MAX_JOG_RAD,
                    "anyOf": [{"maximum": -MIN_JOG_RAD}, {"minimum": MIN_JOG_RAD}]
                }
            },
            "required": ["joint", "delta_rad"],
            "additionalProperties": false
        }),
    }
}

impl Command {
    pub(super) fn parse(action: &str, params: Value) -> Result<Self, String> {
        match action {
            MOVE_XYZ => serde_json::from_value(params)
                .map(Self::MoveXyz)
                .map_err(|_| "Elite ES66 move_xyz requires numeric x_m, y_m and z_m".into()),
            JOG_JOINT => serde_json::from_value(params)
                .map(Self::JogJoint)
                .map_err(|_| {
                    "Elite ES66 jog_joint requires a joint from J1 to J6 and numeric delta_rad"
                        .into()
                }),
            _ => Err("unsupported Elite ES66 action".into()),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::MoveXyz(Xyz { x_m, y_m, z_m })
                if ![x_m, y_m, z_m].iter().all(|value| value.is_finite()) =>
            {
                Err("Elite ES66 XYZ coordinates must be finite".into())
            }
            Self::JogJoint(Jog { delta_rad, .. })
                if !(MIN_JOG_RAD..=MAX_JOG_RAD).contains(&delta_rad.abs()) =>
            {
                Err(format!(
                    "Elite ES66 jog_joint requires a delta_rad magnitude from {MIN_JOG_RAD} to {MAX_JOG_RAD}"
                ))
            }
            _ => Ok(()),
        }
    }

    fn plan(self, before: &Snapshot) -> (Target, String) {
        match self {
            Self::MoveXyz(Xyz { x_m, y_m, z_m }) => {
                let [rx, ry, rz] = before.tcp_pose.rotation_vector_rad;
                (
                    Target::Xyz { target_position_m: [x_m, y_m, z_m] },
                    format!("movel([{x_m:.6},{y_m:.6},{z_m:.6},{rx:.6},{ry:.6},{rz:.6}],a={XYZ_ACCEL_M_S2},v={XYZ_SPEED_M_S},t=0,r=0)"),
                )
            }
            Self::JogJoint(Jog { joint, delta_rad }) => {
                let mut angles = before.joint_positions_rad;
                angles[joint as usize] += delta_rad;
                let [j1, j2, j3, j4, j5, j6] = angles;
                (
                    Target::Joint { target_joint_positions_rad: angles },
                    format!("movej([{j1:.6},{j2:.6},{j3:.6},{j4:.6},{j5:.6},{j6:.6}],a={JOINT_ACCEL_RAD_S2},v={JOINT_SPEED_RAD_S},t=0,r=0)"),
                )
            }
        }
    }
}

impl Target {
    fn reached(&self, after: &Snapshot) -> bool {
        match self {
            Self::Xyz { target_position_m } => {
                let [dx, dy, dz] =
                    std::array::from_fn(|i| after.tcp_pose.position_m[i] - target_position_m[i]);
                dx.hypot(dy).hypot(dz) <= POSITION_TOLERANCE_M
            }
            Self::Joint {
                target_joint_positions_rad,
            } => after
                .joint_positions_rad
                .iter()
                .zip(target_joint_positions_rad)
                .all(|(actual, target)| (actual - target).abs() <= JOINT_TOLERANCE_RAD),
        }
    }
}

pub(super) async fn execute(
    host: Ipv4Addr,
    task: &str,
    move_command: Command,
) -> Result<Outcome, String> {
    move_command.validate()?;
    check_task(host, task).await?;
    let mut connection = Interpreter::connect(host).await?;
    if connection.exchange("state").await? != "state: 0: running: state" {
        return Err(format!(
            "Start interpreter mode in {task} at the Elite controller"
        ));
    }

    // Take the initial pose only after preceding interpreter instructions have drained.
    time::timeout(IO_TIMEOUT, connection.fence())
        .await
        .map_err(|_| "Elite interpreter is still busy")??;
    let before = read_pose(host).await?;
    if !before.state.permits_motion() {
        return Err("Elite robot or safety state does not permit motion".into());
    }
    check_task(host, task).await?;

    let (target, command) = move_command.plan(&before);
    if command.len() as u64 >= MAX_LINE {
        return Err("Elite ES66 motion coordinates exceed the controller request size".into());
    }
    let move_id = connection
        .queue(&command)
        .await
        .map_err(|error| uncertain(&error))?;

    // statelastexecuted may refer to an instruction that has only just started.
    // A later fence can only start after the blocking, unblended move has completed.
    time::timeout(MOVE_TIMEOUT, connection.fence())
        .await
        .map_err(|_| uncertain("controller completion timed out"))?
        .map_err(|error| uncertain(&error))?;
    let after = read_pose(host).await.map_err(|error| uncertain(&error))?;
    if after.controller_timestamp_s <= before.controller_timestamp_s
        || !after.state.permits_motion()
        || !target.reached(&after)
    {
        return Err(uncertain(
            "fresh controller pose did not confirm the target",
        ));
    }
    Ok(Outcome {
        target,
        controller_move_id: move_id,
        observed: after,
    })
}

fn uncertain(detail: &str) -> String {
    format!(
        "Elite ES66 motion outcome unconfirmed: {detail}. Check the controller before retrying."
    )
}

async fn check_task(host: Ipv4Addr, task: &str) -> Result<(), String> {
    let loaded = time::timeout(IO_TIMEOUT, wire::loaded_task(host))
        .await
        .map_err(|_| "Elite Dashboard task check timed out")?
        .map_err(|error| error.to_string())?;
    let filename = loaded
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches(['\'', '"']);
    if filename != task {
        return Err(format!(
            "Load and start the dedicated {task} task at the Elite controller"
        ));
    }
    Ok(())
}

async fn read_pose(host: Ipv4Addr) -> Result<Snapshot, String> {
    time::timeout(IO_TIMEOUT, wire::sample(host))
        .await
        .map_err(|_| "Elite ES66 pose sample timed out")?
        .map_err(|error| error.to_string())
}

struct Interpreter(BufReader<TcpStream>);

impl Interpreter {
    async fn connect(host: Ipv4Addr) -> Result<Self, String> {
        let socket = time::timeout(IO_TIMEOUT, TcpStream::connect((host, INTERPRETER_PORT)))
            .await
            .map_err(|_| "Elite interpreter connection timed out")?
            .map_err(|error| format!("Elite interpreter connection failed: {error}"))?;
        socket
            .set_nodelay(true)
            .map_err(|error| error.to_string())?;
        Ok(Self(BufReader::new(socket)))
    }

    async fn exchange(&mut self, command: &str) -> Result<String, String> {
        let mut reply = String::new();
        time::timeout(IO_TIMEOUT, async {
            self.0.write_all(format!("{command}\n").as_bytes()).await?;
            (&mut self.0).take(MAX_LINE).read_line(&mut reply).await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| "Elite interpreter response timed out")?
        .map_err(|error| format!("Elite interpreter connection failed: {error}"))?;
        if !reply.ends_with('\n') {
            return Err("incomplete Elite interpreter response".into());
        }
        Ok(reply.trim_end_matches(['\r', '\n']).into())
    }

    async fn queue(&mut self, command: &str) -> Result<u64, String> {
        let reply = self.exchange(command).await?;
        if let Some((id, statement)) = reply
            .strip_prefix("ack: ")
            .and_then(|ack| ack.split_once(": "))
        {
            if let Ok(id) = id.parse::<u64>() {
                if id > 0 && statement == command {
                    return Ok(id);
                }
            }
        }
        Err("Elite interpreter did not acknowledge the submitted instruction".into())
    }

    async fn fence(&mut self) -> Result<(), String> {
        let id = self.queue("sync()").await?;
        loop {
            let reply = self.exchange("statelastexecuted").await?;
            let last = reply
                .strip_prefix("state: ")
                .and_then(|state| state.strip_suffix(": statelastexecuted"))
                .and_then(|state| state.parse::<u64>().ok())
                .ok_or("invalid Elite interpreter execution state")?;
            if last == id {
                return Ok(());
            }
            time::sleep(POLL_INTERVAL).await;
        }
    }
}
