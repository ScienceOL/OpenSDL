use super::{v4l2::V4l2Capture, MAX_SNAPSHOT_BYTES};
use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::protocol::{ActionSchema, CommandResult, CommandStatus, DeviceCommand};
use crate::transport::Transport;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

pub(crate) const DEVICE_TYPE: &str = "camera";
pub(crate) const PLATFORM: &str = "camera";

pub(crate) fn transport_id_for(id: &str) -> String {
    format!("camera:{id}")
}

pub(crate) struct Adapter;

impl ProtocolAdapter for Adapter {
    fn platform(&self) -> &str {
        PLATFORM
    }

    fn load_registry(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn match_hardware(&self, _: &str) -> Option<DeviceMatch> {
        None
    }

    fn encode_command(
        &self,
        _device_type: &str,
        command: &DeviceCommand,
    ) -> Result<Vec<u8>, String> {
        if command.action != "snapshot" {
            return Err("camera only supports the snapshot action".into());
        }
        SnapshotCommand::encode(command)
    }

    fn decode_response(&self, _: &str, _: &[u8]) -> Option<HashMap<String, Value>> {
        None
    }
}

pub(crate) struct SnapshotTransport {
    pub camera_id: String,
    pub root: PathBuf,
    pub input: CaptureInput,
}

#[async_trait::async_trait]
impl Transport for SnapshotTransport {
    fn transport_type(&self) -> &str {
        PLATFORM
    }

    fn description(&self) -> String {
        format!("Camera {}", self.camera_id)
    }

    fn is_connected(&self) -> bool {
        true
    }

    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        self.execute(bytes).await.map(|_| ())
    }

    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        let command: SnapshotCommand =
            serde_json::from_slice(bytes).map_err(|_| "invalid snapshot command")?;
        command.check_camera(&self.camera_id)?;
        let jpeg = self.input.jpeg().await?;
        let capture = Capture::save(&self.root, &self.camera_id, jpeg).await?;
        Ok(Some(command.complete(&capture)))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Snapshot,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SnapshotCommand {
    op: Operation,
    command_id: String,
    device_id: String,
}

impl SnapshotCommand {
    pub(crate) fn encode(command: &DeviceCommand) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&Self {
            op: Operation::Snapshot,
            command_id: command.command_id.clone(),
            device_id: command.device_id.clone(),
        })
        .map_err(|_| "cannot encode snapshot command".into())
    }

    pub(crate) fn check_camera(&self, device_id: &str) -> Result<(), String> {
        if self.device_id != device_id || self.command_id.is_empty() {
            return Err("snapshot command does not match camera".into());
        }
        Ok(())
    }

    pub(crate) fn complete(self, capture: &Capture) -> CommandResult {
        CommandResult {
            command_id: self.command_id,
            device_id: self.device_id.clone(),
            status: CommandStatus::Succeeded,
            message: "Camera snapshot captured".into(),
            data: Some(json!({
                "device_id": self.device_id,
                "timestamp": crate::engine::now_millis(),
                "properties": {"snapshot_ok": true, "snapshot_path": capture.path}
            })),
        }
    }
}

pub(crate) fn action() -> ActionSchema {
    ActionSchema {
        name: "snapshot".into(),
        description: "Capture a still JPEG. The command result provides a private snapshot path."
            .into(),
        params: json!({"type": "object"}),
    }
}

#[derive(Serialize)]
pub(crate) struct Capture {
    path: String,
    bytes: usize,
}

impl Capture {
    pub(crate) async fn save(root: &Path, camera_id: &str, jpeg: Vec<u8>) -> Result<Self, String> {
        let directory = root.join(camera_id);
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory)?;
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            file.write_all(&jpeg)?;
            let capture_id = uuid::Uuid::new_v4().as_u128() as u64;
            let path = directory.join(format!("{capture_id}.jpg"));
            file.persist_noclobber(&path)?;
            Ok::<_, std::io::Error>(Self {
                path: path.display().to_string(),
                bytes: jpeg.len(),
            })
        })
        .await
        .map_err(|_| "snapshot storage unavailable")?
        .map_err(|_| "snapshot storage unavailable".into())
    }

    pub(crate) fn event(&self) -> Value {
        json!({"op": "snapshot", "ok": true, "data": self})
    }
}

#[derive(Clone)]
pub(crate) enum CaptureInput {
    Rtsp { executable: PathBuf, url: String },
    V4l2(Arc<V4l2Capture>),
}

impl CaptureInput {
    pub(crate) async fn jpeg(&self) -> Result<Vec<u8>, String> {
        let mut command = match self {
            Self::Rtsp { executable, .. } => Command::new(executable),
            Self::V4l2(capture) => Command::new(capture.executable()),
        };
        command.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
        let frame = match self {
            Self::Rtsp { url, .. } => {
                command.args([
                    "-rtsp_transport",
                    "tcp",
                    "-analyzeduration",
                    "0",
                    "-probesize",
                    "32",
                    "-i",
                    url,
                ]);
                None
            }
            Self::V4l2(capture) => {
                command.args(["-f", "mjpeg", "-i", "pipe:0"]);
                Some(capture.next_frame().await?)
            }
        };
        command.args([
            "-map",
            "0:v:0",
            "-an",
            "-frames:v",
            "1",
            "-c:v",
            "mjpeg",
            "-threads:v",
            "1",
            "-f",
            "image2pipe",
            "pipe:1",
        ]);
        capture_jpeg(
            command,
            frame.as_deref().map(Vec::as_slice),
            Duration::from_secs(12),
        )
        .await
    }
}

async fn capture_jpeg(
    mut command: Command,
    input_frame: Option<&[u8]>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut process = command
        .stdin(if input_frame.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "snapshot media runtime could not start")?;
    let mut input = process.stdin.take();
    let mut output = process
        .stdout
        .take()
        .expect("piped snapshot output")
        .take(MAX_SNAPSHOT_BYTES as u64 + 1);
    let result = tokio::time::timeout(timeout, async {
        let feed = async {
            if let (Some(frame), Some(pipe)) = (input_frame, input.as_mut()) {
                pipe.write_all(frame).await?;
            }
            drop(input);
            Ok::<_, std::io::Error>(())
        };
        let read = async {
            let mut jpeg = Vec::new();
            output.read_to_end(&mut jpeg).await?;
            Ok::<_, std::io::Error>(jpeg)
        };
        let (_, jpeg) = tokio::try_join!(feed, read).map_err(|_| "snapshot media I/O failed")?;
        if jpeg.len() > MAX_SNAPSHOT_BYTES {
            return Err("snapshot exceeds size limit");
        }
        let status = process
            .wait()
            .await
            .map_err(|_| "snapshot process failed")?;
        if !status.success()
            || !jpeg.starts_with(&[0xff, 0xd8, 0xff])
            || !jpeg.ends_with(&[0xff, 0xd9])
        {
            return Err("snapshot media runtime returned no JPEG");
        }
        Ok(jpeg)
    })
    .await;
    match result {
        Ok(Ok(jpeg)) => Ok(jpeg),
        error => {
            let _ = process.kill().await;
            Err(error
                .map_or("snapshot capture timed out", Result::unwrap_err)
                .into())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn fake_ffmpeg(directory: &Path, bytes: &[u8]) -> PathBuf {
        std::fs::write(directory.join("output.jpg"), bytes).unwrap();
        #[cfg(windows)]
        let (filename, script) = ("ffmpeg.cmd", "@echo off\r\ntype \"%~dp0output.jpg\"\r\n");
        #[cfg(unix)]
        let (filename, script) = (
            "ffmpeg",
            "#!/bin/sh\nexec cat \"$(dirname \"$0\")/output.jpg\"\n",
        );
        let executable = directory.join(filename);
        std::fs::write(&executable, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        executable
    }

    #[tokio::test]
    async fn stream_snapshot_rejects_oversized_and_truncated_images() {
        let dir = tempfile::tempdir().unwrap();
        for (bytes, message) in [
            (
                vec![0; MAX_SNAPSHOT_BYTES + 1],
                "snapshot exceeds size limit",
            ),
            (
                vec![0xff, 0xd8, 0xff],
                "snapshot media runtime returned no JPEG",
            ),
        ] {
            let command = Command::new(fake_ffmpeg(dir.path(), &bytes));
            assert_eq!(
                capture_jpeg(command, None, Duration::from_secs(5))
                    .await
                    .unwrap_err(),
                message
            );
        }
    }

    #[tokio::test]
    async fn snapshot_decoder_has_a_process_deadline() {
        #[cfg(windows)]
        let (executable, args) = (
            "powershell.exe",
            &["-NoProfile", "-Command", "Start-Sleep -Seconds 20"][..],
        );
        #[cfg(unix)]
        let (executable, args) = ("sleep", &["20"][..]);
        let mut command = Command::new(executable);
        command.args(args);
        assert_eq!(
            capture_jpeg(command, None, Duration::from_millis(100))
                .await
                .unwrap_err(),
            "snapshot capture timed out",
        );
    }
}
