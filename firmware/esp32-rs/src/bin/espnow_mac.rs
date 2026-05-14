//! Print MAC in a tight loop so serial capture can catch it anytime.

use std::thread;
use std::time::Duration;

use esp_idf_svc::sys::{esp_read_mac, esp_mac_type_t_ESP_MAC_WIFI_STA};

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp!(esp_read_mac(mac.as_mut_ptr(), esp_mac_type_t_ESP_MAC_WIFI_STA))?;
    }

    loop {
        log::info!(
            "MAC = {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
        thread::sleep(Duration::from_millis(500));
    }
}
