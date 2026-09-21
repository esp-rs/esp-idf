//! Basic LCD DSI example for ESP32-P4
//!
//! This example shows how to:
//! - Configure the LCD peripheral for a DSI panel
//! - Create a vendor-specific control panel (ILI9881C over DBI)
//! - Initialize the panel
//! - Create a DPI panel for frame updates
//! - Draw a simple bitmap
//!
//! Requirements:
//! - Target: ESP32-P4 (`ESP_IDF_TARGET=esp32p4`)
//! - ESP-IDF must have the ILI9881C DSI panel component enabled
//! - A compatible 1024×600 MIPI-DSI display connected to the LCD peripheral

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(esp32p4)]
mod example {
    use std::time::Duration;

    use esp_idf_hal::lcd::*;
    use esp_idf_hal::ldo::*;
    use esp_idf_hal::peripherals::Peripherals;

    use esp_idf_sys::*;

    #[allow(unused_variables)]
    #[allow(unused_assignments)]
    #[allow(unused_mut)]
    pub fn run() -> anyhow::Result<()> {
        // Link ESP-IDF patches (required for examples)
        esp_idf_hal::sys::link_patches();

        let peripherals = Peripherals::take()?;

        // Configure LDO3 for MIPI D-PHY supply (typically 2.5 V).
        //
        // NOTE: Adjust the voltage to match your board's design and panel
        // requirements. 2.5 V is a common value for the DSI PHY supply.
        let ldo_cfg = LdoChannelConfig::new(2500);
        let _ldo3 = LdoChannel::new(peripherals.ldo3, &ldo_cfg)?;

        // Give the LDO some time to stabilize before enabling the panel.
        std::thread::sleep(Duration::from_millis(20));

        // Configure video timing for a 1024×600 panel.
        // Adjust these values to match your panel's datasheet.
        let video_timing = VideoTiming::new(1024, 600)
            .hsync_back_porch(100)
            .hsync_pulse_width(100)
            .hsync_front_porch(100)
            .vsync_back_porch(10)
            .vsync_pulse_width(10)
            .vsync_front_porch(10);

        // High-level LCD configuration:
        // - 2 DSI data lanes at 1250 Mbps
        // - RGB888 pixel format
        // - DPI clock at 48 MHz
        let config = LcdConfig::new(
            video_timing,
            2,    // num_data_lanes
            1250, // lane_bit_rate_mbps
            48,   // dpi_clock_freq_mhz
            PixelFormat::Rgb888,
        );

        // Create driver with DSI bus and DBI IO
        let mut lcd = LcdDriver::new(peripherals.dsi, &config)?;

        // Create vendor control panel (e.g., ILI9881C) over DBI.
        //
        // Note: This requires the corresponding ESP-IDF component
        // (e.g., ILI9881C DSI panel driver) to be enabled.
        let mut dev_config: esp_lcd_panel_dev_config_t = lcd.config().into();

        // Configure reset GPIO if your panel uses one (adjust as needed).
        // Use -1 for no reset GPIO.
        dev_config.reset_gpio_num = -1;

        let mut control_panel: PanelHandle = core::ptr::null_mut();
        // Replace `esp_lcd_new_panel_ili9881c` with the appropriate
        // vendor-specific constructor for your panel if needed.
        //
        // Note that you DO need to include the ESP-IDf custom component
        // corresponding to your panel (e.g., ILI9881C DSI panel) in your project for this to work.
        //
        // unsafe {
        //     esp!(esp_lcd_new_panel_ili9881c(
        //         lcd.dbi_io_handle(),
        //         &dev_config,
        //         &mut control_panel
        //     ))?;
        // }
        lcd.set_control_panel(control_panel)?;

        // Initialize the control panel
        lcd.reset()?;
        lcd.init()?;
        lcd.set_display_on(true)?;

        // Create the DPI panel for pixel data transfers.
        // This consumes the NoDpiPanel driver and returns a WithDpiPanel driver.
        let lcd = lcd.create_dpi_panel()?;

        // Simple test bitmap: clear screen to black.
        //
        // NOTE:
        // - draw_bitmap uses half-open intervals [x1, x2) × [y1, y2)
        // - For a 1024×600 display, use x2=1024, y2=600 (NOT 1023, 599)
        // - For RGB888, 3 bytes per pixel
        let bitmap = vec![0u8; 1024 * 600 * 3];

        lcd.draw_bitmap(0, 0, 1024, 600, &bitmap)?;

        // Keep the application alive so the display remains on
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(not(esp32p4))]
mod example {
    pub fn run() -> anyhow::Result<()> {
        println!("This example is not supported on this target");
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    example::run()
}
