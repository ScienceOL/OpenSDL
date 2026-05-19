//! Media gateway demo — exposes one ONVIF camera as RTSP/HLS/WebRTC.
//!
//! Required arguments (also accepted via OSDL_ONVIF_* env vars):
//!   --host <ip>   --user <name>   --pass <secret>
//!
//! Optional remote ingest:
//!   --rtmp <base-url>          e.g. rtmp://srs.example.com:1935/camera
//!   --stream <name>            defaults to the camera id
//!   --http-host <host:port>    SRS public HTTP host for HLS/FLV URLs
//!   --webrtc-host <host>       SRS public WebRTC host
//!
//! Example:
//!   cargo run --example media_gateway -- \
//!     --host 192.168.1.131 --user onvif_op --pass "$ONVIF_PASS" \
//!     --rtmp rtmp://srs.example.com:1935/camera --stream lab-1
//!
//! The engine spawns mediamtx, prints all generated endpoint URLs, and runs
//! until Ctrl-C. Verify with:
//!   ffprobe -rtsp_transport tcp rtsp://127.0.0.1:8554/cam1
//!   ffplay  -rtsp_transport tcp rtsp://127.0.0.1:8554/cam1_h264
//!   open http://127.0.0.1:8888/cam1_h264

use osdl_core::config::OsdlConfig;
use osdl_core::event::OsdlEvent;
use osdl_core::media::onvif_camera::{OnvifCameraConfig, RemoteRtmpConfig};
use osdl_core::media::MediaSourceConfig;
use osdl_core::OsdlEngine;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Credentials are required CLI args. Hardcoding real passwords in
    // examples leaks them into version control — see camera-bringup
    // README for how the operator account is provisioned.
    let args: Vec<String> = std::env::args().collect();
    let host = arg_value(&args, "--host")
        .or_else(|| std::env::var("OSDL_ONVIF_HOST").ok())
        .unwrap_or_else(|| {
            eprintln!("--host <ip> required (or OSDL_ONVIF_HOST env)");
            std::process::exit(2);
        });
    let user = arg_value(&args, "--user")
        .or_else(|| std::env::var("OSDL_ONVIF_USER").ok())
        .unwrap_or_else(|| {
            eprintln!("--user <name> required (or OSDL_ONVIF_USER env)");
            std::process::exit(2);
        });
    let pass = arg_value(&args, "--pass")
        .or_else(|| std::env::var("OSDL_ONVIF_PASS").ok())
        .unwrap_or_else(|| {
            eprintln!("--pass <secret> required (or OSDL_ONVIF_PASS env)");
            std::process::exit(2);
        });

    let remote_rtmp = arg_value(&args, "--rtmp").map(|base_url| RemoteRtmpConfig {
        base_url,
        stream: arg_value(&args, "--stream"),
        http_host: arg_value(&args, "--http-host"),
        webrtc_host: arg_value(&args, "--webrtc-host"),
    });

    let cam = OnvifCameraConfig {
        id: "cam1".into(),
        description: Some("ONVIF dev camera".into()),
        rtsp_main: format!("rtsp://{user}:{pass}@{host}:554/ch01.264"),
        rtsp_sub: Some(format!("rtsp://{user}:{pass}@{host}:554/ch01_sub.264")),
        h264_transcode: Some(true),
        rtsp_transport_tcp: true,
        remote_rtmp,
    };

    let mut config = OsdlConfig {
        mqtt: None,
        adapters: vec![],
        espnow_gateways: vec![],
        buses: vec![],
        media_sources: vec![MediaSourceConfig::OnvifCamera(cam)],
        media_gateway: Default::default(),
    };
    config.media_gateway.advertise_host = "127.0.0.1".into();

    let mut engine = OsdlEngine::new(config, vec![]);
    let event_rx = engine.take_event_rx();
    let stop = engine.stop_handle();

    // Drain events to stdout in a background task — the engine emits
    // MediaSourceOnline once mediamtx is up.
    tokio::spawn(async move {
        let mut guard = event_rx.lock().await;
        let Some(mut rx) = guard.take() else { return };
        drop(guard);
        while let Some(ev) = rx.recv().await {
            match ev {
                OsdlEvent::MediaSourceOnline { id, description, endpoints } => {
                    println!("\n=== media source online: {id} ({description}) ===");
                    for e in endpoints {
                        println!("  [{:?}] {:?}\t{}", e.location, e.protocol, e.url);
                    }
                    println!();
                }
                OsdlEvent::MediaGatewayDown { reason } => {
                    eprintln!("media gateway down: {reason}");
                }
                _ => {}
            }
        }
    });

    // Stop on Ctrl-C.
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            println!("\nShutting down…");
            let _ = stop.send(true);
        }
    });

    engine.run().await;
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == key {
            return iter.next().cloned();
        }
    }
    None
}
