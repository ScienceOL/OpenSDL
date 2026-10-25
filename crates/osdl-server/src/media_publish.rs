use std::collections::HashSet;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Notify;

use osdl_core::EngineHandle;
use osdl_proto::v1 as pb;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_stream::Stream;
use tonic::Status;

pub type PublishStream = Pin<Box<dyn Stream<Item = Result<pb::PublishMediaStatus, Status>> + Send>>;

#[derive(Clone, Default)]
pub struct Publishers {
    active: Arc<Mutex<HashSet<String>>>,
}
struct Slot {
    source: String,
    active: Arc<Mutex<HashSet<String>>>,
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.source);
    }
}
impl Publishers {
    fn acquire(&self, source: String) -> Result<Slot, Status> {
        if !self
            .active
            .lock()
            .map_err(|_| Status::unavailable("camera publisher state unavailable"))?
            .insert(source.clone())
        {
            return Err(Status::already_exists("camera publisher already active"));
        }
        Ok(Slot {
            source,
            active: self.active.clone(),
        })
    }
    pub async fn start(
        &self,
        engine: &EngineHandle,
        request: pb::PublishMediaRequest,
        shutdown: Arc<Notify>,
    ) -> Result<PublishStream, Status> {
        let target = url::Url::parse(&request.publish_url)
            .map_err(|_| Status::invalid_argument("invalid media target"))?;
        if target.scheme() != "rtmps"
            || target.host_str().is_none()
            || target.username() != ""
            || target.password().is_some()
        {
            return Err(Status::invalid_argument(
                "remote camera publishing requires RTMPS",
            ));
        }
        let executable = engine
            .config()
            .media_gateway
            .ffmpeg_binary
            .as_ref()
            .filter(|path| path.is_absolute() && path.is_file())
            .ok_or_else(|| Status::unavailable("device media runtime is unavailable"))?;
        let source = engine
            .get_media_source(&request.source_id)
            .await
            .ok_or_else(|| Status::not_found("camera source unavailable"))?;
        let h264_path = format!("{}_h264", request.source_id);
        let input = source
            .endpoints
            .iter()
            .filter(|endpoint| {
                endpoint.protocol == osdl_core::media::Protocol::Rtsp
                    && endpoint.location == osdl_core::media::Location::Local
            })
            .max_by_key(|endpoint| endpoint.path == h264_path)
            .ok_or_else(|| Status::unavailable("camera RTSP source unavailable"))?;
        // Argument arrays preserve credentials and paths as single arguments. All
        // upstream credentials stay on this explicitly selected device.
        let mut command = tokio::process::Command::new(executable);
        command.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-progress",
            "pipe:1",
            "-stats_period",
            "0.1",
            "-rtsp_transport",
            "tcp",
            "-i",
            &input.url,
            "-map",
            "0:v:0",
            "-an",
            "-c:v",
            "copy",
            "-f",
            "flv",
            "-tls_verify",
            "1",
            "-verifyhost",
            target.host_str().expect("validated media host"),
        ]);
        if let Some(cafile) = &engine.config().media_gateway.ffmpeg_ca_file {
            command.arg("-ca_file").arg(cafile);
        }
        command.arg(&request.publish_url);
        let slot = self.acquire(request.source_id)?;
        publish_process(command, shutdown, slot).await
    }
}

async fn publish_process(
    mut command: tokio::process::Command,
    shutdown: Arc<Notify>,
    slot: Slot,
) -> Result<PublishStream, Status> {
    let mut stopped = Box::pin(shutdown.notified_owned());
    stopped.as_mut().enable();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Status::unavailable("device media runtime could not start"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Status::unavailable("camera progress unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Status::unavailable("camera diagnostics unavailable"))?;
    let mut diagnostics = Box::pin(log_diagnostics(stderr));
    let mut progress = BufReader::new(stdout).lines();
    // MediaMTX on-demand source startup permits 10 seconds before the first frame.
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let line = tokio::select! {
                _ = &mut stopped => return Err(Status::unavailable("device media runtime stopped")),
                _ = &mut diagnostics => unreachable!(),
                line = progress.next_line() => line,
            }
            .map_err(|_| Status::unavailable("camera publisher failed"))?
            .ok_or_else(|| Status::unavailable("camera publisher failed"))?;
            if media_has_progress(&line) {
                return Ok::<(), Status>(());
            }
        }
    })
    .await
    .map_err(|_| Status::unavailable("camera publish timed out"))??;
    // The authenticated stream owns Child; cancellation kills the local
    // publisher. The configured SRS idle policy also disconnects publishers
    // after their last viewer leaves.
    Ok(Box::pin(async_stream::try_stream! {
        let _slot = slot;
        yield pb::PublishMediaStatus {running:true};
        loop {
            let ended: Result<bool, Status> = tokio::select! {
                _ = &mut stopped => Ok(true),
                _ = &mut diagnostics => unreachable!(),
                status = child.wait() => {
                    if status.is_ok_and(|status| status.success()) { Ok(true) }
                    else { Err(Status::unavailable("camera publisher stopped")) }
                },
                line = progress.next_line() => {
                    if line.is_ok_and(|line| line.is_some()) { Ok(false) }
                    else { Err(Status::unavailable("camera publisher progress ended")) }
                },
            };
            if ended? { break; }
        }
    }))
}

// Stream-copy progress omits frame counts on some FFmpeg builds. Positive
// media time proves packets advanced; bytes alone could be just FLV headers.
fn media_has_progress(line: &str) -> bool {
    let Some((key, value)) = line.split_once('=') else {
        return false;
    };
    matches!(key, "frame" | "out_time_us")
        && value.trim().parse::<i64>().is_ok_and(|value| value > 0)
}

async fn log_diagnostics(mut stderr: tokio::process::ChildStderr) {
    let mut buffer = [0u8; 4096];
    while let Ok(size) = stderr.read(&mut buffer).await {
        if size == 0 {
            break;
        }
        // FFmpeg errors contain the RTSP password and signed target URL.
        // Emit only a closed classification, never raw stderr or arguments.
        let text = String::from_utf8_lossy(&buffer[..size]).to_ascii_lowercase();
        let category = if text.contains("certificate") || text.contains("tls") {
            "TLS verification or handshake"
        } else if text.contains("403") || text.contains("401") {
            "media admission rejected"
        } else if text.contains("connection") || text.contains("timed out") {
            "camera or ingest connection"
        } else if text.contains("codec") || text.contains("invalid data") {
            "camera codec or format"
        } else {
            "device media process"
        };
        log::warn!("camera publisher error: {category}");
    }
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[test]
    fn publishing_accepts_stream_copy_and_encoded_progress() {
        for line in ["frame=1", "out_time_us=1200000"] {
            assert!(media_has_progress(line), "{line}");
        }
        for line in [
            "frame=0",
            "out_time_us=0",
            "out_time_us=-1",
            "out_time_us=N/A",
            "total_size=4096",
            "progress=continue",
        ] {
            assert!(!media_has_progress(line), "{line}");
        }
    }

    #[test]
    fn publisher_fixture() {
        if let Some(marker) = std::env::var_os("LAB_MEDIA_TEST_MARKER") {
            std::fs::write(&marker, b"started").unwrap();
            if let Some(pid_file) = std::env::var_os("LAB_MEDIA_TEST_PID") {
                std::fs::write(pid_file, std::process::id().to_string()).unwrap();
            }
            println!("frame=1");
            use std::io::Write;
            std::io::stdout().flush().unwrap();
            #[cfg(unix)]
            if std::env::var_os("LAB_MEDIA_TEST_CLOSE_STDOUT").is_some() {
                unsafe {
                    libc::close(libc::STDOUT_FILENO);
                }
            }
            let seconds = std::env::var("LAB_MEDIA_TEST_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2);
            std::thread::sleep(std::time::Duration::from_secs(seconds));
            std::fs::write(marker, b"finished").unwrap();
        }
    }

    #[tokio::test]
    async fn dropping_the_media_stream_kills_its_device_process() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("publisher");
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "media_publish::tests::publisher_fixture",
                "--nocapture",
            ])
            .env("LAB_MEDIA_TEST_MARKER", &marker);
        let publishers = Publishers::default();
        let slot = publishers.acquire("camera-one".into()).unwrap();
        let mut stream = publish_process(command, Arc::new(Notify::new()), slot)
            .await
            .unwrap();
        assert!(stream.next().await.unwrap().unwrap().running);
        assert!(publishers.acquire("camera-one".into()).is_err());
        drop(stream);
        assert!(publishers.acquire("camera-one".into()).is_ok());
        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        assert_eq!(
            std::fs::read(marker).unwrap(),
            b"started",
            "cancelled RPC cannot leave its publisher running"
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn closed_progress_cannot_wedge_publisher_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("publisher");
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "media_publish::tests::publisher_fixture",
                "--nocapture",
            ])
            .env("LAB_MEDIA_TEST_MARKER", &marker)
            .env("LAB_MEDIA_TEST_CLOSE_STDOUT", "1");
        let slot = Publishers::default().acquire("camera-one".into()).unwrap();
        let mut stream = publish_process(command, Arc::new(Notify::new()), slot)
            .await
            .unwrap();
        assert!(stream.next().await.unwrap().unwrap().running);
        let ended = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next()).await;
        assert!(
            ended.unwrap().unwrap().is_err(),
            "EOF must end the stream without awaiting a wedged child"
        );
    }
    #[tokio::test]
    async fn authorization_renewal_preserves_publisher_pid_until_expiry() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("publisher");
        let pid_file = directory.path().join("pid");
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "media_publish::tests::publisher_fixture",
                "--nocapture",
            ])
            .env("LAB_MEDIA_TEST_MARKER", &marker)
            .env("LAB_MEDIA_TEST_PID", &pid_file)
            .env("LAB_MEDIA_TEST_SECONDS", "6");
        let publishers = Publishers::default();
        let slot = publishers.acquire("camera".into()).unwrap();
        let registry = crate::account::lease::Leases::default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut identity = crate::account::Identity {
            principal: crate::account::Principal {
                device_id: uuid::Uuid::new_v4(),
                device_generation: 1,
                installation_id: uuid::Uuid::new_v4(),
            },
            scope: crate::account::Scope::Read,
            expires_at: now + 2,
        };
        let lease = registry.acquire(&identity);
        let stream = publish_process(command, Arc::new(Notify::new()), slot)
            .await
            .unwrap();
        let mut stream = crate::account::service::guard_stream(stream, Some(lease));
        assert!(stream.next().await.unwrap().unwrap().running);
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        identity.expires_at = now + 4;
        registry.renew(&identity);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(2100), stream.next())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), pid);
        assert!(publishers.acquire("camera".into()).is_err());
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        drop(stream);
        assert!(publishers.acquire("camera".into()).is_ok());
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        assert_eq!(
            std::fs::read(marker).unwrap(),
            b"started",
            "expired publisher must be killed before its completion marker"
        );
    }
}
