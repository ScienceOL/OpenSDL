//! mDNS service advertisement for automatic child node discovery.
//!
//! The mother node advertises `_osdl._tcp.local` so that ESP32 child nodes
//! can find the MQTT broker IP:port without any hardcoded configuration.
//! This is the same mechanism used by AirPlay, Chromecast, and Home Assistant.

use mdns_sd::{ServiceDaemon, ServiceInfo};

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
    /// ESP32 child nodes query this to discover the mother node's IP.
    pub fn start(mqtt_port: u16) -> Result<Self, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS daemon: {}", e))?;

        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| {
                #[cfg(unix)]
                {
                    // gethostname via nix-style: read /etc/hostname or use libc
                    std::fs::read_to_string("/etc/hostname")
                        .map(|s| s.trim().to_string())
                }
                #[cfg(not(unix))]
                {
                    Err(std::io::Error::new(std::io::ErrorKind::Other, "no hostname"))
                }
            })
            .unwrap_or_else(|_| "osdl-mother".into());

        let instance_name = format!("OpenSDL on {}", hostname);

        let host_label = hostname
            .trim_end_matches(".local")
            .trim_end_matches('.');
        let fqdn = format!("{}.local.", host_label);

        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &fqdn,
            "",  // empty = auto-detect IP
            mqtt_port,
            None,
        )
        .map_err(|e| format!("mDNS service info: {}", e))?
        .enable_addr_auto();

        let fullname = service.get_fullname().to_string();

        daemon
            .register(service)
            .map_err(|e| format!("mDNS register: {}", e))?;

        log::info!(
            "mDNS advertising: {} → port {} (service: {})",
            instance_name,
            mqtt_port,
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
    use super::*;

    #[test]
    fn test_mdns_advertise_and_discover() {
        // Start advertising on a random port
        let port = 18830;
        let _adv = MdnsAdvertiser::start(port).expect("should start mDNS advertiser");

        // Browse for the service
        let daemon = ServiceDaemon::new().unwrap();
        let receiver = daemon.browse(SERVICE_TYPE).unwrap();

        // Wait up to 3 seconds for the service to appear
        let found = loop {
            match receiver.recv_timeout(std::time::Duration::from_secs(3)) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                    if info.get_port() == port {
                        break true;
                    }
                }
                Ok(_) => continue, // other events (SearchStarted, etc.)
                Err(_) => break false,
            }
        };

        let _ = daemon.shutdown();
        assert!(found, "Should discover our own mDNS service within 3s");
    }
}
