//! A simple example of how to read from the ESP-IDF console

use esp_idf_hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::UartDriver;
use esp_idf_svc::io::vfs::BlockingStdIo;
use esp_idf_svc::sys::EspError;

use std::io::Write;

fn main() -> Result<(), EspError> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // NOTE: This example assumes that the console configuration is not altered by the user
    // by using non-default `CONFIG_ESP_CONSOLE_*` settings in `sdkconfig.defaults`
    //
    // The default configuration is:
    // - UART0
    // - 115200 baud rate
    // - 8N1
    // - RTS/CTS disabled
    // - No flow control
    // - The default TX/RX pins for UART0 (1 & 3 for ESP32, 17 & 16 for ESP32-S2 and so on.)

    let peripherals = Peripherals::take()?;

    let uart_driver = UartDriver::new(
        peripherals.uart0,
        // Change this to the correct UART0 TX pin for your MCU if you are not using esp32
        peripherals.pins.gpio1,
        // Change this to the correct UART0 RX pin for your MCU if you are not using esp32
        peripherals.pins.gpio3,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &Default::default(),
    )?;

    // Keep it around till the end of your program
    // If you drop it, the console will go back to non-blocking UART mode
    let _blocking_io = BlockingStdIo::uart(uart_driver)?;

    loop {
        print!("Enter a message: ");
        std::io::stdout().flush().unwrap();

        let mut buffer = String::new();
        std::io::stdin().read_line(&mut buffer).unwrap();

        println!("You entered: {buffer}");
    }
}
