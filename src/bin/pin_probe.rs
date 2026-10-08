//! Relay pin probe: finds which GPIO drives each relay on a board whose pinout isn't documented.
//!
//! Cycles through every GPIO that's safe to drive on an ESP32-WROOM-32E, one at a time: drives it
//! HIGH, then LOW, then releases it (back to a floating input), logging each phase over serial.
//! Watch/listen for a relay click (and its indicator LED) and match it to the logged phase:
//!   - relay turns on during the HIGH phase => that relay is active-high on that GPIO
//!   - relay turns on during the LOW phase  => that relay is active-low on that GPIO
//!
//! No WiFi/MQTT, no cfg.toml needed. **Disconnect all loads from the relay contacts first.**
//!
//! Build & flash (board in download mode: hold DOWNLOAD, tap RESET):
//!   cargo build --release --bin pin_probe
//!   espflash flash -p <port> target/xtensa-esp32-espidf/release/pin_probe
//! then tap RESET and `espflash monitor -p <port>`.

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{AnyOutputPin, PinDriver};

/// Output-capable GPIOs that are safe to drive after boot on a WROOM-32E. Excluded:
///   0      — wired to the DOWNLOAD button; driving it while pressed shorts the pin
///   1, 3   — UART0 TX/RX, needed for this probe's serial log
///   6..=11 — internal SPI flash; touching them crashes the chip
///   34..=39 — input-only
/// 2, 5, 12, 15 are boot strapping pins — fine to drive once running, and included since relay
/// boards sometimes use them.
const CANDIDATE_GPIOS: &[u8] = &[
    2, 4, 5, 12, 13, 14, 15, 16, 17, 18, 19, 21, 22, 23, 25, 26, 27, 32, 33,
];

const HIGH_MS: u32 = 3000;
const LOW_MS: u32 = 3000;
const RELEASE_MS: u32 = 1500;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("=== Relay pin probe: {} candidate GPIOs ===", CANDIDATE_GPIOS.len());
    log::info!("Note which relay clicks ON, and in which phase (HIGH or LOW), for each GPIO.");

    for round in 1.. {
        log::info!("--- Round {round} ---");
        for &gpio in CANDIDATE_GPIOS {
            // SAFETY: nothing else in this program owns any GPIO, and the driver is dropped
            // (resetting the pin to a floating input) before the next pin is stolen.
            let pin = unsafe { AnyOutputPin::steal(gpio as _) };
            let mut driver = PinDriver::output(pin)?;

            log::info!("GPIO {gpio:>2} -> HIGH");
            driver.set_high()?;
            FreeRtos::delay_ms(HIGH_MS);

            log::info!("GPIO {gpio:>2} -> LOW");
            driver.set_low()?;
            FreeRtos::delay_ms(LOW_MS);

            log::info!("GPIO {gpio:>2} -> released");
            drop(driver);
            FreeRtos::delay_ms(RELEASE_MS);
        }
    }

    Ok(())
}
