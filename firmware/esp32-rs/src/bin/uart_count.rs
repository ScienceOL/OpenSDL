//! Minimal UART/RS-485 TX diagnostic.
//!
//! Configures UART1 on GPIO17 (TX) / GPIO18 (RX) at 9600 8N1 — identical to
//! `espnow_child.rs` — and then prints an incrementing integer once per
//! second, *without* the open-drain GPIO override. On the bus sniffer
//! (USB-RS485 adapter at 9600 baud) you should see "1\n2\n3\n..." appearing.
//!
//! If this shows up on the sniffer → our UART config is fine and the
//! open-drain override is what was killing TX in the ESP-NOW child.
//!
//! If this shows nothing → something more fundamental is wrong with how
//! we're driving UART1 from ESP-IDF-HAL.

use std::thread;
use std::time::Duration;

use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::{config::Config as UartConfig, UartDriver};

const UART_BAUD: u32 = 9600;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    thread::sleep(Duration::from_secs(1));
    log::info!("[uart-count] boot");

    let peripherals = Peripherals::take()?;

    let uart = UartDriver::new(
        peripherals.uart1,
        peripherals.pins.gpio17, // TX
        peripherals.pins.gpio18, // RX
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &UartConfig::new().baudrate(esp_idf_svc::hal::units::Hertz(UART_BAUD)),
    )?;

    log::info!(
        "[uart-count] UART1 ready on GPIO17(TX)/GPIO18(RX) @ {} baud (push-pull default)",
        UART_BAUD
    );

    let mut counter: u32 = 0;
    loop {
        counter = counter.wrapping_add(1);
        let line = format!("{}\n", counter);
        match uart.write(line.as_bytes()) {
            Ok(n) => log::info!("[tx] #{} ({} bytes)", counter, n),
            Err(e) => log::warn!("[tx] failed: {:?}", e),
        }
        thread::sleep(Duration::from_secs(1));
    }
}
