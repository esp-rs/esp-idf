//! A simple example demonstrating logging with `EspIdfLogger`
//! Uncomment one of the options and comment all the others to see how it operates

use log::info;

fn main() {
    esp_idf_svc::sys::link_patches();

    //
    // Option 1
    //
    // Easiest option: initialize the `log` crate with a hard-coded log level and with `EspIdfLogger`

    // esp_idf_svc::log::init(::log::LevelFilter::Info);

    //
    // Option 2
    //
    // This is the same as the above, but uses a `RUST_LOG` environment variable to set the log level
    // If this environment variable is not set, it defaults to `Info`

    esp_idf_svc::log::init_from_env();

    //
    // Option 3
    //
    // A third option is to initialize the logger in a way where its log level is controlled
    // by the ESP-IDF log configuration
    //
    // This way both C and Rust logs are controlled by the same configuration in `sdkconfig.defaults`,
    // but this is not always desired, hence the above options

    // esp_idf_svc::log::init_from_esp_idf();

    // This is equivalent to `init_from_esp_idf()` and only kept for backwards compatibility
    // esp_idf_svc::log::EspLogger::initialize_default();

    //
    // Option 4
    //
    // You can also do it all in a custom way

    // static LOGGER: esp_idf_svc::log::EspIdfLogger<()> = esp_idf_svc::log::EspIdfLogger::new(()); // You can pass your own log filter too
    // use esp_idf_svc::log::LogFilterBackend;
    // ::log::set_logger(&LOGGER)
    //     .map(|()| LOGGER.filter().initialize())
    //     .unwrap();
    // ::log::set_max_level(::log::LevelFilter::Debug);

    info!("Hello, world! This is a logging example using `EspIdfLogger`.");
}
