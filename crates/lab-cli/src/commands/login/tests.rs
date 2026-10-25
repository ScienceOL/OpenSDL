use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
async fn server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        socket.read(&mut buf).await.unwrap();
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    (format!("http://{addr}"), task)
}
#[tokio::test]
async fn polling_approval_pending_denial_and_expiry_over_http() {
    let client = reqwest::Client::new();
    let (url, task) = server("200 OK", r#"{"state":"authorization_pending"}"#).await;
    assert!(poll(&client, &url, "private-code").await.unwrap().is_none());
    task.await.unwrap();
    let binding = r#"{"issuer":"https://liyanlabs.local","channel":"dev","account_id":"owner","lab_id":"11111111-1111-4111-8111-111111111111","instance":"lab","key_id":"key","public_key":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
    let (url, task) = server("200 OK", binding).await;
    assert_eq!(
        poll(&client, &url, "private-code")
            .await
            .unwrap()
            .unwrap()
            .instance,
        "lab"
    );
    task.await.unwrap();
    for (status, want) in [
        ("403 Forbidden", "denied"),
        ("410 Gone", "expired"),
        ("409 Conflict", "consumed"),
    ] {
        let (url, task) = server(status, r#"{"detail":"private-code"}"#).await;
        let error = poll(&client, &url, "private-code")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(want), "{error}");
        assert!(!error.contains("private-code"));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn begin_rejects_cross_origin_urls_and_accepts_public_challenge() {
    let client = reqwest::Client::new();
    for foreign in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let verification = if foreign {
            "https://example.org".to_owned()
        } else {
            origin.clone()
        };
        let body=serde_json::json!({"device_code":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","user_code":"ABCD-EFGH","verification_uri":format!("{verification}/#/osdl/authorize"),"verification_uri_complete":format!("{verification}/#/osdl/authorize?user_code=ABCD-EFGH"),"expires_in":600,"interval":5}).to_string();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            socket.read(&mut buf).await.unwrap();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        assert_eq!(begin(&client, &origin, "lab").await.is_err(), foreign);
        task.await.unwrap();
    }
}
