//! Media sources — cameras and other continuous streams that don't fit the
//! command/response `Transport` + `ProtocolAdapter` model.
//!
//! A `MediaSource` declares one or more upstream RTSP URLs and how they
//! should be re-exposed (passthrough or transcoded). The engine renders these
//! into a single mediamtx config and supervises one mediamtx process.
//!
//! The engine does not implement the camera control plane (PTZ / snapshot /
//! presets) — that lives outside core for now. Core's job is to make the
//! stream available to consumers (browsers, AI pipelines, recorders) at
//! stable URLs.

pub mod mediamtx;
pub mod onvif_camera;

use serde::{Deserialize, Serialize};

/// One upstream media source declared in `OsdlConfig.media_sources`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSourceConfig {
    OnvifCamera(onvif_camera::OnvifCameraConfig),
}

impl MediaSourceConfig {
    pub fn id(&self) -> &str {
        match self {
            Self::OnvifCamera(c) => &c.id,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::OnvifCamera(c) => c.description.as_deref().unwrap_or(""),
        }
    }

    /// Validate cross-field invariants. Currently this just delegates per
    /// source kind; it exists at this level so the engine can call it on
    /// every source up-front.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::OnvifCamera(c) => c.validate(),
        }
    }

    /// Render the mediamtx `paths:` entries this source contributes.
    pub fn paths(&self) -> Vec<MediaPath> {
        match self {
            Self::OnvifCamera(c) => c.paths(),
        }
    }

    /// External-facing endpoints to advertise via `MediaSourceOnline`.
    /// Includes both local gateway endpoints (RTSP/HLS/WebRTC at the
    /// `gateway_host`) and any configured remote ingest playback URLs.
    ///
    /// Local WebRTC remains advertised even when this source pushes to
    /// remote ingest. mediamtx pins its own ICE UDP mux away from SRS's
    /// UDP 8000, so local viewers can stay on the low-latency LAN path
    /// while teammates use SRS.
    pub fn endpoints(
        &self,
        gateway_host: &str,
        gateway: &mediamtx::ListenerPorts,
    ) -> Vec<MediaEndpoint> {
        let mut out = Vec::new();
        for p in self.paths() {
            for proto in [Protocol::Rtsp, Protocol::Hls] {
                out.push(MediaEndpoint {
                    source_id: self.id().to_string(),
                    path: p.name.clone(),
                    protocol: proto,
                    url: build_url(gateway_host, gateway, proto, &p.name),
                    location: Location::Local,
                });
            }
            out.push(MediaEndpoint {
                source_id: self.id().to_string(),
                path: p.name.clone(),
                protocol: Protocol::Webrtc,
                url: build_url(gateway_host, gateway, Protocol::Webrtc, &p.name),
                location: Location::Local,
            });
        }
        out.extend(self.remote_endpoints());
        out
    }

    /// Remote ingest playback endpoints, if any are configured. Suppressed
    /// when this source isn't actually pushing to remote (e.g. camera with
    /// remote_rtmp set but transcoding disabled — no path to publish from).
    fn remote_endpoints(&self) -> Vec<MediaEndpoint> {
        match self {
            Self::OnvifCamera(c) if c.pushes_to_remote() => c
                .remote_rtmp
                .as_ref()
                .map(|r| build_remote_endpoints(&c.id, r))
                .unwrap_or_default(),
            Self::OnvifCamera(_) => Vec::new(),
        }
    }
}

fn build_remote_endpoints(id: &str, remote: &onvif_camera::RemoteRtmpConfig) -> Vec<MediaEndpoint> {
    let stream = remote.stream.as_deref().unwrap_or(id);
    let app = remote.app();
    let mut out = vec![MediaEndpoint {
        source_id: id.to_string(),
        path: stream.to_string(),
        protocol: Protocol::Rtmp,
        url: remote.full_push_url(id),
        location: Location::Remote,
    }];
    if let Some(http_host) = &remote.http_host {
        let path = if app.is_empty() {
            stream.to_string()
        } else {
            format!("{app}/{stream}")
        };
        out.push(MediaEndpoint {
            source_id: id.to_string(),
            path: stream.to_string(),
            protocol: Protocol::Flv,
            url: format!("http://{http_host}/{path}.flv"),
            location: Location::Remote,
        });
        out.push(MediaEndpoint {
            source_id: id.to_string(),
            path: stream.to_string(),
            protocol: Protocol::Hls,
            url: format!("http://{http_host}/{path}.m3u8"),
            location: Location::Remote,
        });
    }
    if let Some(webrtc_host) = &remote.webrtc_host {
        // SRS WHEP endpoint. WHEP is HTTP-POST-application/sdp, so the URL
        // must be a real http(s):// — a `webrtc://` URI would crash the
        // fetch() in CameraTile.tsx before any candidate exchange happens.
        //
        // SRS puts WHEP under /rtc/v1/whep/ and takes the stream path as
        // query params (`app=` + `stream=`) rather than URL segments,
        // because the resource being POSTed to is the *WHEP signalling
        // endpoint*, not the stream itself.
        //
        // mediamtx's local WHEP URL shape is different
        // (`http://host:port/<path>/whep`, see `build_url`) — we don't try
        // to unify them because the local and remote viewers use different
        // fetch code paths in practice (the local one has no `app`).
        //
        // Production cameras are pinned to H.264 Baseline. Make that explicit
        // for SRS so Electron builds with optional HEVC WebRTC support don't
        // send a broader offer that SRS answers on the wrong codec path.
        let app_q = if app.is_empty() {
            "live".to_string()
        } else {
            app.to_string()
        };
        out.push(MediaEndpoint {
            source_id: id.to_string(),
            path: stream.to_string(),
            protocol: Protocol::Webrtc,
            url: format!(
                "http://{webrtc_host}/rtc/v1/whep/?app={app_q}&stream={stream}&codec=h264"
            ),
            location: Location::Remote,
        });
    }
    out
}

/// An entry in the rendered mediamtx config.
///
/// `source_uri` is the upstream RTSP URL when mediamtx pulls directly.
/// `transcode_from` is set when an ffmpeg sidecar must transcode another path
/// or upstream URL into this one (used for optional H.264 fallback rewrites).
/// `push_to` is set when this path's output should additionally be republished
/// to a remote ingest (e.g. RTMP at SRS) — uses ffmpeg `-c copy`, no extra
/// encoding cost since the path is already H.264.
#[derive(Debug, Clone)]
pub struct MediaPath {
    pub name: String,
    pub source_uri: Option<String>,
    pub rtsp_transport_tcp: bool,
    pub transcode_from: Option<String>,
    pub push_to: Option<String>,
}

/// One protocol/URL pair pointing at either the local gateway or a remote
/// ingest server (when push-to-RTMP is configured).
#[derive(Debug, Clone, Serialize)]
pub struct MediaEndpoint {
    pub source_id: String,
    pub path: String,
    pub protocol: Protocol,
    pub url: String,
    pub location: Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Rtsp,
    Hls,
    Webrtc,
    Rtmp,
    Flv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Location {
    /// Served by the local mediamtx gateway.
    Local,
    /// Served by a remote ingest (e.g. SRS) that this source pushes to.
    Remote,
}

fn build_url(host: &str, ports: &mediamtx::ListenerPorts, proto: Protocol, path: &str) -> String {
    match proto {
        Protocol::Rtsp => format!("rtsp://{}:{}/{}", host, ports.rtsp, path),
        Protocol::Hls => format!("http://{}:{}/{}/index.m3u8", host, ports.hls, path),
        // mediamtx serves the WHEP negotiation endpoint at `<path>/whep`
        // (the bare `<path>` is the HTML demo player). Subscribers POST
        // their SDP offer to `/whep` and receive the answer in the
        // response body. Without the suffix, mediamtx returns 404.
        Protocol::Webrtc => format!("http://{}:{}/{}/whep", host, ports.webrtc, path),
        // Rtmp/Flv are remote-only; build_url is only called for local
        // gateway endpoints. Reaching here would indicate a programming bug.
        Protocol::Rtmp | Protocol::Flv => unreachable!("RTMP/FLV are remote-only"),
    }
}
