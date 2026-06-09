//! ONVIF camera as a `MediaSource`. Right now this is purely about *streaming*
//! — the engine does not call ONVIF SOAP itself. The control plane (PTZ,
//! snapshots, presets) lives in tooling outside core.

use serde::{Deserialize, Serialize};

use super::MediaPath;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnvifCameraConfig {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,

    /// Upstream RTSP URL of the high-quality main stream. Re-exposed as a
    /// passthrough path (no transcoding) under `id`. Always required —
    /// the main path is what the host-side gateway publishes by default.
    pub rtsp_main: String,

    /// Optional sub-stream URL. Only consumed when
    /// `h264_transcode_source: sub`, where it lets a CPU-constrained host
    /// transcode from the cheaper low-resolution feed instead of the main
    /// 4K one. Has no effect on the main passthrough path. Leave unset
    /// unless you need that override.
    #[serde(default)]
    pub rtsp_sub: Option<String>,

    /// If true, generate an additional H.264 transcoded path `{id}_h264`
    /// alongside the main passthrough. Lets clients that can't decode
    /// HEVC (older browsers, some WebRTC stacks) still play the camera.
    /// Default false — modern Mac/Win Electron handles HEVC natively.
    #[serde(default)]
    pub h264_transcode: Option<bool>,

    /// Where the H.264 transcode pulls from when `h264_transcode` is on.
    /// Defaults to `Main` so the H.264 path matches the HEVC path's
    /// resolution / quality. Use `Sub` (and set `rtsp_sub`) only when
    /// the host is CPU-constrained and you accept lower-res H.264.
    #[serde(default)]
    pub h264_transcode_source: H264TranscodeSource,

    /// Force RTSP transport=tcp on the upstream pull. Most cameras work
    /// better with TCP (no UDP packet loss); default true.
    #[serde(default = "default_true")]
    pub rtsp_transport_tcp: bool,

    /// Push the H.264 stream to a remote RTMP ingest (e.g. SRS). The
    /// transcoded `{id}_h264` path is reused — ffmpeg uses `-c copy` so
    /// there's no extra encoding cost.
    #[serde(default)]
    pub remote_rtmp: Option<RemoteRtmpConfig>,
}

/// Selects which upstream feeds the optional `{id}_h264` transcode path.
/// Only consulted when `h264_transcode` is on; the HEVC main passthrough
/// path always sources from `rtsp_main` regardless.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum H264TranscodeSource {
    /// Transcode from `rtsp_main`. Same resolution as the HEVC path,
    /// higher CPU on the host.
    #[default]
    Main,
    /// Transcode from `rtsp_sub`. Lower CPU, lower resolution. Requires
    /// `rtsp_sub` to be set; otherwise validation fails.
    Sub,
}

/// Remote RTMP ingest target. Resulting push URL is `{base_url}/{stream}`,
/// e.g. `rtmp://srs.example.com:1935/openSDL/cam1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRtmpConfig {
    /// Base URL up to and including the app, no trailing slash. Example:
    /// `rtmp://srs.sciol.ac.cn:1935/openSDL`.
    pub base_url: String,

    /// Stream name. Defaults to the camera id.
    #[serde(default)]
    pub stream: Option<String>,

    /// Public host:port for HTTP-FLV / HLS playback URLs published by the
    /// SRS server, e.g. `srs.sciol.ac.cn:8080`. Used only to assemble
    /// `MediaEndpoint`s for callers.
    #[serde(default)]
    pub http_host: Option<String>,

    /// Public host (no port) for WebRTC playback. Example:
    /// `srs.sciol.ac.cn`. Used only to assemble `MediaEndpoint`s.
    #[serde(default)]
    pub webrtc_host: Option<String>,
}

impl RemoteRtmpConfig {
    pub fn full_push_url(&self, default_stream: &str) -> String {
        let stream = self.stream.as_deref().unwrap_or(default_stream);
        format!("{}/{}", self.base_url.trim_end_matches('/'), stream)
    }

    /// Best-effort extraction of `app` from `base_url`. Returns `""` if the
    /// URL doesn't have one, in which case advertised playback URLs will be
    /// less useful — caller should still get the push URL.
    pub fn app(&self) -> &str {
        // base_url like rtmp://host[:port]/app — split off scheme then take
        // the last path segment.
        let after_scheme = self
            .base_url
            .splitn(2, "://")
            .nth(1)
            .unwrap_or(&self.base_url);
        match after_scheme.split_once('/') {
            Some((_, app)) => app.trim_matches('/'),
            None => "",
        }
    }
}

fn default_true() -> bool {
    true
}

impl OnvifCameraConfig {
    /// Whether this config will produce a path that pushes to remote RTMP.
    /// `remote_rtmp` only takes effect on the H.264 transcode path; if the
    /// caller disabled transcoding, no push happens. Used by both `paths()`
    /// (to attach `push_to`) and `endpoints()` (to suppress remote URL
    /// advertisement that no one would actually be publishing).
    fn produces_h264_path(&self) -> bool {
        // Default off — modern downstreams (Electron 33+, Safari, Chrome 107+)
        // play HEVC fine, and skipping the transcode preserves 4K quality at
        // near-zero CPU cost. Set explicitly when a legacy / WebRTC consumer
        // really needs an H.264 fallback.
        self.h264_transcode.unwrap_or(false)
    }

    /// Validate cross-field invariants. Called from `MediaSourceConfig::paths`
    /// before the engine spawns mediamtx, so config errors fail fast.
    pub fn validate(&self) -> Result<(), String> {
        // h264_transcode_source = sub  ⇒  rtsp_sub must be present.
        // Without this check the transcode would silently fall back to
        // main, masking a config typo.
        if self.produces_h264_path()
            && self.h264_transcode_source == H264TranscodeSource::Sub
            && self.rtsp_sub.is_none()
        {
            return Err(format!(
                "camera {:?}: h264_transcode_source=sub requires rtsp_sub to be set",
                self.id,
            ));
        }
        Ok(())
    }

    /// Whether this camera will advertise remote-ingest endpoints. With
    /// HEVC-passthrough as the default, any `remote_rtmp` setup pushes —
    /// the transcode path is purely an optional H.264 fallback.
    pub fn pushes_to_remote(&self) -> bool {
        self.remote_rtmp.is_some()
    }

    pub fn paths(&self) -> Vec<MediaPath> {
        // Main path: HEVC passthrough. ffmpeg/mediamtx needs no encoder —
        // RTMP push (when configured) is `-c copy`, ~0 CPU.
        let main_push_to = self
            .remote_rtmp
            .as_ref()
            .map(|r| r.full_push_url(&self.id));
        let mut out = vec![MediaPath {
            name: self.id.clone(),
            source_uri: Some(self.rtsp_main.clone()),
            rtsp_transport_tcp: self.rtsp_transport_tcp,
            transcode_from: None,
            push_to: main_push_to,
        }];

        if self.produces_h264_path() {
            // Optional H.264 transcode for clients that can't play HEVC
            // (some browser WebRTC stacks, older mobile decoders).
            //
            // Source: `Main` (default) so the H.264 path matches the HEVC
            // path's resolution; `Sub` for CPU-constrained hosts willing
            // to accept the low-res feed. validate() guarantees rtsp_sub
            // is set whenever we reach the Sub branch here.
            let upstream = match self.h264_transcode_source {
                H264TranscodeSource::Main => self.rtsp_main.clone(),
                H264TranscodeSource::Sub => self
                    .rtsp_sub
                    .clone()
                    .expect("validate() ensures rtsp_sub is Some when source=sub"),
            };
            // Don't double-push to remote — the HEVC main path already does.
            // Local-only H.264 fallback for now.
            out.push(MediaPath {
                name: format!("{}_h264", self.id),
                source_uri: None,
                rtsp_transport_tcp: self.rtsp_transport_tcp,
                transcode_from: Some(upstream),
                push_to: None,
            });
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(id: &str) -> OnvifCameraConfig {
        OnvifCameraConfig {
            id: id.into(),
            description: None,
            rtsp_main: "rtsp://1.2.3.4/main".into(),
            rtsp_sub: None,
            h264_transcode: None,
            h264_transcode_source: H264TranscodeSource::Main,
            rtsp_transport_tcp: true,
            remote_rtmp: None,
        }
    }

    fn rtmp() -> RemoteRtmpConfig {
        RemoteRtmpConfig {
            base_url: "rtmp://srs.example.com:1935/camera".into(),
            stream: None,
            http_host: Some("srs.example.com:8080".into()),
            webrtc_host: Some("srs.example.com".into()),
        }
    }

    #[test]
    fn remote_rtmp_pushes_main_path_by_default() {
        // Modern default: HEVC main passthrough is what gets pushed.
        // No explicit transcode needed.
        let mut c = cam("cam1");
        c.remote_rtmp = Some(rtmp());
        assert!(c.validate().is_ok());
        assert!(c.pushes_to_remote());

        let paths = c.paths();
        assert_eq!(paths.len(), 1, "no transcode path by default");
        assert_eq!(paths[0].name, "cam1");
        assert!(paths[0].push_to.is_some(), "main path carries push_to");
        assert!(paths[0].transcode_from.is_none());
    }

    #[test]
    fn explicit_h264_transcode_adds_local_only_h264_path() {
        let mut c = cam("cam1");
        c.rtsp_sub = Some("rtsp://1.2.3.4/sub".into());
        c.h264_transcode = Some(true);
        c.remote_rtmp = Some(rtmp());
        let paths = c.paths();
        assert_eq!(paths.len(), 2);
        // Main HEVC: source + push_to (the remote push lives here now).
        assert_eq!(paths[0].name, "cam1");
        assert!(paths[0].push_to.is_some());
        // H.264 fallback: transcoded from MAIN (matches HEVC path quality),
        // local-only (no double push to remote).
        assert_eq!(paths[1].name, "cam1_h264");
        assert!(paths[1].push_to.is_none(), "no double-push to remote");
        assert_eq!(
            paths[1].transcode_from.as_deref(),
            Some("rtsp://1.2.3.4/main"),
            "h264 transcode should source from main, not sub",
        );
    }

    #[test]
    fn no_remote_rtmp_is_always_valid() {
        let c = cam("cam1");
        assert!(c.validate().is_ok());
        assert!(!c.pushes_to_remote());
        let paths = c.paths();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].push_to.is_none());
    }

    #[test]
    fn h264_transcode_without_remote_is_pure_local_addition() {
        let mut c = cam("cam1");
        c.h264_transcode = Some(true);
        let paths = c.paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|p| p.push_to.is_none()));
    }

    #[test]
    fn h264_transcode_source_sub_uses_rtsp_sub() {
        let mut c = cam("cam1");
        c.rtsp_sub = Some("rtsp://1.2.3.4/sub".into());
        c.h264_transcode = Some(true);
        c.h264_transcode_source = H264TranscodeSource::Sub;
        assert!(c.validate().is_ok());
        let paths = c.paths();
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[1].transcode_from.as_deref(),
            Some("rtsp://1.2.3.4/sub"),
            "source=sub should pull rtsp_sub",
        );
    }

    #[test]
    fn h264_transcode_source_sub_without_rtsp_sub_fails_validation() {
        let mut c = cam("cam1");
        // rtsp_sub deliberately left None
        c.h264_transcode = Some(true);
        c.h264_transcode_source = H264TranscodeSource::Sub;
        assert!(c.validate().is_err());
    }

    #[test]
    fn h264_transcode_source_sub_with_transcode_off_is_silently_ignored() {
        // Source override only matters when h264_transcode is on.
        let mut c = cam("cam1");
        c.h264_transcode_source = H264TranscodeSource::Sub;
        // No rtsp_sub, no h264_transcode — validate should still pass.
        assert!(c.validate().is_ok());
        assert_eq!(c.paths().len(), 1);
    }
}
