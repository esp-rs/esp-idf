//! This example demonstrates how to configure an RMII based Ethernet adapter
//!
//! To use it, you need an RMII-capable Espressif MCU, like the original ESP32 chip
//!
//! Note: On ESP-IDF v6.0+, specific RMII PHY drivers were moved out of the main tree.
//! This example uses `RmiiEthChipset::Generic` which is always available. To use a
//! specific PHY, add the corresponding component to your `Cargo.toml`, e.g.:
//! ```toml
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "espressif/lan87xx", version = "1.*" }
//! ```
//! Other available PHY components: `espressif/ip101`, `espressif/dp83848`,
//! `espressif/rtl8201`, `espressif/ksz80xx`.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(esp32)]
use esp_idf_svc::{
    eth::{BlockingEth, EspEth, EthDriver},
    eventloop::EspSystemEventLoop,
    hal::peripherals::Peripherals,
    log::EspLogger,
};
#[cfg(esp32)]
use log::info;

#[cfg(esp32)]
fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;
    let sys_loop = EspSystemEventLoop::take()?;

    // Make sure to configure ethernet in sdkconfig and adjust the parameters below for your hardware
    let eth_driver = EthDriver::new_rmii(
        peripherals.mac,
        pins.gpio25,
        pins.gpio26,
        pins.gpio27,
        pins.gpio23,
        pins.gpio22,
        pins.gpio21,
        pins.gpio19,
        pins.gpio18,
        esp_idf_svc::eth::RmiiClockConfig::OutputInvertedGpio17(pins.gpio17),
        Some(pins.gpio5),
        // Replace with a specific PHY variant (e.g. LAN87XX, IP101) if you have it available.
        // On ESP-IDF v6.0+ these require external components (see note at top of file).
        // Generic is available since ESP-IDF v5.4; use LAN87XX for older versions.
        #[cfg(esp_idf_version_at_least_5_4_0)]
        esp_idf_svc::eth::RmiiEthChipset::Generic,
        #[cfg(not(esp_idf_version_at_least_5_4_0))]
        esp_idf_svc::eth::RmiiEthChipset::LAN87XX,
        Some(0),
        sys_loop.clone(),
    )?;
    let eth = EspEth::wrap(eth_driver)?;

    info!("Eth created");

    let mut eth = BlockingEth::wrap(eth, sys_loop.clone())?;

    info!("Starting eth...");

    eth.start()?;

    info!("Waiting for DHCP lease...");

    eth.wait_netif_up()?;

    let ip_info = eth.eth().netif().get_ip_info()?;

    info!("Eth DHCP info: {ip_info:?}");

    Ok(())
}

#[cfg(not(esp32))]
fn main() {
    use esp_idf_svc::{self as _};

    panic!("This example is configured for esp32, please adjust pins to your module");
}
