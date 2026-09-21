//! Example of a Thread Border Router.
//!
//! This example only works on MCUs that do have Wifi capabilities, as follows:
//! - On MCUs with both native `Thread` as well as `Wifi` capabilities, the example will run in co-exist mode, on a single MCU;
//! - On MCUs with only `Wifi` capabilities, the example will run in UART mode, so you need to flash the `thread_rcp` example
//!   on a separate MCU which does have native `Thread` capabilities, and connect the two via UART.
//!
//! NOTE NOTE NOTE:
//! To build, you need to put the following in your `sdkconfig.defaults`:
//! ```text
//! CONFIG_OPENTHREAD_ENABLED=y
//!
//! # Thread Border Router
//! CONFIG_OPENTHREAD_BORDER_ROUTER=y
//!
//! # These are also necessary for the Joiner feature
//! CONFIG_MBEDTLS_CMAC_C=y
//! CONFIG_MBEDTLS_SSL_PROTO_DTLS=y
//! CONFIG_MBEDTLS_KEY_EXCHANGE_ECJPAKE=y
//! CONFIG_MBEDTLS_ECJPAKE_C=y
//!
//! # Border Router again, lwIP
//! CONFIG_LWIP_IPV6_NUM_ADDRESSES=12
//! CONFIG_LWIP_NETIF_STATUS_CALLBACK=y
//! CONFIG_LWIP_IPV6_FORWARD=y
//! CONFIG_LWIP_MULTICAST_PING=y
//! CONFIG_LWIP_NETIF_STATUS_CALLBACK=y
//! CONFIG_LWIP_HOOK_IP6_ROUTE_DEFAULT=y
//! CONFIG_LWIP_HOOK_ND6_GET_GW_DEFAULT=y
//! CONFIG_LWIP_HOOK_IP6_INPUT_CUSTOM=y
//! CONFIG_LWIP_HOOK_IP6_SELECT_SRC_ADDR_CUSTOM=y
//! CONFIG_LWIP_IPV6_AUTOCONFIG=y
//! CONFIG_LWIP_TCPIP_TASK_STACK_SIZE=4096
//!
//! # Border Router again, mDNS
//! CONFIG_MDNS_MULTIPLE_INSTANCE=y
//!
//! # Border Router again, Wifi/IEEE802.15.4 software co-exist
//! CONFIG_ESP_COEX_SW_COEXIST_ENABLE=y
//! ```
//!
//! And also the following in your `Cargo.toml`:
//! ```toml
//! [[package.metadata.esp-idf-sys.extra_components]]
//! remote_component = { name = "espressif/mdns", version = "1.7" }
//! ```

#![allow(unexpected_cfgs)]

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    #[cfg(any(esp32h2, esp32h4))]
    {
        log::error!("This example only works on MCUs which do have Wifi support.");
    }

    #[cfg(not(any(esp32h2, esp32h4)))]
    {
        // Remove this `cfg` when you have done all of the above for the example to compile
        #[cfg(i_have_done_all_configs_from_the_top_comment)]
        example::main()?;

        // Remove this whole code block when you have done all of the above for the example to compile
        #[cfg(not(i_have_done_all_configs_from_the_top_comment))]
        {
            log::error!("Please follow the instructions in the source code.");
        }
    }

    Ok(())
}

#[cfg(i_have_done_all_configs_from_the_top_comment)] // Remove this `cfg` when you have done all of the above for the example to compile
#[cfg(not(any(esp32h2, esp32h4)))]
mod example {
    use core::convert::TryInto;
    use core::net::Ipv6Addr;

    use std::sync::Arc;

    use log::info;

    use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};

    use esp_idf_svc::eventloop::EspSystemSubscription;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::handle::RawHandle;
    use esp_idf_svc::io::vfs::MountedEventfs;
    use esp_idf_svc::sys::{
        esp, esp_ip6_addr_t, esp_netif_create_ip6_linklocal, esp_netif_get_all_ip6,
        mdns_hostname_set, mdns_init, LWIP_IPV6_NUM_ADDRESSES,
    };
    use esp_idf_svc::thread::{EspThread, ThreadEvent};
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
    use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};

    const SSID: &str = env!("WIFI_SSID");
    const PASSWORD: &str = env!("WIFI_PASS");

    pub fn main() -> anyhow::Result<()> {
        let peripherals = Peripherals::take()?;
        let sys_loop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;

        #[cfg(esp32c6)]
        let (wifi_modem, _thread_modem, _) = { peripherals.modem.split() };

        #[cfg(not(esp32c6))]
        let wifi_modem = peripherals.modem;

        let mounted_event_fs = Arc::new(MountedEventfs::mount(6)?);

        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(wifi_modem, sys_loop.clone(), Some(nvs.clone()))?,
            sys_loop.clone(),
        )?;

        // Need to start the Wifi driver before calling `set_backbone_netif`
        wifi.start()?;

        // Need to set the BR backbone netif before constructing the Thread stack
        unsafe {
            EspThread::set_backbone_netif(Some(wifi.wifi().sta_netif()));
        }

        // On the C6, run the Thread Border Router in co-exist mode
        #[cfg(esp32c6)]
        let mut thread = EspThread::new_br(_thread_modem, sys_loop.clone(), nvs, mounted_event_fs)?;

        // On all other chips, run the Thread Border Router in UART mode
        #[cfg(not(esp32c6))]
        let mut thread = EspThread::new_br_uart(
            peripherals.uart1,
            peripherals.pins.gpio2,
            peripherals.pins.gpio3,
            &esp_idf_svc::thread::config::uart_default_cfg(),
            sys_loop.clone(),
            nvs,
            mounted_event_fs,
            wifi.wifi().sta_netif(),
        )?;

        #[cfg(esp32c6)]
        thread.init_coex()?;

        connect_wifi(&mut wifi)?;

        let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

        info!("Wifi DHCP info: {:?}", ip_info);

        info!("Initializing Thread Border Router...");

        let _subscription = log_thread_sysloop(sys_loop)?;

        thread.set_tod_from_cfg()?;
        thread.enable_ipv6(true)?;
        thread.enable_thread(true)?;

        esp!(unsafe { mdns_init() })?;
        esp!(unsafe { mdns_hostname_set(b"esp-ot-br\0" as *const _ as *const _) })?;

        info!("Thread Border Router initialized, now running...");

        thread.start()?;

        loop {
            let mut addrs: [esp_ip6_addr_t; LWIP_IPV6_NUM_ADDRESSES as _] = Default::default();
            let cnt = unsafe { esp_netif_get_all_ip6(thread.netif().handle(), addrs.as_mut_ptr()) };

            if cnt > 0 {
                log::info!("IPv6 addresses:");

                for i in 0..cnt as usize {
                    let addr = &addrs[i];

                    let ipv6: Ipv6Addr = [
                        addr.addr[0].to_le_bytes()[0],
                        addr.addr[0].to_le_bytes()[1],
                        addr.addr[0].to_le_bytes()[2],
                        addr.addr[0].to_le_bytes()[3],
                        addr.addr[1].to_le_bytes()[0],
                        addr.addr[1].to_le_bytes()[1],
                        addr.addr[1].to_le_bytes()[2],
                        addr.addr[1].to_le_bytes()[3],
                        addr.addr[2].to_le_bytes()[0],
                        addr.addr[2].to_le_bytes()[1],
                        addr.addr[2].to_le_bytes()[2],
                        addr.addr[2].to_le_bytes()[3],
                        addr.addr[3].to_le_bytes()[0],
                        addr.addr[3].to_le_bytes()[1],
                        addr.addr[3].to_le_bytes()[2],
                        addr.addr[3].to_le_bytes()[3],
                    ]
                    .into();

                    log::info!("  - {}", ipv6);
                }
            } else {
                log::info!("No IPv6 addresses");
            }

            // Keep the main thread alive to allow the Thread Border Router to run
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
        let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            bssid: None,
            auth_method: AuthMethod::WPA2Personal,
            password: PASSWORD.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });

        wifi.set_configuration(&wifi_configuration)?;
        info!("Wifi configuration set");

        wifi.connect()?;
        info!("Wifi connected");

        wifi.wait_netif_up()?;
        info!("Wifi netif up");

        // Create an IPv6 link-local address on the backbone netif or else the Border Router won't work
        esp!(unsafe { esp_netif_create_ip6_linklocal(wifi.wifi().sta_netif().handle()) })?;

        Ok(())
    }

    fn log_thread_sysloop(
        sys_loop: EspSystemEventLoop,
    ) -> Result<EspSystemSubscription<'static>, anyhow::Error> {
        let subscription = sys_loop.subscribe::<ThreadEvent, _>(|event| {
            info!("Got: {:?}", event);
        })?;

        Ok(subscription)
    }
}
