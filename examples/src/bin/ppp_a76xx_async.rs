//! A76xx modem over UART with an asynchronous ESP-NETIF PPP bridge.
//!
//! Set `CELLULAR_APN` at build time. Adjust the UART and power pins for your
//! board, and implement its modem power-key sequence in `ModemPower` below.
//! The A76xx dependency tracks DaneSlattery's fork while its PPP changes are
//! being tested before upstreaming.

#![allow(unexpected_cfgs)]

#[cfg(esp_idf_lwip_ppp_support)]
mod example {
    use core::future::pending;
    use core::time::Duration;

    use a76xx::{Error as ModemError, ModemPower, ModemResources};
    use embassy_futures::select::select;
    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::gpio::{self, Output, PinDriver};
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::hal::uart::{AsyncUartDriver, UartConfig};
    use esp_idf_svc::hal::units::Hertz;
    use esp_idf_svc::handle::RawHandle;
    use esp_idf_svc::netif::{
        AsyncEspNetifChannel, EspNetif, IpEvent, NetifStack, PppConfiguration,
    };
    use esp_idf_svc::timer::{EspAsyncTimer, EspTaskTimerService};
    use static_cell::ConstStaticCell;

    const APN: Option<&str> = option_env!("CELLULAR_APN");
    const DIAL_NUMBER: &str = match option_env!("CELLULAR_DIAL") {
        Some(number) => number,
        None => "*99#",
    };

    type Resources = ModemResources<2048, 2048, 512, 512>;
    static MODEM_RESOURCES: ConstStaticCell<Resources> =
        ConstStaticCell::new(ModemResources::new());

    struct BoardPower {
        enable: PinDriver<'static, Output>,
        power_key: PinDriver<'static, Output>,
        timer: EspAsyncTimer,
    }

    impl ModemPower for BoardPower {
        async fn power_on(&mut self) -> Result<(), ModemError> {
            self.enable
                .set_high()
                .map_err(|_| ModemError::PowerOnError)?;
            self.timer
                .after(Duration::from_millis(100))
                .await
                .map_err(|_| ModemError::PowerOnError)?;
            self.power_key
                .set_high()
                .map_err(|_| ModemError::PowerOnError)?;
            self.timer
                .after(Duration::from_millis(100))
                .await
                .map_err(|_| ModemError::PowerOnError)?;
            self.power_key
                .set_low()
                .map_err(|_| ModemError::PowerOnError)?;
            self.timer
                .after(Duration::from_secs(10))
                .await
                .map_err(|_| ModemError::PowerOnError)?;
            Ok(())
        }
    }

    pub fn run() -> anyhow::Result<()> {
        let apn =
            APN.ok_or_else(|| anyhow::anyhow!("set CELLULAR_APN to your SIM provider's APN"))?;
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;
        let system_loop = EspSystemEventLoop::take()?;
        let timer_service = EspTaskTimerService::new()?;
        let power = BoardPower {
            enable: PinDriver::output(peripherals.pins.gpio2)?,
            power_key: PinDriver::output(peripherals.pins.gpio4)?,
            timer: timer_service.timer_async()?,
        };
        let mut uart = AsyncUartDriver::new(
            peripherals.uart1,
            peripherals.pins.gpio17,
            peripherals.pins.gpio18,
            Option::<gpio::Gpio0>::None,
            Option::<gpio::Gpio0>::None,
            &UartConfig::default().baudrate(Hertz(115_200)),
        )?;
        let (mut modem, rx_pump, tx_pump) = a76xx::Modem::new(MODEM_RESOURCES.take(), power);

        let mut bridge =
            AsyncEspNetifChannel::<_, 8>::new(EspNetif::new(NetifStack::Ppp)?, |netif| {
                netif.set_ppp_conf(&PppConfiguration::default())
            })?;
        let handle = bridge.driver().netif().handle() as usize;
        let mut subscription = system_loop.subscribe_async::<IpEvent>()?;
        bridge.driver_mut().start()?;

        esp_idf_svc::hal::task::block_on(async {
            let (uart_tx, uart_rx) = uart.split();
            let modem_task = async {
                if let Err(error) = modem.power_on().await {
                    log::error!("modem power-on failed: {error:?}");
                    pending::<()>().await;
                }
                modem.wait_for_connection().await;

                let mut ppp_io = match modem.connect_ppp_without_pin(apn, DIAL_NUMBER).await {
                    Ok(io) => io,
                    Err(error) => {
                        log::error!("modem PPP negotiation failed: {error:?}");
                        pending::<()>().await;
                        unreachable!()
                    }
                };
                let monitor = async {
                    loop {
                        match subscription.recv().await {
                            Ok(event)
                                if event.is_for_handle(handle as *mut _)
                                    && matches!(event, IpEvent::DhcpIpAssigned(_)) =>
                            {
                                log::info!("PPP has an IPv4 address");
                            }
                            Ok(_) => {}
                            Err(error) => {
                                log::error!("IP event subscription stopped: {error}");
                                break;
                            }
                        }
                    }
                };
                let bridge_task = async {
                    let mut rx_buffer = [0_u8; 1600];
                    if let Err(error) = bridge.run(&mut ppp_io, &mut rx_buffer).await {
                        log::error!("PPP bridge stopped: {error}");
                    }
                };
                select(monitor, bridge_task).await;
            };
            select(
                rx_pump.run(uart_rx),
                select(tx_pump.run(uart_tx), modem_task),
            )
            .await;
        });
        anyhow::bail!("a PPP task stopped")
    }
}

#[cfg(esp_idf_lwip_ppp_support)]
fn main() -> anyhow::Result<()> {
    example::run()
}

#[cfg(not(esp_idf_lwip_ppp_support))]
fn main() {
    panic!("Enable CONFIG_LWIP_PPP_SUPPORT in sdkconfig.defaults");
}
