use super::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

async fn serve_camera(
    listener: TcpListener,
    exchanges: Vec<(&str, &str, String)>,
    stopped: mpsc::UnboundedSender<()>,
) {
    for (path, request, response) in exchanges {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert!(line.starts_with(&format!("POST {path} HTTP/1.1")), "{line}");
        let mut length = None;
        loop {
            line.clear();
            assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    length = Some(value.trim().parse().unwrap());
                }
            }
        }
        let mut body = vec![0; length.unwrap()];
        stream.read_exact(&mut body).await.unwrap();
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains(request), "expected {request} in {body}");
        let envelope = format!("<s:Envelope><s:Body>{response}</s:Body></s:Envelope>");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{envelope}",
            envelope.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        if request == "<tptz:Stop>" {
            stopped.send(()).unwrap();
        }
    }
}

#[tokio::test]
async fn continuous_move_sends_camera_timeout_and_backup_stop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let exchanges = vec![
        ("/device", "<tds:GetCapabilities>", capabilities(&base)),
        (
            "/media",
            "<trt:GetProfiles/>",
            concat!(
                r#"<trt:GetProfilesResponse><trt:Profiles token="main">"#,
                r#"<tt:PTZConfiguration token="config&amp;1"/></trt:Profiles>"#,
                r#"<trt:Profiles token="next"><tt:PTZConfiguration token="other"/>"#,
                "</trt:Profiles></trt:GetProfilesResponse>"
            )
            .into(),
        ),
        (
            "/ptz",
            "<tptz:ConfigurationToken>config&amp;1</tptz:ConfigurationToken>",
            options("PT0.1S"),
        ),
        (
            "/ptz",
            "</tptz:Velocity>\n  <tptz:Timeout>PT0.500S</tptz:Timeout>",
            String::new(),
        ),
        ("/ptz", "<tptz:Stop>", String::new()),
        (
            "/ptz",
            "</tptz:Velocity>\n  <tptz:Timeout>PT0.500S</tptz:Timeout>",
            String::new(),
        ),
        ("/ptz", "<tptz:Stop>", String::new()),
    ];
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let server = tokio::spawn(serve_camera(listener, exchanges, stopped));
    let camera = camera(&base, None);
    for _ in 0..2 {
        let started = tokio::time::Instant::now();
        let moved = camera
            .dispatch("ptz_move", &json!({"pan": 0.5, "duration_ms": 500}))
            .await;
        assert_eq!(moved["ok"], true, "{moved}");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), stops.recv())
                .await
                .unwrap(),
            Some(())
        );
        assert!(started.elapsed() >= Duration::from_millis(500));
    }
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn continuous_move_rejects_camera_minimum_above_requested_duration() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let exchanges = vec![
        ("/device", "<tds:GetCapabilities>", capabilities(&base)),
        (
            "/media",
            "<trt:ProfileToken>override</trt:ProfileToken>",
            concat!(
                r#"<trt:GetProfileResponse><trt:Profile token="override">"#,
                r#"<tt:PTZConfiguration token="selected"/></trt:Profile></trt:GetProfileResponse>"#
            )
            .into(),
        ),
        (
            "/ptz",
            "<tptz:ConfigurationToken>selected</tptz:ConfigurationToken>",
            options("PT0.5001S"),
        ),
    ];
    let server = tokio::spawn(serve_camera(
        listener,
        exchanges,
        mpsc::unbounded_channel().0,
    ));
    let camera = camera(&base, Some("override".into()));
    let moved = camera
        .dispatch("ptz_move", &json!({"pan": 0.5, "duration_ms": 500}))
        .await;
    assert_eq!(moved["ok"], false, "{moved}");
    assert!(
        moved["error"]
            .as_str()
            .unwrap()
            .contains("outside camera PTZ timeout range"),
        "{moved}"
    );
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

fn capabilities(base: &str) -> String {
    format!("<tds:GetCapabilitiesResponse><tds:Capabilities><tt:Media><tt:XAddr>{base}/media</tt:XAddr></tt:Media><tt:PTZ><tt:XAddr>{base}/ptz</tt:XAddr></tt:PTZ></tds:Capabilities></tds:GetCapabilitiesResponse>")
}

fn options(min: &str) -> String {
    format!("<tptz:GetConfigurationOptionsResponse><tptz:PTZConfigurationOptions><tt:PTZTimeout><tt:Min>{min}</tt:Min><tt:Max>PT1M</tt:Max></tt:PTZTimeout></tptz:PTZConfigurationOptions></tptz:GetConfigurationOptionsResponse>")
}

fn camera(base: &str, profile: Option<String>) -> OnvifTransport {
    OnvifTransport::new(
        "loopback".into(),
        format!("{base}/device"),
        String::new(),
        String::new(),
        profile,
        PathBuf::new(),
        None,
        mpsc::unbounded_channel().0,
    )
}
