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
    /// passthrough path (no transcoding) under `id`.
    pub rtsp_main: String,

    /// Optional sub stream URL. When present, an additional H.264 transcode
    /// path `{id}_h264` is generated, fed by this stream, so browsers /
    /// WebRTC can play even when the camera produces HEVC.
    #[serde(default)]
    pub rtsp_sub: Option<String>,

    /// If true, generate the H.264 transcode path. Default: true when
    /// `rtsp_sub` is set, false otherwise.
    #[serde(default)]
    pub h264_transcode: Option<bool>,

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
        self.h264_transcode
            .unwrap_or_else(|| self.rtsp_sub.is_some())
    }

    /// Validate cross-field invariants. Called from `MediaSourceConfig::paths`
    /// before the engine spawns mediamtx, so config errors fail fast.
    pub fn validate(&self) -> Result<(), String> {
        if self.remote_rtmp.is_some() && !self.produces_h264_path() {
            return Err(format!(
                "camera {:?}: remote_rtmp is set but no H.264 path is generated. \
                 Either provide rtsp_sub, or set h264_transcode: true so the \
                 main stream is transcoded.",
                self.id,
            ));
        }
        Ok(())
    }

    /// Whether this camera will advertise remote-ingest endpoints. Mirrors
    /// the actual push behavior so we don't lie to consumers.
    pub fn pushes_to_remote(&self) -> bool {
        self.remote_rtmp.is_some() && self.produces_h264_path()
    }

    pub fn paths(&self) -> Vec<MediaPath> {
        let mut out = vec![MediaPath {
            name: self.id.clone(),
            source_uri: Some(self.rtsp_main.clone()),
            rtsp_transport_tcp: self.rtsp_transport_tcp,
            transcode_from: None,
            push_to: None,
        }];

        if self.produces_h264_path() {
            // Prefer sub stream as transcode source — far cheaper to encode.
            let upstream = self
                .rtsp_sub
                .clone()
                .unwrap_or_else(|| self.rtsp_main.clone());
            let push_to = self
                .remote_rtmp
                .as_ref()
                .map(|r| r.full_push_url(&self.id));
            out.push(MediaPath {
                name: format!("{}_h264", self.id),
                source_uri: None,
                rtsp_transport_tcp: self.rtsp_transport_tcp,
                transcode_from: Some(upstream),
                push_to,
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
    fn rejects_remote_rtmp_without_transcode_path() {
        // remote_rtmp configured, but no rtsp_sub and h264_transcode=false
        // means there's no path that would actually push.
        let mut c = cam("cam1");
        c.h264_transcode = Some(false);
        c.remote_rtmp = Some(rtmp());
        assert!(c.validate().is_err());
        assert!(!c.pushes_to_remote());
    }

    #[test]
    fn accepts_remote_rtmp_with_explicit_transcode() {
        let mut c = cam("cam1");
        c.h264_transcode = Some(true);
        c.remote_rtmp = Some(rtmp());
        assert!(c.validate().is_ok());
        assert!(c.pushes_to_remote());
    }

    #[test]
    fn accepts_remote_rtmp_with_sub_stream_default_transcode() {
        let mut c = cam("cam1");
        c.rtsp_sub = Some("rtsp://1.2.3.4/sub".into());
        c.remote_rtmp = Some(rtmp());
        // h264_transcode defaults to true when sub is set
        assert!(c.validate().is_ok());
        assert!(c.pushes_to_remote());
    }

    #[test]
    fn no_remote_rtmp_is_always_valid() {
        let c = cam("cam1");
        assert!(c.validate().is_ok());
        assert!(!c.pushes_to_remote());
    }
}
