//! This example demonstrates how to configure an SPI based Ethernet adapter
//!
//! To be able to use this example on ESP-IDF v5.x, you need to set the following in your
//! sdkconfig.defaults file (pick one depending on your chip):
//! CONFIG_ETH_SPI_ETHERNET_DM9051=y
//! CONFIG_ETH_SPI_ETHERNET_W5500=y
//! CONFIG_ETH_SPI_ETHERNET_KSZ8851SNL=y
//! Also adjust the EspEth::wrap call below to match your board's chip.
//!
//! Note: On ESP-IDF v6.0+, SPI Ethernet drivers were moved out of the main tree.
//! Add the relevant component to your `Cargo.toml` depending on your chip:
//! ```toml
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "espressif/w5500", version = "1.*" }   # or dm9051, ksz8851snl
//! ```

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(
    esp32,
    any(
        esp_idf_eth_spi_ethernet_dm9051,
        esp_idf_comp_espressif__dm9051_enabled,
        esp_idf_eth_spi_ethernet_w5500,
        esp_idf_comp_espressif__w5500_enabled,
        esp_idf_eth_spi_ethernet_ksz8851snl,
        esp_idf_comp_espressif__ksz8851snl_enabled
    )
))]
fn main() {
    example::main().unwrap();
}

#[cfg(not(esp32))]
fn main() {
    panic!("This example is configured for esp32, please adjust pins to your module");
}

#[cfg(all(
    esp32,
    not(any(
        esp_idf_eth_spi_ethernet_dm9051,
        esp_idf_comp_espressif__dm9051_enabled,
        esp_idf_eth_spi_ethernet_w5500,
        esp_idf_comp_espressif__w5500_enabled,
        esp_idf_eth_spi_ethernet_ksz8851snl,
        esp_idf_comp_espressif__ksz8851snl_enabled
    ))
))]
fn main() {
    panic!("No SPI Ethernet chipset enabled. See the note at the top of this file.");
}

#[cfg(all(
    esp32,
    any(
        esp_idf_eth_spi_ethernet_dm9051,
        esp_idf_comp_espressif__dm9051_enabled,
        esp_idf_eth_spi_ethernet_w5500,
        esp_idf_comp_espressif__w5500_enabled,
        esp_idf_eth_spi_ethernet_ksz8851snl,
        esp_idf_comp_espressif__ksz8851snl_enabled
    )
))]
pub mod example {
    use esp_idf_svc::eth;
    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::spi;
    use esp_idf_svc::hal::{peripherals::Peripherals, units::FromValueType};
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::sys::EspError;
    use esp_idf_svc::timer::EspTaskTimerService;
    use esp_idf_svc::{ipv4, ping};

    use log::{info, warn};

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let pins = peripherals.pins;
        let sysloop = EspSystemEventLoop::take()?;
        let timer_service = EspTaskTimerService::new()?;

        let mut eth = eth::EspEth::wrap(eth::EthDriver::new_spi(
            spi::SpiDriver::new(
                peripherals.spi2,
                pins.gpio13,
                pins.gpio12,
                Some(pins.gpio26),
                &spi::SpiDriverConfig::new().dma(spi::Dma::Auto(4096)),
            )?,
            pins.gpio27,
            Some(pins.gpio14),
            Some(pins.gpio25),
            // Replace with DM9051 or KSZ8851SNL if you have some of these variants
            eth::SpiEthChipset::W5500,
            20_u32.MHz().into(),
            Some(&[0x02, 0x00, 0x00, 0x12, 0x34, 0x56]),
            None,
            sysloop.clone(),
        )?)?;

        // Wait for the Eth peripheral and network layer 3 to come up - in an async way because we can
        let ip_info = esp_idf_svc::hal::task::block_on(async {
            let mut eth = eth::AsyncEth::wrap(&mut eth, sysloop.clone(), timer_service)?;

            info!("Starting eth...");

            eth.start().await?;

            info!("Waiting for DHCP lease...");

            eth.wait_netif_up().await?;

            let ip_info = eth.eth().netif().get_ip_info()?;

            info!("Eth DHCP info: {ip_info:?}");

            Result::<_, EspError>::Ok(ip_info)
        })?;

        ping(ip_info.subnet.gateway)?;

        Ok(())
    }

    fn ping(ip: ipv4::Ipv4Addr) -> Result<(), EspError> {
        info!("About to do some pings for {ip:?}");

        let ping_summary = ping::EspPing::default().ping(ip, &Default::default())?;
        if ping_summary.transmitted != ping_summary.received {
            warn!("Pinging IP {ip} resulted in timeouts");
        }

        info!("Pinging done");

        Ok(())
    }
}
