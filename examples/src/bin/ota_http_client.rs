//! OTA update example.
//!
//! In this example, we are checking for updates from a distant server hosting the latest firmware.
//! The hosting server is supposed to check if an update should be done or not by responding with
//! a 200 (OK, you should update) or 304 (Not Modified, you are up to date) status code.
//!
//! For this example to work, you need a OTA ready partition table.
//! You need at least 2 OTA partitions (see https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/kconfig.html#config-partition-table-type)
//!
//! The most common starting point for OTA is the "Factory app, two OTA definitions" layout (https://github.com/espressif/esp-idf/blob/master/components/partition_table/partitions_two_ota.csv).
//!
//! To use a custom partition table, download the CSV file of your choice (or create a custom one)
//! and use either use the `--partition-table` option of `espflash`, or set this option in your `espflash.toml` file.
//!
//! After a successful OTA update, you will need to reset the `otadata` partition. Otherwise, the ESP
//! will continue to boot on the second partition, while `cargo run` is flashing the first one by default.
//! To reset the `otadata` partition, add `--erase-parts otadata` to the runner command in `.cargo/config.toml`.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(any(esp32h2, esp32h4, esp32p4))]
fn main() -> anyhow::Result<()> {
    panic!("ESP32-H2, ESP32-H4 and ESP32-P4 do not have a Wifi radio (but you could enable the esp-wifi-remote component to use them with a WiFi co-processor)");
}

#[cfg(not(any(esp32h2, esp32h4, esp32p4)))]
mod example {
    use anyhow::{anyhow, Context};
    use embedded_svc::http::client::Client as HttpClient;
    use esp_idf_svc::http::client::{EspHttpConnection, Method, Response};
    use esp_idf_svc::io;
    use esp_idf_svc::log::EspLogger;
    use esp_idf_svc::ota::{EspFirmwareInfoLoad, EspOtaUpdate, FirmwareInfo};
    use esp_idf_svc::wifi::{BlockingWifi, ClientConfiguration, Configuration, EspWifi};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
    use esp_idf_svc::{
        hal::peripherals::Peripherals,
        ota::{EspOta, SlotState},
    };

    use esp_idf_sys::esp_app_desc;
    use log::{error, info};

    const OTA_FIRMWARE_URI: &str = "http://your.domain/path/to/firmware";

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    // Add package metadata from Cargo.toml. that will be used to create the App Image and used for OTA.
    esp_app_desc!();

    mod http_status {
        pub const OK: u16 = 200;
        pub const NOT_MODIFIED: u16 = 304;
    }

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        // Setup Wifi

        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
            sys_loop,
        )?;

        connect_wifi(&mut wifi)?;

        let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);

        // Check for available updates. This function reboots the ESP once update is complete.
        check_for_updates(&mut client)?;

        // Once an OTA update happened, you have the opportunity to validate that the new firmware is
        // working as expected, and rollback if it's not the case.
        //
        // By default, a firmware will continue to boot until it is marked as invalid.
        // You can change this behavior by setting the `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` option.
        // When enabled, if a reset happen before the firmware have been marked valid, the bootloader
        // will automatically rollback to the previous valid firmware. This is recommended to limit the
        // risk of loosing access the device and requiring a manual flash to fix it.
        //
        // Note: to enable this feature, you need to also flash the bootloader when flashing your image.
        //       Add `--bootloader ./target/<your arch target>/debug/bootloader.bin` to your runner
        //       command in `.cargo/config.toml`
        check_firmware_is_valid()?;

        Ok(())
    }

    fn check_firmware_is_valid() -> anyhow::Result<()> {
        let mut ota = EspOta::new()?;
        let running_slot = ota.get_running_slot()?;

        // Factory Reset slots should not be marked. Trying to mark it will not crash but will display
        // some errors in the logs.
        if running_slot.state == SlotState::Factory {
            info!("Factory slot can't be marked");
            return Ok(());
        }

        if running_slot.state != SlotState::Valid {
            let is_app_valid = true;

            // Do the necessary checks to validate that your app is working as expected.
            // For example, you can contact your API to verify you still have access to it.

            if is_app_valid {
                ota.mark_running_slot_valid()?;
            } else {
                ota.mark_running_slot_invalid_and_reboot();
            }
        }

        Ok(())
    }

    pub fn check_for_updates(client: &mut HttpClient<EspHttpConnection>) -> anyhow::Result<()> {
        let mut ota = EspOta::new().context("failed to obtain OTA instance")?;

        let current_version = get_running_version(&ota)?;
        info!("Current version: {current_version}");

        info!("Checking for updates...");

        let headers = [
            ("Accept", "application/octet-stream"),
            ("X-Esp32-Version", &current_version),
        ];
        let request = client
            .request(Method::Get, OTA_FIRMWARE_URI, &headers)
            .context("failed to create update request")?;
        let response = request.submit().context("failed to send update request")?;

        if response.status() == http_status::NOT_MODIFIED {
            info!("Already up to date");
        } else if response.status() == http_status::OK {
            info!("An update is available, updating...");
            let mut update = ota.initiate_update().context("failed to initiate update")?;

            match download_update(response, &mut update).context("failed to download update") {
                Ok(_) => {
                    info!("Update done. Restarting...");
                    update.complete().context("failed to complete update")?;
                    esp_idf_svc::hal::reset::restart();
                }
                Err(err) => {
                    error!("Update failed: {err}");
                    update.abort().context("failed to abort update")?;
                }
            };
        }

        Ok(())
    }

    fn download_update(
        mut response: Response<&mut EspHttpConnection>,
        update: &mut EspOtaUpdate<'_>,
    ) -> anyhow::Result<()> {
        let mut buffer = [0_u8; 1024];

        // You can optionally read the firmware metadata header.
        // It contains information like version and signature you can check before continuing the update
        let update_info = read_firmware_info(&mut buffer, &mut response, update)?;
        info!("Update version: {}", update_info.version);

        io::utils::copy(response, update, &mut buffer)?;

        Ok(())
    }

    fn read_firmware_info(
        buffer: &mut [u8],
        response: &mut Response<&mut EspHttpConnection>,
        update: &mut EspOtaUpdate,
    ) -> anyhow::Result<FirmwareInfo> {
        let update_info_load = EspFirmwareInfoLoad {};
        let mut update_info = FirmwareInfo {
            version: Default::default(),
            released: Default::default(),
            description: Default::default(),
            signature: Default::default(),
            download_id: Default::default(),
        };

        loop {
            let n = response.read(buffer)?;
            update.write(&buffer[0..n])?;
            if update_info_load.fetch(&buffer[0..n], &mut update_info)? {
                return Ok(update_info);
            }
        }
    }

    fn get_running_version(ota: &EspOta) -> anyhow::Result<heapless::String<24>> {
        Ok(ota
            .get_running_slot()?
            .firmware
            .ok_or(anyhow!("missing firmware info for running slot"))?
            .version)
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            password: PASSWORD.try_into().unwrap(),
            ..Default::default()
        });

        wifi.set_configuration(&wifi_configuration)?;

        wifi.start()?;
        info!("Wifi started");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        Ok(())
    }
}
