//! Same-user discovery credentials are never part of the public lockfile.
use anyhow::{ensure, Result};
use osdl_auth::ProfileStore;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAccess {
    pub endpoint: String,
    pub token: String,
}
impl LocalAccess {
    pub fn store(lock_dir: &Path, instance: &str) -> Result<ProfileStore> {
        ensure!(
            !instance.is_empty()
                && instance.len() <= 80
                && instance
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid_instance"
        );
        ProfileStore::open(lock_dir.join(format!("private-{instance}")))
    }
    pub fn load(lock_dir: &Path, instance: &str) -> Result<Self> {
        let store = Self::store(lock_dir, instance)?;
        Ok(serde_json::from_slice(&store.read_secret("discovery")?)?)
    }
    pub fn save(&self, lock_dir: &Path, instance: &str) -> Result<()> {
        Self::store(lock_dir, instance)?.write_secret("discovery", &serde_json::to_vec(self)?, true)
    }
    pub fn generate(endpoint: String) -> Result<Self> {
        use base64::Engine;
        let mut bytes = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .map_err(|_| anyhow::anyhow!("entropy_unavailable"))?;
        Ok(Self {
            endpoint,
            token: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_local_loopback_requires_private_discovery_capability() {
        let temp = tempfile::tempdir().unwrap();
        let locks = temp.path().join("locks");
        let engine = osdl_core::OsdlEngine::new(osdl_core::config::OsdlConfig::default(), vec![]);
        let handle = engine.handle();
        let config = crate::ServeConfig {
            instance: "private".into(),
            listen: crate::ListenConfig {
                socket_path: None,
                tcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            },
            lock_dir: locks.clone(),
            auth: crate::auth::ListenerAuth::Local,
        };
        let service = tokio::spawn(crate::serve(handle, config));
        let access = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Ok(access) = LocalAccess::load(&locks, "private") {
                    break access;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let public =
            serde_json::to_string(&crate::lockfile::find(&locks, "private").unwrap()).unwrap();
        assert!(!public.contains(&access.token));
        let mut client = osdl_proto::v1::osdl_client::OsdlClient::connect(access.endpoint)
            .await
            .unwrap();
        assert_eq!(
            client
                .status(osdl_proto::v1::StatusRequest {})
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        let mut request = tonic::Request::new(osdl_proto::v1::ShutdownRequest {});
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", access.token).parse().unwrap(),
        );
        client.shutdown(request).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), service)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    #[test]
    fn local_mode_rejects_remote_bindings() {
        for addr in ["0.0.0.0:50051", "192.168.1.2:50051", "100.64.0.9:50051"] {
            assert!(crate::auth::ListenerAuth::Local
                .validate_listener("test", Some(addr.parse().unwrap()))
                .is_err());
        }
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    #[tokio::test]
    async fn protected_unix_socket_is_local_authority() {
        let temp = tempfile::tempdir().unwrap();
        let run = osdl_auth::ProfileStore::open(temp.path().join("run")).unwrap();
        let path = run.root().join("lab.sock");
        let engine = osdl_core::OsdlEngine::new(osdl_core::config::OsdlConfig::default(), vec![]);
        let service = tokio::spawn(crate::serve(
            engine.handle(),
            crate::ServeConfig {
                instance: "unix".into(),
                listen: crate::ListenConfig {
                    socket_path: Some(path.clone()),
                    tcp_addr: None,
                },
                lock_dir: temp.path().join("locks"),
                auth: crate::auth::ListenerAuth::Local,
            },
        ));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !path.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let channel = tonic::transport::Endpoint::from_static("http://[::]")
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(
                        tokio::net::UnixStream::connect(path).await?,
                    ))
                }
            }))
            .await
            .unwrap();
        let mut client = osdl_proto::v1::osdl_client::OsdlClient::new(channel);
        assert!(client
            .status(osdl_proto::v1::StatusRequest {})
            .await
            .is_ok());
        client
            .shutdown(osdl_proto::v1::ShutdownRequest {})
            .await
            .unwrap();
        service.await.unwrap().unwrap();
    }
}
