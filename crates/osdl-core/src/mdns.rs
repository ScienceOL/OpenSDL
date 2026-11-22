//! mDNS service advertisement for automatic node discovery.
//!
//! The mother node advertises `_osdl._tcp.local` so that ESP32 nodes
//! can find the MQTT broker IP:port without any hardcoded configuration.
//! This is the same mechanism used by AirPlay, Chromecast, and Home Assistant.

use std::net::SocketAddr;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

/// Advertises the OpenSDL MQTT broker via mDNS.
///
/// Holds the `ServiceDaemon` — when dropped, the advertisement stops.
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

const SERVICE_TYPE: &str = "_osdl._tcp.local.";

impl MdnsAdvertiser {
    /// Start advertising the MQTT broker on the given port.
    ///
    /// The service is visible as `_osdl._tcp.local.` on the local network.
    /// ESP32 nodes query this to discover the mother node's IP.
    pub fn start(mqtt: SocketAddr) -> Result<Self, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS daemon: {}", e))?;
        if !mqtt.ip().is_unspecified() {
            daemon
                .disable_interface(IfKind::All)
                .map_err(|e| e.to_string())?;
            daemon
                .enable_interface(IfKind::Addr(mqtt.ip()))
                .map_err(|e| e.to_string())?;
        }

        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| {
                #[cfg(unix)]
                {
                    // gethostname via nix-style: read /etc/hostname or use libc
                    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string())
                }
                #[cfg(not(unix))]
                {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "no hostname",
                    ))
                }
            })
            .unwrap_or_else(|_| "osdl-mother".into());

        Self::register(daemon, &hostname, mqtt)
    }

    fn register(daemon: ServiceDaemon, hostname: &str, mqtt: SocketAddr) -> Result<Self, String> {
        let instance_name = format!("OpenSDL on {}", hostname);

        let host_label = hostname.trim_end_matches(".local").trim_end_matches('.');
        let fqdn = format!("{}.local.", host_label);

        let automatic = mqtt.ip().is_unspecified();
        let addresses = if automatic {
            String::new()
        } else {
            mqtt.ip().to_string()
        };
        let mut service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &fqdn,
            addresses.as_str(),
            mqtt.port(),
            None,
        )
        .map_err(|e| format!("mDNS service info: {}", e))?;
        if automatic {
            service = service.enable_addr_auto();
        }

        let fullname = service.get_fullname().to_string();

        daemon
            .register(service)
            .map_err(|e| format!("mDNS register: {}", e))?;

        log::info!(
            "mDNS advertising: {} → port {} (service: {})",
            instance_name,
            mqtt.port(),
            SERVICE_TYPE
        );

        Ok(MdnsAdvertiser { daemon, fullname })
    }
}

impl Drop for MdnsAdvertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mdns_sd::{IfKind, ServiceEvent};

    use super::*;

    fn loopback_daemon() -> ServiceDaemon {
        let daemon = ServiceDaemon::new().unwrap();
        daemon.disable_interface(IfKind::All).unwrap();
        daemon.enable_interface(IfKind::LoopbackV4).unwrap();
        daemon
    }

    #[test]
    fn test_mdns_advertise_and_discover() {
        let browser = loopback_daemon();
        let receiver = browser.browse(SERVICE_TYPE).unwrap();

        let port = 18830;
        let hostname = format!("osdl-mdns-test-{}", std::process::id());
        let advertisement =
            MdnsAdvertiser::register(loopback_daemon(), &hostname, ([127, 0, 0, 1], port).into())
                .unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut discovered = false;
        while let Ok(event) = receiver.recv_deadline(deadline) {
            if let ServiceEvent::ServiceResolved(info) = event {
                if info.get_fullname() == advertisement.fullname {
                    assert_eq!(info.get_port(), port);
                    assert!(info.get_addresses().iter().any(|ip| ip.is_loopback()));
                    discovered = true;
                    break;
                }
            }
        }
        browser.shutdown().unwrap();
        assert!(
            discovered,
            "mDNS service was not discovered on IPv4 loopback"
        );
    }
}
