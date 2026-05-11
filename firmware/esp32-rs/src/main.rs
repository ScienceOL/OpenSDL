use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::mqtt::client::{
    EspMqttClient, EventPayload, MqttClientConfiguration, QoS,
};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

mod config;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    thread::sleep(Duration::from_secs(3));

    log::info!("=== OpenSDL child node (Rust) — Phase 1b: mDNS + MQTT ===");
    log::info!("node_id={} hardware_id={}", config::NODE_ID, config::HARDWARE_ID);

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;

    wifi_connect(&mut wifi)?;

    // Discover mother: mDNS first, then static fallback.
    let (mqtt_host, mqtt_port) = discover_mother()?;
    log::info!("[discovery] using broker {}:{}", mqtt_host, mqtt_port);

    let topic_register = format!("osdl/nodes/{}/register", config::NODE_ID);
    let topic_heartbeat = format!("osdl/nodes/{}/heartbeat", config::NODE_ID);
    let topic_tx = format!("osdl/serial/{}/tx", config::NODE_ID);
    let topic_rx = format!("osdl/serial/{}/rx", config::NODE_ID);

    let (tx_event, rx_event): (Sender<MqttEvent>, Receiver<MqttEvent>) = mpsc::channel();

    let broker_url = format!("mqtt://{}:{}", mqtt_host, mqtt_port);
    log::info!("[mqtt] connecting to {}", broker_url);

    let mqtt_config = MqttClientConfiguration {
        client_id: Some(config::NODE_ID),
        keep_alive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    };

    let topic_tx_for_cb = topic_tx.clone();
    let mut mqtt = EspMqttClient::new_cb(&broker_url, &mqtt_config, move |event| {
        match event.payload() {
            EventPayload::Connected(_) => {
                log::info!("[mqtt] << Connected");
                let _ = tx_event.send(MqttEvent::Connected);
            }
            EventPayload::Disconnected => {
                log::warn!("[mqtt] << Disconnected");
                let _ = tx_event.send(MqttEvent::Disconnected);
            }
            EventPayload::Received { topic, data, .. } => {
                if let Some(t) = topic {
                    log::info!("[mqtt] << msg topic={} len={}", t, data.len());
                    if t == topic_tx_for_cb {
                        let _ = tx_event.send(MqttEvent::TxBytes(data.to_vec()));
                    }
                }
            }
            EventPayload::Error(e) => log::warn!("[mqtt] << error: {e:?}"),
            EventPayload::Subscribed(id) => log::info!("[mqtt] << subscribed ({id})"),
            EventPayload::Published(id) => log::debug!("[mqtt] << published ({id})"),
            _ => {}
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match rx_event.recv_timeout(Duration::from_secs(1)) {
            Ok(MqttEvent::Connected) => break,
            Ok(_) => {}
            Err(_) if Instant::now() > deadline => {
                return Err(anyhow::anyhow!("MQTT Connected not received in 15s"));
            }
            Err(_) => {}
        }
    }

    mqtt.subscribe(&topic_tx, QoS::AtLeastOnce)?;
    log::info!("[mqtt] >> subscribe {}", topic_tx);

    let reg_payload = format!(
        "{{\"hardware_id\":\"{}\",\"baud_rate\":{}}}",
        config::HARDWARE_ID,
        config::DEVICE_BAUD
    );
    mqtt.enqueue(&topic_register, QoS::AtLeastOnce, true, reg_payload.as_bytes())?;
    log::info!("[mqtt] >> register {} (retained): {}", topic_register, reg_payload);

    let mut last_heartbeat = Instant::now();
    let heartbeat_period = Duration::from_millis(config::HEARTBEAT_INTERVAL_MS);

    loop {
        if last_heartbeat.elapsed() >= heartbeat_period {
            if let Err(e) = mqtt.enqueue(&topic_heartbeat, QoS::AtMostOnce, false, b"1") {
                log::warn!("[mqtt] heartbeat publish failed: {e:?}");
            } else {
                log::info!("[mqtt] >> heartbeat");
            }
            last_heartbeat = Instant::now();
        }

        match rx_event.recv_timeout(Duration::from_millis(200)) {
            Ok(MqttEvent::TxBytes(bytes)) => {
                log::info!(
                    "[uart-stub] would write {} bytes: {:02x?}",
                    bytes.len(),
                    &bytes[..bytes.len().min(32)]
                );
                let _ = mqtt.enqueue(&topic_rx, QoS::AtLeastOnce, false, &bytes);
                log::info!("[mqtt] >> echoed {} bytes to {}", bytes.len(), topic_rx);
            }
            Ok(MqttEvent::Connected) => log::info!("[loop] (re)connected"),
            Ok(MqttEvent::Disconnected) => log::warn!("[loop] broker dropped us"),
            Err(_) => {}
        }
    }
}

#[derive(Debug)]
enum MqttEvent {
    Connected,
    Disconnected,
    TxBytes(Vec<u8>),
}

/// Find the OpenSDL mother node. Returns (host_or_ip_string, port).
///
/// Resolution order:
///   1. Try mDNS `_osdl._tcp.local` (2 attempts, ~10s total).
///   2. If mDNS fails AND `config::MQTT_HOST` is non-empty → use it as fallback.
///   3. If both fail → error.
///
/// Some networks (enterprise WiFi, SIIFC-3F) block mDNS multicast traffic.
/// Keep `MQTT_HOST` set to a known broker IP so the node still boots.
fn discover_mother() -> anyhow::Result<(String, u16)> {
    match try_mdns() {
        Ok(result) => return Ok(result),
        Err(e) => log::warn!("[discovery] mDNS failed: {}", e),
    }

    if !config::MQTT_HOST.is_empty() {
        log::warn!(
            "[discovery] falling back to static MQTT_HOST={}:{}",
            config::MQTT_HOST,
            config::MQTT_PORT
        );
        return Ok((config::MQTT_HOST.to_string(), config::MQTT_PORT));
    }

    Err(anyhow::anyhow!(
        "mDNS failed and no static MQTT_HOST configured"
    ))
}

fn try_mdns() -> anyhow::Result<(String, u16)> {
    log::info!("[discovery] querying mDNS _osdl._tcp.local ...");
    let mdns = EspMdns::take()?;

    for attempt in 1..=2 {
        let mut results = [esp_idf_svc::mdns::QueryResult {
            instance_name: None,
            hostname: None,
            port: 0,
            txt: vec![],
            addr: vec![],
            interface: esp_idf_svc::mdns::Interface::STA,
            ip_protocol: esp_idf_svc::mdns::Protocol::V4,
        }];

        match mdns.query_ptr(
            "_osdl",
            "_tcp",
            Duration::from_millis(config::MDNS_TIMEOUT_MS),
            1,
            &mut results,
        ) {
            Ok(count) if count > 0 => {
                let r = &results[0];
                log::info!(
                    "[discovery] mDNS hit: instance={:?} host={:?} port={} addrs={:?}",
                    r.instance_name, r.hostname, r.port, r.addr
                );
                if let Some(addr) = r.addr.iter().find_map(|a| match a {
                    esp_idf_svc::ipv4::IpAddr::V4(v4) => Some(v4.to_string()),
                    _ => None,
                }) {
                    return Ok((addr, r.port));
                }
                log::warn!("[discovery] mDNS result had no IPv4 address");
            }
            Ok(_) => log::warn!("[discovery] mDNS 0 results (attempt {})", attempt),
            Err(e) => log::warn!("[discovery] mDNS query error (attempt {}): {:?}", attempt, e),
        }
    }

    Err(anyhow::anyhow!("no mDNS response after 2 attempts"))
}

fn wifi_connect(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    let ssid = config::WIFI_SSID.try_into().map_err(|_| anyhow::anyhow!("SSID too long"))?;
    let password = config::WIFI_PASSWORD
        .try_into()
        .map_err(|_| anyhow::anyhow!("password too long"))?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid,
        password,
        auth_method: AuthMethod::None,
        ..Default::default()
    }))?;

    log::info!("[wifi] start");
    wifi.start()?;

    unsafe {
        esp_idf_svc::sys::esp!(esp_idf_svc::sys::esp_wifi_set_ps(
            esp_idf_svc::sys::wifi_ps_type_t_WIFI_PS_NONE
        ))?;
    }

    log::info!("[wifi] connect to {}", config::WIFI_SSID);
    wifi.connect()?;
    wifi.wait_netif_up()?;

    let info = wifi.wifi().sta_netif().get_ip_info()?;
    log::info!("[wifi] up: ip={} gw={}", info.ip, info.subnet.gateway);
    Ok(())
}
