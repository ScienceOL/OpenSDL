//! ESP-NOW diagnostic — same binary on BOTH boards.
//!
//! Each board:
//!   - broadcasts a ping to FF:FF:FF:FF:FF:FF every 1s with its own MAC in payload
//!   - listens for anything and logs every rx (including its own broadcast echoing? no, ESP-NOW drops self-broadcasts)
//!
//! If either board sees RX from the other, the radio link works.
//! If neither sees anything → channel or PHY issue.

use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::espnow::{EspNow, PeerInfo};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{esp_mac_type_t_ESP_MAC_WIFI_STA, esp_read_mac};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};

const BROADCAST: [u8; 6] = [0xFF; 6];

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("[diag] booting");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start()?;

    unsafe {
        let r = esp_idf_svc::sys::esp_wifi_set_channel(
            1,
            esp_idf_svc::sys::wifi_second_chan_t_WIFI_SECOND_CHAN_NONE,
        );
        log::info!("[diag] set_channel(1) rc={}", r);

        let mut pri: u8 = 0;
        let mut sec = esp_idf_svc::sys::wifi_second_chan_t_WIFI_SECOND_CHAN_NONE;
        esp_idf_svc::sys::esp_wifi_get_channel(&mut pri, &mut sec);
        log::info!("[diag] get_channel -> {}", pri);
    }

    let mut my_mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp!(esp_read_mac(my_mac.as_mut_ptr(), esp_mac_type_t_ESP_MAC_WIFI_STA))?;
    }
    log::info!("[diag] my MAC = {:02X?}", my_mac);

    let espnow = EspNow::take()?;

    // Add broadcast peer
    let mut peer = PeerInfo::default();
    peer.peer_addr = BROADCAST;
    peer.channel = 1;
    peer.encrypt = false;
    let rc = espnow.add_peer(peer);
    log::info!("[diag] add_peer(broadcast) = {:?}", rc);

    espnow.register_recv_cb(|info: &esp_idf_svc::espnow::ReceiveInfo, data: &[u8]| {
        log::info!(
            "[RX] from {:02X?} len={} data={:02X?}",
            info.src_addr, data.len(), data
        );
    })?;

    let mut counter: u32 = 0;
    loop {
        let mut payload = [0u8; 10];
        payload[0..6].copy_from_slice(&my_mac);
        payload[6..10].copy_from_slice(&counter.to_le_bytes());

        match espnow.send(BROADCAST, &payload) {
            Ok(()) => log::info!("[TX] bcast counter={}", counter),
            Err(e) => log::warn!("[TX] failed: {:?}", e),
        }
        counter = counter.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}
