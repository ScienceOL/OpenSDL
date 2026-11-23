use super::V4l2CameraConfig;
use crate::media::MAX_SNAPSHOT_BYTES;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{watch, Mutex, MutexGuard};

const FRAME_DEADLINE: Duration = Duration::from_secs(5);
const HEADER: &[u8] = b"--ffmpeg\r\nContent-type: image/jpeg\r\nContent-length: ";
type FrameSender = watch::Sender<Option<Arc<Vec<u8>>>>;
type Active = Arc<Mutex<Option<FrameSender>>>;

/// One on-demand capture process for the configured camera. Receivers retain
/// only the latest compressed frame, so a slow preview cannot delay snapshots.
pub struct V4l2Capture {
    executable: PathBuf,
    config: V4l2CameraConfig,
    active: Active,
}

impl std::fmt::Debug for V4l2Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V4l2Capture")
            .field("id", &self.config.id)
            .finish_non_exhaustive()
    }
}

impl V4l2Capture {
    pub(crate) fn new(executable: PathBuf, config: V4l2CameraConfig) -> Self {
        Self {
            executable,
            config,
            active: Default::default(),
        }
    }

    pub(in crate::media) fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn frames_per_second(&self) -> u8 {
        self.config.frames_per_second
    }

    fn capture_command(&self) -> Command {
        let config = &self.config;
        let mut command = Command::new(&self.executable);
        command.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "v4l2",
            "-input_format",
            "mjpeg",
            "-video_size",
            &format!("{}x{}", config.width, config.height),
            "-framerate",
            &config.frames_per_second.to_string(),
            "-i",
        ]);
        command.arg(&config.device).args([
            "-map",
            "0:v:0",
            "-an",
            "-c:v",
            "copy",
            "-flush_packets",
            "1",
            "-f",
            "mpjpeg",
            "pipe:1",
        ]);
        command
    }

    async fn subscribe(&self) -> Frames {
        self.subscribe_with(|| self.capture_command()).await
    }

    async fn subscribe_with(&self, command: impl FnOnce() -> Command) -> Frames {
        let mut active = self.active.lock().await;
        if let Some(sender) = active.as_ref() {
            return Frames(sender.subscribe());
        }
        let (sender, receiver) = watch::channel(None);
        *active = Some(sender.clone());
        tokio::spawn(run_capture(self.active.clone(), sender, command()));
        Frames(receiver)
    }

    pub(in crate::media) async fn next_frame(&self) -> Result<Arc<Vec<u8>>, String> {
        tokio::time::timeout(FRAME_DEADLINE, async {
            self.subscribe().await.next().await
        })
        .await
        .map_err(|_| "USB camera frame timed out")?
    }

    pub async fn feed_preview(&self, mut output: ChildStdin) -> Result<(), String> {
        let mut frames = self.subscribe().await;
        loop {
            let frame = frames.next().await?;
            output
                .write_all(&frame)
                .await
                .map_err(|_| "USB camera preview ended")?;
        }
    }
}

struct Frames(watch::Receiver<Option<Arc<Vec<u8>>>>);

impl Frames {
    async fn next(&mut self) -> Result<Arc<Vec<u8>>, String> {
        self.0
            .changed()
            .await
            .map_err(|_| "USB camera capture stopped")?;
        let frame = self
            .0
            .borrow_and_update()
            .clone()
            .expect("captures publish only complete frames");
        Ok(frame)
    }
}

async fn run_capture(active: Active, sender: FrameSender, mut command: Command) {
    let mut process = match command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(process) => process,
        Err(_) => {
            log::warn!("USB camera capture process could not start");
            active.lock().await.take();
            return;
        }
    };
    let mut output = BufReader::new(process.stdout.take().expect("piped USB camera output"));
    let mut state = tokio::select! {
        state = idle_capture(&active, &sender) => state,
        () = forward_frames(&mut output, &sender) => {
            log::warn!("USB camera capture ended before receiving a valid frame");
            active.lock().await
        }
    };
    // Hold the startup lock until the V4L2 descriptor is closed and the
    // process is reaped. A new reader cannot race the exclusive device open.
    let _ = process.kill().await;
    state.take();
}

async fn idle_capture<'a>(
    active: &'a Mutex<Option<FrameSender>>,
    sender: &FrameSender,
) -> MutexGuard<'a, Option<FrameSender>> {
    loop {
        sender.closed().await;
        let state = active.lock().await;
        if sender.is_closed() {
            return state;
        }
    }
}

async fn forward_frames(output: &mut (impl AsyncBufRead + Unpin), sender: &FrameSender) {
    while let Ok(Ok(frame)) = tokio::time::timeout(FRAME_DEADLINE, read_frame(output)).await {
        sender.send_replace(Some(Arc::new(frame)));
    }
}

async fn read_frame(input: &mut (impl AsyncBufRead + Unpin)) -> std::io::Result<Vec<u8>> {
    let invalid = || Error::new(ErrorKind::InvalidData, "invalid multipart JPEG frame");
    let mut header = [0; HEADER.len()];
    input.read_exact(&mut header).await?;
    if header != HEADER {
        return Err(invalid());
    }
    let mut size = Vec::with_capacity(12);
    input.take(12).read_until(b'\n', &mut size).await?;
    let size = size
        .strip_suffix(b"\r\n")
        .and_then(|line| std::str::from_utf8(line).ok())
        .and_then(|line| line.parse::<usize>().ok())
        .filter(|size| (4..=MAX_SNAPSHOT_BYTES).contains(size))
        .ok_or_else(invalid)?;
    let mut separator = [0; 2];
    input.read_exact(&mut separator).await?;
    if separator != *b"\r\n" {
        return Err(invalid());
    }
    let mut frame = vec![0; size];
    input.read_exact(&mut frame).await?;
    input.read_exact(&mut separator).await?;
    if separator != *b"\r\n" || !frame.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(invalid());
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn multipart_reader_checks_length_before_reading_payload() {
        let frame = b"--ffmpeg\r\nContent-type: image/jpeg\r\nContent-length: 4\r\n\r\n\xff\xd8\xff\xd9\r\n";
        let mut valid = &frame[..];
        assert_eq!(
            read_frame(&mut valid).await.unwrap(),
            [0xff, 0xd8, 0xff, 0xd9]
        );
        let oversized = format!(
            "--ffmpeg\r\nContent-type: image/jpeg\r\nContent-length: {}\r\n\r\n",
            MAX_SNAPSHOT_BYTES + 1
        );
        let mut body = oversized.as_bytes();
        assert_eq!(
            read_frame(&mut body).await.unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn preview_and_snapshot_share_the_running_capture_and_release_it() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let frame = dir.path().join("frame");
        std::fs::write(&frame, b"--ffmpeg\r\nContent-type: image/jpeg\r\nContent-length: 4\r\n\r\n\xff\xd8\xff\xd9\r\n").unwrap();
        let config = V4l2CameraConfig {
            id: "color".into(),
            description: None,
            device: "/dev/video0".into(),
            width: 640,
            height: 480,
            frames_per_second: 15,
        };
        let camera = V4l2Capture::new(PathBuf::new(), config);
        let start = || {
            #[cfg(unix)]
            let mut command = {
                let mut command = Command::new("/bin/sh");
                command.args([
                    "-c",
                    r#"printf x >> "$TEST_MARKER"; while :; do cat "$TEST_FRAME"; sleep .05; done"#,
                ]);
                command
            };
            #[cfg(windows)]
            let mut command = {
                let mut command = Command::new("powershell.exe");
                command.args(["-NoProfile", "-NonInteractive", "-Command",
                    r#"[IO.File]::AppendAllText($env:TEST_MARKER, 'x'); $frame = [IO.File]::ReadAllBytes($env:TEST_FRAME); $out = [Console]::OpenStandardOutput(); while ($true) { $out.Write($frame, 0, $frame.Length); $out.Flush(); Start-Sleep -Milliseconds 50 }"#]);
                command
            };
            command
                .env("TEST_MARKER", &marker)
                .env("TEST_FRAME", &frame);
            command
        };
        let mut preview = camera.subscribe_with(start).await;
        let mut snapshot = camera.subscribe_with(start).await;
        assert_eq!(*preview.next().await.unwrap(), [0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(*snapshot.next().await.unwrap(), [0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(std::fs::read(&marker).unwrap(), b"x");
        drop(snapshot);
        assert!(preview.next().await.is_ok());
        drop(preview);
        tokio::time::timeout(Duration::from_secs(3), async {
            while camera.active.lock().await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("unobserved capture should stop and be reaped");
        let mut restarted = camera.subscribe_with(start).await;
        assert!(restarted.next().await.is_ok());
        assert_eq!(std::fs::read(marker).unwrap(), b"xx");
    }
}
