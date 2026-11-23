use super::{now_millis, MediaSourceState, OsdlEngine};
use crate::event::OsdlEvent;
use crate::media::{
    mediamtx::MediamtxProcess,
    onvif_camera::{OnvifCameraConfig, OnvifControlConfig},
    snapshot::{self, CaptureInput, SnapshotTransport},
    v4l2::V4l2Capture,
    Location, MediaSourceConfig, Protocol,
};
use crate::protocol::Device;
use crate::transport::onvif::{transport_id_for as onvif_transport_id, OnvifTransport};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

impl MediaSourceState {
    /// Both capture and publishing read from the device's gateway listener.
    /// The advertised host can be a different address for remote viewers.
    pub fn local_rtsp_input(&self, port: u16) -> Option<String> {
        let h264 = format!("{}_h264", self.id);
        self.endpoints
            .iter()
            .filter(|endpoint| {
                endpoint.protocol == Protocol::Rtsp && endpoint.location == Location::Local
            })
            .max_by_key(|endpoint| endpoint.path == h264)
            .map(|endpoint| format!("rtsp://127.0.0.1:{port}/{}", endpoint.path))
    }

    fn online_event(&self) -> OsdlEvent {
        OsdlEvent::MediaSourceOnline {
            id: self.id.clone(),
            description: self.description.clone(),
            endpoints: self.endpoints.clone(),
        }
    }
}

impl OsdlEngine {
    pub(super) async fn media_gateway_down(&self, reason: String) {
        let mut sources = self.handle.media_sources.write().await;
        sources.retain(|_, source| source.v4l2.is_some());
        self.handle.emit(OsdlEvent::MediaGatewayDown { reason });
        // Consumers clear their gateway snapshot on this event. Replay direct
        // capture sources, which remain available without MediaMTX.
        for source in sources.values() {
            self.handle.emit(source.online_event());
        }
    }

    /// Start MediaMTX for RTSP-backed sources and register direct capture sources.
    /// Returns the process handle so the caller can shut it down.
    pub(super) async fn start_media_gateway(&self) -> Option<MediamtxProcess> {
        let cfg = &self.handle.config;
        if cfg.media_sources.is_empty() {
            return None;
        }

        // Validate all sources before spawning. Catches misconfigurations
        // like remote_rtmp without a transcode path that would otherwise
        // silently produce dead playback URLs.
        let mut ids = HashSet::new();
        for src in &cfg.media_sources {
            let valid = src.validate().and_then(|()| {
                ids.insert(src.id())
                    .then_some(())
                    .ok_or_else(|| format!("Duplicate media source ID: {}", src.id()))
            });
            if let Err(reason) = valid {
                log::error!("Media source rejected: {reason}");
                self.handle.emit(OsdlEvent::MediaGatewayDown { reason });
                return None;
            }
        }

        let gateway = &cfg.media_gateway;
        let all_paths: Vec<_> = cfg
            .media_sources
            .iter()
            .flat_map(MediaSourceConfig::paths)
            .collect();
        let proc = if all_paths.is_empty() {
            None
        } else {
            match MediamtxProcess::spawn(gateway, &all_paths).await {
                Ok(proc) => Some(proc),
                Err(error) => {
                    log::error!("Failed to start media gateway: {error}");
                    self.handle.emit(OsdlEvent::MediaGatewayDown {
                        reason: error.to_string(),
                    });
                    None
                }
            }
        };

        // Commit each entry to the shared map BEFORE emitting the
        // live event. A subscriber that races the emit window finds
        // the snapshot already populated; a subscriber that arrives
        // after engine startup sees both the snapshot replay and any
        // live event still in the broadcast buffer (clients dedupe by
        // camera id). The reverse order — emit first, commit later —
        // creates a window where a fresh subscriber observes neither.
        let mut registered = HashMap::new();
        let mut online = Vec::new();
        let mut capture_paths = HashSet::new();
        let ffmpeg = gateway.publisher_binary();
        let snapshot_root = self
            .handle
            .snapshot_root()
            .unwrap_or_else(|| std::env::temp_dir().join("osdl-snapshots"));
        for src in &cfg.media_sources {
            let (endpoints, v4l2) = match src {
                MediaSourceConfig::OnvifCamera(_) if proc.is_some() => {
                    (src.endpoints(&gateway.advertise_host, &gateway.ports), None)
                }
                MediaSourceConfig::OnvifCamera(_) => continue,
                MediaSourceConfig::V4l2Camera(camera) => {
                    let Some(executable) = ffmpeg else {
                        log::warn!("V4L2 source {} requires an FFmpeg binary", src.id());
                        continue;
                    };
                    if !matches!(camera.supports_mjpeg(), Ok(true)) {
                        log::warn!("V4L2 source {} cannot open MJPEG capture", src.id());
                        continue;
                    }
                    let device = match std::fs::canonicalize(&camera.device) {
                        Ok(device) => device,
                        Err(_) => continue,
                    };
                    if !capture_paths.insert(device) {
                        log::warn!(
                            "V4L2 source {} duplicates a configured capture device",
                            src.id()
                        );
                        continue;
                    }
                    (
                        Vec::new(),
                        Some(Arc::new(V4l2Capture::new(
                            executable.to_owned(),
                            camera.clone(),
                        ))),
                    )
                }
            };
            let source = MediaSourceState {
                id: src.id().to_string(),
                description: src.description().to_string(),
                endpoints,
                v4l2,
            };
            if let MediaSourceConfig::OnvifCamera(camera) = src {
                if let Some(control) = &camera.control {
                    let fallback = ffmpeg.and_then(|executable| {
                        let url = source.local_rtsp_input(gateway.ports.rtsp)?;
                        Some(CaptureInput::Rtsp {
                            executable: executable.to_owned(),
                            url,
                        })
                    });
                    self.register_onvif_control_device(camera, control, fallback, &snapshot_root)
                        .await;
                }
            } else if let Some(camera) = &source.v4l2 {
                self.register_capture_device(
                    &source.id,
                    &source.description,
                    CaptureInput::V4l2(camera.clone()),
                    &snapshot_root,
                )
                .await;
            }
            online.push(source.online_event());
            log::info!("Media source online: {}", src.id());
            registered.insert(source.id.clone(), source);
        }
        let mut sources_map = self.handle.media_sources.write().await;
        *sources_map = registered;
        for event in online {
            self.handle.emit(event);
        }
        drop(sources_map);
        self.handle.broadcast_status().await;
        proc
    }

    async fn register_onvif_control_device(
        &self,
        camera: &OnvifCameraConfig,
        control: &OnvifControlConfig,
        fallback: Option<CaptureInput>,
        root: &Path,
    ) {
        let OnvifControlConfig {
            onvif_url,
            username,
            password,
            profile_token,
        } = control;
        let transport_id = onvif_transport_id(&camera.id);
        let transport = Arc::new(OnvifTransport::new(
            camera.id.clone(),
            onvif_url.clone(),
            username.clone(),
            password.clone(),
            profile_token.clone(),
            root.to_owned(),
            fallback,
            self.handle.transport_rx_tx.clone(),
        ));
        self.handle
            .transports
            .write()
            .await
            .insert(transport_id.clone(), transport);
        self.publish_discovered_device(camera_device(
            &camera.id,
            transport_id,
            crate::adapter::onvif::DEVICE_TYPE_COMBINED,
            crate::adapter::onvif::PLATFORM,
            camera
                .description
                .clone()
                .unwrap_or_else(|| format!("ONVIF camera {}", camera.id)),
            crate::adapter::onvif::OnvifAdapter::combined_actions(),
        ))
        .await;
    }

    async fn register_capture_device(
        &self,
        id: &str,
        description: &str,
        input: CaptureInput,
        root: &Path,
    ) {
        let transport_id = snapshot::transport_id_for(id);
        let transport = Arc::new(SnapshotTransport {
            camera_id: id.into(),
            root: root.to_owned(),
            input,
        });
        self.handle
            .transports
            .write()
            .await
            .insert(transport_id.clone(), transport);
        if let Err(error) = self
            .handle
            .publish_color_capture(camera_device(
                id,
                transport_id,
                snapshot::DEVICE_TYPE,
                snapshot::PLATFORM,
                description.into(),
                vec![snapshot::action()],
            ))
            .await
        {
            log::warn!("{error}");
        }
    }
}

fn camera_device(
    id: &str,
    transport_id: String,
    device_type: &str,
    adapter: &str,
    description: String,
    actions: Vec<crate::protocol::ActionSchema>,
) -> Device {
    Device {
        id: id.into(),
        transport_id,
        device_type: device_type.into(),
        adapter: adapter.into(),
        description,
        online: true,
        properties: HashMap::new(),
        properties_timestamp_ms: now_millis(),
        actions,
        role: Some("camera".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OsdlConfig;
    use crate::protocol::{CommandStatus, DeviceCommand};

    #[tokio::test]
    async fn depth_probe_registered_before_color_capture_shares_camera_identity() {
        assert_combined_depth_camera(true).await;
    }

    #[tokio::test]
    async fn color_capture_registered_before_depth_probe_shares_camera_identity() {
        assert_combined_depth_camera(false).await;
    }

    async fn assert_combined_depth_camera(probe_first: bool) {
        let directory = tempfile::tempdir().unwrap();
        let jpeg = [0xff, 0xd8, 0xff, 0xd9];
        let id = "gemini-335";
        let config: MediaSourceConfig = serde_yaml::from_str(
            "kind: v4l2_camera\nid: gemini-335\ndevice: /dev/opensdl/gemini-335-color\nwidth: 640\nheight: 480\nframes_per_second: 15",
        ).unwrap();
        let engine = OsdlEngine::new(
            OsdlConfig {
                data_dir: Some(directory.path().into()),
                media_sources: vec![config],
                ..Default::default()
            },
            Vec::new(),
        );
        let handle = engine.handle();
        let snapshot_root = handle.snapshot_root().unwrap();
        let probe = serde_yaml::from_str(
            "depth_device: /dev/opensdl/gemini-335-depth\ncolor_device: /dev/opensdl/gemini-335-color",
        )
        .unwrap();
        let register_probe = || {
            handle.register_depth_camera_probe(id, "v4l2.depth_camera", "Orbbec Gemini 335", &probe)
        };
        let register_capture = || {
            engine.register_capture_device(
                id,
                "Gemini color",
                CaptureInput::Rtsp {
                    executable: snapshot::tests::fake_ffmpeg(directory.path(), &jpeg),
                    url: "rtsp://127.0.0.1:8554/camera".into(),
                },
                &snapshot_root,
            )
        };
        let properties = Some(HashMap::from([
            ("depth_available".into(), serde_json::Value::Bool(true)),
            ("color_available".into(), serde_json::Value::Bool(true)),
        ]));
        let observer = if probe_first {
            let observer = register_probe().await.unwrap();
            observer.observe(properties).await;
            register_capture().await;
            observer
        } else {
            register_capture().await;
            let observer = register_probe().await.unwrap();
            observer.observe(properties).await;
            observer
        };
        let devices = handle.list_devices().await;
        assert_eq!(devices.len(), 1);
        let camera = handle.get_device(id).await.unwrap();
        assert_eq!(camera.id, id);
        assert_eq!(camera.device_type, "v4l2.depth_camera");
        assert_eq!(camera.description, "Orbbec Gemini 335");
        assert_eq!(camera.role.as_deref(), Some("camera"));
        assert!(camera.online);
        assert_eq!(camera.properties["depth_available"], true);
        assert_eq!(camera.properties["color_available"], true);
        assert_eq!(camera.actions.len(), 1);
        assert_eq!(camera.actions[0].name, "snapshot");
        assert!(handle
            .register_read_only_device(id, "tcp", "Unrelated probe", None)
            .await
            .is_err());
        let unrelated_depth = serde_yaml::from_str(
            "depth_device: /dev/v4l/by-id/other-depth\ncolor_device: /dev/v4l/by-id/other-color",
        )
        .unwrap();
        assert!(handle
            .register_depth_camera_probe(
                id,
                "v4l2.depth_camera",
                "Unrelated camera",
                &unrelated_depth
            )
            .await
            .is_err());
        let receipt = handle
            .send_command(DeviceCommand {
                command_id: "capture".into(),
                device_id: id.into(),
                action: "snapshot".into(),
                params: serde_json::json!({}),
            })
            .await
            .unwrap();
        assert_eq!(receipt.command_id, "capture");
        assert_eq!(receipt.status, CommandStatus::Succeeded);
        let data = receipt.data.unwrap();
        let path = data["properties"]["snapshot_path"].as_str().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), jpeg);
        observer.observe(None).await;
        let offline = handle.get_device(id).await.unwrap();
        assert!(!offline.online);
        assert!(offline.properties.is_empty());
        assert_eq!(offline.actions[0].name, "snapshot");
    }

    #[tokio::test]
    async fn capture_only_camera_has_a_correlated_snapshot_action_and_no_motion() {
        let directory = tempfile::tempdir().unwrap();
        let jpeg = [0xff, 0xd8, 0xff, 0xd9];
        let config: MediaSourceConfig = serde_yaml::from_str(
            "kind: onvif_camera\nid: color\nrtsp_main: rtsp://example.invalid/camera\n",
        )
        .unwrap();
        let engine = OsdlEngine::new(
            OsdlConfig {
                data_dir: Some(directory.path().into()),
                media_sources: vec![config],
                ..Default::default()
            },
            Vec::new(),
        );
        engine
            .register_capture_device(
                "color",
                "Color camera",
                CaptureInput::Rtsp {
                    executable: snapshot::tests::fake_ffmpeg(directory.path(), &jpeg),
                    url: "rtsp://127.0.0.1:8554/color".into(),
                },
                &engine.handle().snapshot_root().unwrap(),
            )
            .await;
        let handle = engine.handle();
        let device = handle.get_device("color").await.unwrap();
        assert_eq!(device.role.as_deref(), Some("camera"));
        assert_eq!(device.actions.len(), 1);
        assert_eq!(device.actions[0].name, "snapshot");
        assert!(handle
            .list_devices()
            .await
            .iter()
            .any(|device| device.id == "color"));

        let command = |action: &str| DeviceCommand {
            device_id: "color".into(),
            command_id: action.into(),
            action: action.into(),
            params: serde_json::json!({}),
        };
        let result = handle.send_command(command("snapshot")).await.unwrap();
        assert_eq!(result.command_id, "snapshot");
        assert_eq!(result.device_id, "color");
        assert_eq!(result.status, CommandStatus::Succeeded);
        let data = result.data.unwrap();
        let path = data["properties"]["snapshot_path"].as_str().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), jpeg);
        assert!(handle.send_command(command("ptz_move")).await.is_err());
    }
}
