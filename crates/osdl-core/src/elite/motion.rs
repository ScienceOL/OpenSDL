//! Elite CS interpreter protocol for a dedicated, operator-started task.

use std::{net::Ipv4Addr, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time,
};

use super::wire::{self, Snapshot};
use crate::protocol::ActionSchema;

pub(super) const ACTION: &str = "move_xyz";
const INTERPRETER_PORT: u16 = 30020;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MOVE_TIMEOUT: Duration = Duration::from_secs(75);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const POSITION_TOLERANCE_M: f64 = 0.001;
const SPEED_M_S: f64 = 0.05;
const ACCEL_M_S2: f64 = 0.1;
const MAX_LINE: u64 = 512;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    x_m: f64,
    y_m: f64,
    z_m: f64,
}

#[derive(Serialize)]
pub(super) struct Outcome {
    target_position_m: [f64; 3],
    controller_move_id: u64,
    observed: Snapshot,
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

pub(super) fn schema(task: &str) -> ActionSchema {
    ActionSchema {
        name: ACTION.into(),
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

pub(super) async fn execute(host: Ipv4Addr, task: &str, target: Target) -> Result<Outcome, String> {
    let xyz = [target.x_m, target.y_m, target.z_m];
    if !xyz.iter().all(|value| value.is_finite()) {
        return Err("Elite ES66 XYZ coordinates must be finite".into());
    }

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
    if !before.state.permits_xyz() {
        return Err("Elite robot or safety state does not permit XYZ motion".into());
    }
    check_task(host, task).await?;

    let [x, y, z] = xyz;
    let [rx, ry, rz] = before.tcp_pose.rotation_vector_rad;
    let command = format!(
        "movel([{x:.6},{y:.6},{z:.6},{rx:.6},{ry:.6},{rz:.6}],a={ACCEL_M_S2},v={SPEED_M_S},t=0,r=0)"
    );
    if command.len() as u64 >= MAX_LINE {
        return Err("Elite ES66 XYZ coordinates exceed the controller request size".into());
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
    let [dx, dy, dz] = std::array::from_fn(|index| after.tcp_pose.position_m[index] - xyz[index]);
    if after.controller_timestamp_s <= before.controller_timestamp_s
        || !after.state.permits_xyz()
        || dx.hypot(dy).hypot(dz) > POSITION_TOLERANCE_M
    {
        return Err(uncertain(
            "fresh controller pose did not confirm the target",
        ));
    }
    Ok(Outcome {
        target_position_m: xyz,
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
