//! mediamtx subprocess manager.
//!
//! The engine writes a config file derived from `MediaSourceConfig`s, spawns
//! `mediamtx <config>` as a child process, and tears it down on shutdown.
//! Health is monitored passively: if the process exits we log and emit no
//! further `MediaSourceOnline` events for that lifetime — caller decides
//! whether to retry.
//!
//! mediamtx itself is not bundled; users install it via package manager
//! (`brew install mediamtx`, `apt install mediamtx`, etc.). The binary path
//! is auto-detected via PATH or overridable in config.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

use super::MediaPath;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaGatewayConfig {
    /// Bind host advertised in `MediaEndpoint.url`. Default `127.0.0.1`. Set
    /// to a LAN-reachable IP / hostname when consumers run on other hosts.
    #[serde(default = "default_advertise_host")]
    pub advertise_host: String,

    /// Path to the mediamtx binary. If `None`, found on PATH.
    #[serde(default)]
    pub binary: Option<PathBuf>,

    /// Listener ports. Defaults match mediamtx itself (8554/8888/8889).
    #[serde(default)]
    pub ports: ListenerPorts,
}

impl Default for MediaGatewayConfig {
    fn default() -> Self {
        Self {
            advertise_host: default_advertise_host(),
            binary: None,
            ports: ListenerPorts::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenerPorts {
    #[serde(default = "default_rtsp_port")]
    pub rtsp: u16,
    #[serde(default = "default_hls_port")]
    pub hls: u16,
    #[serde(default = "default_webrtc_port")]
    pub webrtc: u16,
    #[serde(default = "default_webrtc_udp_port")]
    pub webrtc_udp: u16,
}

impl Default for ListenerPorts {
    fn default() -> Self {
        Self {
            rtsp: default_rtsp_port(),
            hls: default_hls_port(),
            webrtc: default_webrtc_port(),
            webrtc_udp: default_webrtc_udp_port(),
        }
    }
}

fn default_advertise_host() -> String {
    "127.0.0.1".into()
}
fn default_rtsp_port() -> u16 {
    8554
}
fn default_hls_port() -> u16 {
    8888
}
fn default_webrtc_port() -> u16 {
    8889
}
fn default_webrtc_udp_port() -> u16 {
    8189
}

/// Validate a `MediaPath` field that will be string-interpolated into the
/// generated mediamtx YAML. Rejects anything that could break out of the
/// current line (which would let the user inject sibling YAML keys, and
/// since `runOnInit` / `runOnReady` execute commands, reach RCE).
///
/// Path names follow mediamtx's own constraint: `[A-Za-z0-9_-]+`. URLs
/// must parse, must use a known scheme, and must not contain control
/// characters or quotes that could confuse the ffmpeg argv splitter.
fn validate_path(p: &MediaPath) -> Result<(), MediamtxError> {
    if p.name.is_empty()
        || !p
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(MediamtxError::InvalidConfig(format!(
            "path name {:?}: must match [A-Za-z0-9_-]+",
            p.name
        )));
    }
    for (label, value, schemes) in [
        ("source_uri", p.source_uri.as_deref(), &["rtsp", "rtmp"][..]),
        (
            "transcode_from",
            p.transcode_from.as_deref(),
            &["rtsp", "rtmp"][..],
        ),
        ("push_to", p.push_to.as_deref(), &["rtmp", "rtsp"][..]),
    ] {
        if let Some(v) = value {
            validate_url(label, v, schemes)?;
        }
    }
    Ok(())
}

fn validate_url(label: &str, value: &str, schemes: &[&str]) -> Result<(), MediamtxError> {
    // Reject anything that could break a YAML line or an ffmpeg argv token.
    for c in value.chars() {
        if c.is_control() || matches!(c, '"' | '\'' | '\\' | '<' | '>' | '`') {
            return Err(MediamtxError::InvalidConfig(format!(
                "{label} {value:?}: contains forbidden character {c:?}"
            )));
        }
    }
    let scheme_ok = schemes.iter().any(|s| {
        value.len() > s.len() + 3
            && value.as_bytes()[..s.len()].eq_ignore_ascii_case(s.as_bytes())
            && value[s.len()..].starts_with("://")
    });
    if !scheme_ok {
        return Err(MediamtxError::InvalidConfig(format!(
            "{label} {value:?}: must start with one of {schemes:?}://"
        )));
    }
    Ok(())
}

/// ffmpeg flags applied at the *input* of every demuxer we spawn, on both
/// the transcode (`runOnDemand` / `runOnInit`) and the remote-push
/// (`runOnReady`) paths. They strip ffmpeg's default ~5s probe / analyze
/// window and the demuxer reorder buffer so a frame goes downstream as
/// soon as it arrives, instead of getting parked.
///
/// Single source of truth — both renderers below read this constant so
/// future tuning lands in one place. If you split per-path tuning later,
/// duplicate this string before changing it, don't shadow the const.
const FFMPEG_LOW_LATENCY_INPUT: &str =
    "-fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0";

/// libx264 + AAC encode flags shared by every transcode path (both
/// rtsp-republish and direct-RTMP-push variants). Kept separate from
/// FFMPEG_LOW_LATENCY_INPUT because that one applies to copy-only
/// runOnReady jobs too, while these flags only matter when we encode.
///
/// Tuning rationale:
///   - libx264 ultrafast / zerolatency / -bf 0 — no B-frames, no lookahead
///   - -profile:v baseline -pix_fmt yuv420p — broadest decoder support
///   - -g 8 + x264 keyint=8 scenecut=0 — IDR ~every 0.5 s @15fps so a
///     fresh subscriber waits at most ~500 ms for a keyframe
///   - repeat_headers=1 — SPS/PPS prepended to every IDR for mid-stream
///     joins without out-of-band negotiation
///   - rc-lookahead=0 / sync-lookahead=0 — encoder doesn't queue frames
///   - -c:a aac -ar 44100 -b:a 64k — lightweight, browser-friendly audio
///   - -flush_packets 1 — muxer doesn't pool packets before writing
///     (matters for FLV/RTSP intermediates)
const FFMPEG_H264_LOWLATENCY_ENCODE: &str = "-c:v libx264 -preset ultrafast -tune zerolatency \
     -profile:v baseline -pix_fmt yuv420p \
     -bf 0 -g 8 \
     -x264-params keyint=8:scenecut=0:repeat_headers=1:rc-lookahead=0:sync-lookahead=0:bframes=0 \
     -c:a aac -ar 44100 -b:a 64k \
     -flush_packets 1";

/// Render a mediamtx YAML config from gateway settings + path entries.
pub fn render_config(
    cfg: &MediaGatewayConfig,
    paths: &[MediaPath],
) -> Result<String, MediamtxError> {
    for p in paths {
        validate_path(p)?;
    }
    Ok(render_config_unchecked(cfg, paths))
}

/// Whether the rendered mediamtx config must avoid binding UDP 8000.
/// SRS hard-codes `a=candidate:... <host> 8000` into its SDP answer, so
/// SRS must own UDP 8000 on the host whenever a path pushes to SRS *and*
/// we want to read back over WHEP. Generic RTMP relays (Twitch, YouTube)
/// don't re-serve over WebRTC, so they don't trigger this — see the
/// `pushes_to_srs` field on `MediaPath`. mediamtx can still serve local
/// WHEP as long as its own WebRTC UDP mux is pinned elsewhere
/// (`webrtcLocalUDPAddress`, default :8189) and RTSP over UDP is disabled.
fn should_reserve_webrtc_udp_for_srs(paths: &[MediaPath]) -> bool {
    paths.iter().any(|p| p.pushes_to_srs)
}

fn render_config_unchecked(cfg: &MediaGatewayConfig, paths: &[MediaPath]) -> String {
    let mut s = String::new();
    s.push_str("# Generated by OpenSDL — do not edit by hand.\n");
    s.push_str("logLevel: info\n");
    s.push_str("logDestinations: [stdout]\n\n");

    s.push_str("rtsp: yes\n");
    s.push_str(&format!("rtspAddress: :{}\n", cfg.ports.rtsp));
    let reserve_udp_8000 = should_reserve_webrtc_udp_for_srs(paths);
    // Default RTSP transport list. When a path pushes to SRS, reserve UDP
    // 8000 for SRS WebRTC by forcing mediamtx RTSP to TCP-only. mediamtx's
    // own local WebRTC stays enabled on `webrtc_udp` instead.
    if reserve_udp_8000 {
        s.push_str("rtspTransports: [tcp]\n");
        s.push_str("rtpAddress: :0\n");
        s.push_str("rtcpAddress: :0\n");
        s.push_str("multicastRTPPort: 0\n\n");
    } else {
        s.push_str("rtspTransports: [tcp, udp]\n\n");
    }

    s.push_str("hls: yes\n");
    s.push_str(&format!("hlsAddress: :{}\n", cfg.ports.hls));
    s.push_str("hlsAlwaysRemux: no\n\n");

    s.push_str("webrtc: yes\n");
    s.push_str(&format!("webrtcAddress: :{}\n", cfg.ports.webrtc));
    s.push_str(&format!(
        "webrtcLocalUDPAddress: :{}\n",
        cfg.ports.webrtc_udp
    ));
    // Pin the ICE candidate's host to whatever we advertise to consumers.
    // Without this, mediamtx gathers every interface — including macOS's
    // mDNS-anonymized `*.local` candidate that Chromium 110+ emits for
    // privacy. Some embedded WebRTC stacks (Electron's renderer) can't
    // resolve those, so the SDP exchange succeeds but ICE never connects
    // and the tile sits at "handshake ok, no media". Pinning to the
    // advertise host (loopback in the desktop case, the LAN IP for
    // off-host viewers) keeps ICE on a network path the client just used
    // to reach mediamtx — guaranteed reachable.
    s.push_str(&format!(
        "webrtcAdditionalHosts: [{}]\n",
        cfg.advertise_host
    ));
    s.push_str("\n");

    s.push_str("rtmp: no\n");
    s.push_str("srt: no\n\n");

    s.push_str("paths:\n");
    for p in paths {
        s.push_str(&format!("  {}:\n", p.name));
        if let Some(src) = &p.source_uri {
            s.push_str(&format!("    source: {}\n", src));
            if p.rtsp_transport_tcp {
                s.push_str("    rtspTransport: tcp\n");
            }
            // When this path also republishes to a remote, keep it always-on
            // so the upstream pull stays alive between local consumers — the
            // remote ingest is itself a permanent consumer. Without this,
            // mediamtx waits for a local viewer before pulling the camera,
            // and the remote push never fires because the SRS ingest side
            // has nothing to consume yet → deadlock.
            if p.push_to.is_some() {
                s.push_str("    sourceOnDemand: no\n");
            } else {
                s.push_str("    sourceOnDemand: yes\n");
                s.push_str("    sourceOnDemandStartTimeout: 10s\n");
                s.push_str("    sourceOnDemandCloseAfter: 30s\n");
            }
        } else if let Some(upstream) = &p.transcode_from {
            // ffmpeg pulls upstream and republishes as H.264.
            //
            // Two driving modes:
            //   - on-demand: only transcode while a consumer is connected.
            //   - always-on: start immediately and keep running. Required
            //     when `push_to` is set, otherwise nothing forces the
            //     upstream link to stay alive between consumer connects.
            //
            // When `push_to` is set we push to the remote target directly
            // (RTMP/FLV). We do NOT also publish to the local mediamtx
            // path because mediamtx's RTSP listener in our config is
            // TCP-only (UDP is disabled to free port 8000 for SRS's
            // WebRTC egress), and ffmpeg's tee muxer can't be told to
            // publish over RTSP-over-TCP for one sub-output only —
            // publishing over UDP gets rejected with "method SETUP
            // failed: 461 (Unsupported Transport)" and the tee muxer
            // then aborts BOTH outputs, killing the SRS push too.
            // The local `cam131_h264` path stays empty unless a local
            // viewer connects; in that case the source config below
            // (sourceOnDemand-style) re-pulls from the upstream RTSP
            // and serves it via mediamtx — but for SRS delivery we
            // bypass mediamtx entirely.
            let transport = if p.rtsp_transport_tcp {
                "-rtsp_transport tcp"
            } else {
                ""
            };
            let always_on = p.push_to.is_some();
            s.push_str("    source: publisher\n");
            let runner_keyword = if always_on {
                "runOnInit"
            } else {
                "runOnDemand"
            };
            // When `push_to` is set, the encoder writes to RTMP/FLV
            // directly — one ffmpeg process, one encoder, one output.
            // No tee, no runOnReady re-pull → no flap, no UDP-vs-TCP
            // transport mismatch with mediamtx. Otherwise we publish
            // locally and let mediamtx serve consumers.
            let output = match p.push_to.as_deref() {
                Some(target) => format!("-f flv {target}"),
                None => "-f rtsp rtsp://localhost:$RTSP_PORT/$MTX_PATH".to_string(),
            };
            s.push_str(&format!(
                "    {runner_keyword}: >\n      ffmpeg -hide_banner -loglevel warning \
                  {FFMPEG_LOW_LATENCY_INPUT} \
                  {transport} \
                  -i {upstream} \
                  {FFMPEG_H264_LOWLATENCY_ENCODE} \
                  {output}\n",
            ));
            if always_on {
                s.push_str("    runOnInitRestart: yes\n");
            } else {
                s.push_str("    runOnDemandRestart: yes\n");
                s.push_str("    runOnDemandStartTimeout: 10s\n");
                s.push_str("    runOnDemandCloseAfter: 30s\n");
            }
        }
        // Passthrough path (no transcode): encoder is upstream,
        // so we can't bake the push into the transcode ffmpeg. Use runOnReady
        // that `-c copy`s from mediamtx to SRS.
        if p.push_to.is_some() && p.transcode_from.is_none() {
            if let Some(target) = &p.push_to {
                s.push_str(&format!(
                    "    runOnReady: >\n      ffmpeg -hide_banner -loglevel warning \
                     {FFMPEG_LOW_LATENCY_INPUT} \
                     -rtsp_transport tcp \
                     -i rtsp://localhost:$RTSP_PORT/$MTX_PATH \
                     -c copy -f flv {target}\n"
                ));
                s.push_str("    runOnReadyRestart: yes\n");
            }
        }
    }
    s
}

#[derive(thiserror::Error, Debug)]
pub enum MediamtxError {
    #[error("mediamtx binary not found in PATH; install via 'brew install mediamtx' / 'apt install mediamtx', or set media_gateway.binary in config")]
    BinaryMissing,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid media config: {0}")]
    InvalidConfig(String),
    #[error("mediamtx exited before binding RTSP listener (status={0:?})")]
    ChildExitedDuringStartup(Option<i32>),
    #[error("mediamtx did not bind RTSP listener within {0:?}")]
    StartupTimeout(Duration),
}

/// Poll the RTSP port until it accepts connections, until the child exits,
/// or until `timeout` elapses. mediamtx logs `[RTSP] listener opened` to
/// stdout when it's ready, but we'd rather not parse logs when a TCP
/// connect probe is straightforward and language-agnostic.
async fn wait_listening(
    child: &mut Child,
    port: u16,
    timeout: Duration,
) -> Result<(), MediamtxError> {
    let addr = format!("127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(MediamtxError::ChildExitedDuringStartup(status.code()));
        }
        if TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(MediamtxError::StartupTimeout(timeout));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Locate the mediamtx binary. Honours an explicit override, then `which`,
/// then a couple of common install locations.
pub fn locate_binary(override_path: Option<&PathBuf>) -> Result<PathBuf, MediamtxError> {
    if let Some(p) = override_path {
        if p.exists() {
            return Ok(p.clone());
        }
        return Err(MediamtxError::BinaryMissing);
    }
    // Walk $PATH manually rather than pulling in `which` as a dep.
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("mediamtx");
            if cand.exists() {
                return Ok(cand);
            }
        }
    }
    for cand in [
        "/opt/homebrew/bin/mediamtx",
        "/opt/homebrew/opt/mediamtx/bin/mediamtx",
        "/usr/local/bin/mediamtx",
        "/usr/bin/mediamtx",
    ] {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(MediamtxError::BinaryMissing)
}

/// Owns one mediamtx child process and its temp config file.
pub struct MediamtxProcess {
    child: Child,
    _config_path: tempfile::TempPath,
}

impl MediamtxProcess {
    pub async fn spawn(
        cfg: &MediaGatewayConfig,
        paths: &[MediaPath],
    ) -> Result<Self, MediamtxError> {
        let bin = locate_binary(cfg.binary.as_ref())?;
        let yaml = render_config(cfg, paths)?;

        let mut tf = tempfile::NamedTempFile::with_suffix(".yml")?;
        std::io::Write::write_all(&mut tf, yaml.as_bytes())?;
        let cfg_path = tf.into_temp_path();

        log::info!(
            "Starting mediamtx ({}) with config {} ({} paths)",
            bin.display(),
            cfg_path.display(),
            paths.len(),
        );

        // mediamtx logs (and the ffmpeg children it spawns) go to stdout so
        // operators can see push status, transcoding errors, etc. inherit
        // means they go to the engine's terminal.
        let mut child = Command::new(&bin)
            .arg(&*cfg_path)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;

        // Block until mediamtx has bound its RTSP listener (which it does
        // first), or fail. Without this we'd advertise endpoint URLs before
        // the listener is ready, and surface a "bind failed" only when a
        // consumer connects — too late.
        if let Err(e) = wait_listening(
            &mut child,
            cfg.ports.rtsp,
            std::time::Duration::from_secs(5),
        )
        .await
        {
            // Process exited or never bound. Make sure it's gone.
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(e);
        }

        Ok(Self {
            child,
            _config_path: cfg_path,
        })
    }

    /// Graceful shutdown: SIGTERM first so mediamtx can stop its own ffmpeg
    /// children cleanly, then SIGKILL after a grace period if it didn't go.
    /// On always-on remote-push paths the ffmpeg sidecars are long-running
    /// and a hard kill leaves orphans visible in `ps`.
    pub async fn shutdown(mut self) {
        const GRACE: Duration = Duration::from_secs(2);

        let pid = self.child.id();
        if let Some(p) = pid {
            log::info!("Stopping mediamtx pid={} (SIGTERM, grace {GRACE:?})", p);
        }

        #[cfg(unix)]
        if let Some(p) = pid {
            // tokio's Child has no SIGTERM helper; use raw libc.
            // Safety: passing a known pid we own to a signal call.
            unsafe { libc::kill(p as libc::pid_t, libc::SIGTERM) };
        }
        #[cfg(not(unix))]
        let _ = self.child.start_kill();

        match tokio::time::timeout(GRACE, self.child.wait()).await {
            Ok(Ok(status)) => {
                log::info!("mediamtx exited cleanly: {status}");
            }
            Ok(Err(e)) => log::warn!("mediamtx wait failed: {e}"),
            Err(_) => {
                log::warn!("mediamtx did not exit within {GRACE:?}; sending SIGKILL");
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
            }
        }
    }

    /// Has the child exited?
    pub fn try_exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_config_passthrough_only() {
        let cfg = MediaGatewayConfig::default();
        let paths = vec![MediaPath {
            name: "cam1".into(),
            source_uri: Some("rtsp://1.2.3.4/foo".into()),
            rtsp_transport_tcp: true,
            transcode_from: None,
            push_to: None,
            pushes_to_srs: false,
        }];
        let yaml = render_config(&cfg, &paths).unwrap();
        assert!(yaml.contains("rtspAddress: :8554"));
        assert!(yaml.contains("source: rtsp://1.2.3.4/foo"));
        assert!(yaml.contains("rtspTransport: tcp"));
        assert!(yaml.contains("sourceOnDemand: yes"));
        // No transcoding path present
        assert!(!yaml.contains("runOnDemand:"));
    }

    #[test]
    fn render_config_with_transcode() {
        let cfg = MediaGatewayConfig::default();
        let paths = vec![
            MediaPath {
                name: "cam1".into(),
                source_uri: Some("rtsp://1.2.3.4/main".into()),
                rtsp_transport_tcp: true,
                transcode_from: None,
                push_to: None,
                pushes_to_srs: false,
            },
            MediaPath {
                name: "cam1_h264".into(),
                source_uri: None,
                rtsp_transport_tcp: true,
                transcode_from: Some("rtsp://1.2.3.4/sub".into()),
                push_to: None,
                pushes_to_srs: false,
            },
        ];
        let yaml = render_config(&cfg, &paths).unwrap();
        assert!(yaml.contains("cam1:"));
        assert!(yaml.contains("cam1_h264:"));
        assert!(yaml.contains("ffmpeg"));
        assert!(yaml.contains("libx264"));
        assert!(yaml.contains("rtsp://1.2.3.4/sub"));
        assert!(yaml.contains("rtsp://localhost:$RTSP_PORT/$MTX_PATH"));
    }

    #[test]
    fn render_config_with_remote_push() {
        let cfg = MediaGatewayConfig::default();
        let paths = vec![MediaPath {
            name: "cam1_h264".into(),
            source_uri: None,
            rtsp_transport_tcp: true,
            transcode_from: Some("rtsp://1.2.3.4/sub".into()),
            push_to: Some("rtmp://srs.example.com:1935/openSDL/cam1".into()),
            pushes_to_srs: true,
        }];
        let yaml = render_config(&cfg, &paths).unwrap();
        // Push on a transcode path: encoder writes directly to SRS in the
        // SAME ffmpeg process (no tee, no runOnReady hook).
        assert!(
            yaml.contains("rtmp://srs.example.com:1935/openSDL/cam1"),
            "should contain the SRS push URL"
        );
        assert!(
            yaml.contains("-f flv"),
            "should output FLV (RTMP-compatible)"
        );
        assert!(
            !yaml.contains("-f tee"),
            "should NOT use tee muxer (rtsp-over-udp publish rejected by mediamtx TCP-only)"
        );
        assert!(
            yaml.contains("runOnInitRestart: yes"),
            "push_to makes the path always-on"
        );
    }

    #[test]
    fn srs_push_reserves_udp_8000_but_generic_rtmp_does_not() {
        // SRS push: gateway must yield host UDP 8000 → RTSP forced TCP-only,
        // RTP/RTCP UDP zeroed, mediamtx WebRTC pinned to webrtc_udp instead.
        let cfg = MediaGatewayConfig::default();
        let srs_paths = vec![MediaPath {
            name: "cam".into(),
            source_uri: Some("rtsp://1.2.3.4/main".into()),
            rtsp_transport_tcp: true,
            transcode_from: None,
            push_to: Some("rtmp://127.0.0.1:1935/live/cam".into()),
            pushes_to_srs: true,
        }];
        let yaml = render_config(&cfg, &srs_paths).unwrap();
        assert!(yaml.contains("rtspTransports: [tcp]"));
        assert!(yaml.contains("rtpAddress: :0"));

        // Generic RTMP relay (Twitch, YouTube, etc.) — no WHEP read-back, no
        // need to surrender UDP 8000. Default transports stay enabled.
        let twitch_paths = vec![MediaPath {
            name: "cam".into(),
            source_uri: Some("rtsp://1.2.3.4/main".into()),
            rtsp_transport_tcp: true,
            transcode_from: None,
            push_to: Some("rtmp://live.twitch.tv/app/streamkey".into()),
            pushes_to_srs: false,
        }];
        let yaml = render_config(&cfg, &twitch_paths).unwrap();
        assert!(yaml.contains("rtspTransports: [tcp, udp]"));
        assert!(!yaml.contains("rtpAddress: :0"));
    }

    #[test]
    fn locate_binary_returns_error_for_missing_override() {
        let bogus = PathBuf::from("/definitely/does/not/exist/mediamtx");
        assert!(matches!(
            locate_binary(Some(&bogus)),
            Err(MediamtxError::BinaryMissing)
        ));
    }

    #[test]
    fn rejects_path_name_with_yaml_break() {
        let cfg = MediaGatewayConfig::default();
        let paths = vec![MediaPath {
            // A newline here would let the user inject sibling YAML keys
            // like `runOnInit: bash -c "..."` which mediamtx executes.
            name: "cam1\nrunOnInit: rm -rf /".into(),
            source_uri: Some("rtsp://1.2.3.4/foo".into()),
            rtsp_transport_tcp: true,
            transcode_from: None,
            push_to: None,
            pushes_to_srs: false,
        }];
        assert!(matches!(
            render_config(&cfg, &paths),
            Err(MediamtxError::InvalidConfig(_))
        ));
    }

    #[test]
    fn rejects_url_with_control_char_or_quote() {
        let cfg = MediaGatewayConfig::default();
        let bad_urls = [
            "rtsp://example.com/foo\"--bad-ffmpeg-arg",
            "rtsp://example.com/foo\nrunOnInit: pwn",
            "javascript:alert(1)",
            "ftp://example.com/foo",
        ];
        for u in bad_urls {
            let paths = vec![MediaPath {
                name: "cam".into(),
                source_uri: Some(u.into()),
                rtsp_transport_tcp: true,
                transcode_from: None,
                push_to: None,
                pushes_to_srs: false,
            }];
            assert!(
                matches!(
                    render_config(&cfg, &paths),
                    Err(MediamtxError::InvalidConfig(_))
                ),
                "expected rejection for {u:?}",
            );
        }
    }

    #[test]
    fn rejects_push_to_with_non_rtmp_scheme() {
        let cfg = MediaGatewayConfig::default();
        let paths = vec![MediaPath {
            name: "cam".into(),
            source_uri: None,
            rtsp_transport_tcp: true,
            transcode_from: Some("rtsp://1.2.3.4/foo".into()),
            push_to: Some("http://attacker.example.com/exfil".into()),
            pushes_to_srs: false,
        }];
        assert!(matches!(
            render_config(&cfg, &paths),
            Err(MediamtxError::InvalidConfig(_))
        ));
    }
}
