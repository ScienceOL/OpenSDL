use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::Notify;

use osdl_core::EngineHandle;
use osdl_proto::v1 as pb;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_stream::Stream;
use tonic::Status;

pub type PublishStream = Pin<Box<dyn Stream<Item = Result<pb::PublishMediaStatus, Status>> + Send>>;

pub async fn start(
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
        .list_media_sources()
        .await
        .into_iter()
        .find(|source| source.id == request.source_id)
        .ok_or_else(|| Status::not_found("camera source unavailable"))?;
    let input = source
        .endpoints
        .iter()
        .find(|endpoint| {
            endpoint.protocol == osdl_core::media::Protocol::Rtsp
                && endpoint.location == osdl_core::media::Location::Local
        })
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
    publish_process(command, shutdown).await
}

async fn publish_process(
    mut command: tokio::process::Command,
    shutdown: Arc<Notify>,
) -> Result<PublishStream, Status> {
    let mut stopped = Box::pin(shutdown.notified_owned());
    stopped.as_mut().enable();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Status::unavailable("device media runtime could not start"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Status::unavailable("camera progress unavailable"))?;
    let mut progress = BufReader::new(stdout).lines();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let line = tokio::select! {
                _ = &mut stopped => return Err(Status::unavailable("device media runtime stopped")),
                line = progress.next_line() => line,
            }
            .map_err(|_| Status::unavailable("camera publisher failed"))?
            .ok_or_else(|| Status::unavailable("camera publisher failed"))?;
            if line
                .strip_prefix("frame=")
                .is_some_and(|value| value.trim().parse::<u64>().is_ok_and(|frames| frames > 0))
            {
                return Ok::<(), Status>(());
            }
        }
    })
    .await
    .map_err(|_| Status::unavailable("camera publish timed out"))??;
    // The authenticated stream owns Child; cancellation kills the local
    // publisher. SRS also disconnects publishers after its last viewer leaves.
    Ok(Box::pin(async_stream::try_stream! {
        yield pb::PublishMediaStatus {running:true};
        loop {
            let status=tokio::select! {
                _ = &mut stopped => break,
                status=child.wait()=>Some(status),
                line=progress.next_line()=>{
                    if line.is_ok_and(|line|line.is_some()){None}else{Some(child.wait().await)}
                },
            };
            if let Some(status)=status {
                if !status.is_ok_and(|status|status.success()){Err(Status::unavailable("camera publisher stopped"))?;}
                break;
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[test]
    fn publisher_fixture() {
        if let Some(marker) = std::env::var_os("LAB_MEDIA_TEST_MARKER") {
            std::fs::write(&marker, b"started").unwrap();
            println!("frame=1");
            use std::io::Write;
            std::io::stdout().flush().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
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
        let mut stream = publish_process(command, Arc::new(Notify::new()))
            .await
            .unwrap();
        assert!(stream.next().await.unwrap().unwrap().running);
        drop(stream);
        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        assert_eq!(
            std::fs::read(marker).unwrap(),
            b"started",
            "cancelled RPC cannot leave its publisher running"
        );
    }
}
