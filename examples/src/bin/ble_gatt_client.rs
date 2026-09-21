//! Example of a BLE GATT client (central) using the ESP IDF NimBLE bindings.
//!
//! Requires a NimBLE build with the GATT client (`CONFIG_BT_NIMBLE_GATT_CLIENT=y`).
//!
//! It connects to a fixed peer address (set `PEER` to your device), discovers its services, and
//! logs discovery results and any received notifications/indications. Chaining discovery →
//! characteristic discovery → read/write is left as an extension: do it from the main loop the same
//! way `discover_services` is driven here (the `GattcEvent`s carry the handles you need).

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(not(any(esp32s2, esp32p4)), esp_idf_bt_nimble_gatt_client))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(all(not(any(esp32s2, esp32p4)), esp_idf_bt_nimble_gatt_client)))]
fn main() -> anyhow::Result<()> {
    panic!("This example requires a NimBLE GATT-client build (CONFIG_BT_NIMBLE_GATT_CLIENT=y) on a chip with a BLE radio");
}

#[cfg(all(not(any(esp32s2, esp32p4)), esp_idf_bt_nimble_gatt_client))]
mod example {
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use esp_idf_svc::ble::gap::GapEvent;
    use esp_idf_svc::ble::gatt::client::GattcEvent;
    use esp_idf_svc::ble::{ensure_addr, BleAddr, BleDriver, ConnHandle, HostEvent};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;

    use log::{info, warn};

    // The peer to connect to. Set this to your server's address; the bytes are in NimBLE order
    // (least-significant first). `0` = BLE_ADDR_PUBLIC.
    const PEER: BleAddr = BleAddr::new(0, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);

    static CONN: Mutex<Option<ConnHandle>> = Mutex::new(None);
    // The GAP / sync callbacks only flip these flags; `main` owns the driver and issues the
    // connect / discovery in response (so the callbacks stay `'static`, no `unsafe`).
    static NEEDS_CONNECT: AtomicBool = AtomicBool::new(false);
    static NEEDS_DISCOVER: AtomicBool = AtomicBool::new(false);

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;

        // A central has no service table, so `S = ()`.
        let driver = BleDriver::new(peripherals.modem)?;

        // Connect once the stack is in sync (re-armed on reset).
        driver.host_subscribe(|event| {
            if let HostEvent::Sync = event {
                NEEDS_CONNECT.store(true, Ordering::Relaxed);
            }
        });

        driver.gap_subscribe(|event| {
            match event {
                GapEvent::Connect {
                    conn_handle,
                    status,
                } => match status {
                    Ok(()) => {
                        info!("connected (handle {conn_handle})");
                        *CONN.lock().unwrap() = Some(conn_handle);
                        NEEDS_DISCOVER.store(true, Ordering::Relaxed);
                    }
                    Err(e) => {
                        warn!("connection failed: {e}");
                        NEEDS_CONNECT.store(true, Ordering::Relaxed);
                    }
                },
                GapEvent::Disconnect { reason, .. } => {
                    info!("disconnected ({reason}); reconnecting");
                    *CONN.lock().unwrap() = None;
                    NEEDS_CONNECT.store(true, Ordering::Relaxed);
                }
                GapEvent::Mtu { value, .. } => info!("MTU negotiated: {value}"),
                _ => {}
            }

            0
        });

        // One hook for all client completions plus received notifications/indications.
        driver.gattc_subscribe(|event| match event {
            GattcEvent::Service {
                status, service, ..
            } => match service {
                Some(s) => info!(
                    "service {:?}: handles {}..={}",
                    s.uuid, s.start_handle, s.end_handle
                ),
                None => info!("service discovery complete (status {status})"),
            },
            GattcEvent::Characteristic { status, chr, .. } => match chr {
                Some(c) => info!(
                    "  characteristic {:?}: val_handle {}, props {:#04x}",
                    c.uuid, c.val_handle, c.properties
                ),
                None => info!("  characteristic discovery complete (status {status})"),
            },
            GattcEvent::ReadComplete {
                status,
                attr_handle,
                data,
                ..
            } => {
                let mut buf = [0u8; 64];
                let n = data.read(&mut buf).unwrap_or(0);
                info!(
                    "read of handle {attr_handle} (status {status}): {:?}",
                    &buf[..n]
                );
            }
            GattcEvent::WriteComplete {
                status,
                attr_handle,
                ..
            } => info!("write of handle {attr_handle} complete (status {status})"),
            GattcEvent::Notify {
                attr_handle,
                indication,
                data,
                ..
            } => {
                let mut buf = [0u8; 64];
                let n = data.read(&mut buf).unwrap_or(0);
                info!(
                    "{} on handle {attr_handle}: {:?}",
                    if indication {
                        "indication"
                    } else {
                        "notification"
                    },
                    &buf[..n]
                );
            }
        });

        driver.start()?;
        info!("NimBLE host started; will connect to {PEER}");

        loop {
            FreeRtos::delay_ms(1000);

            if NEEDS_CONNECT.swap(false, Ordering::Relaxed) {
                ensure_addr(false)?;
                match driver.connect(0 /* BLE_OWN_ADDR_PUBLIC */, &PEER) {
                    Ok(()) => info!("connecting to {PEER}..."),
                    Err(e) => {
                        warn!("failed to start connecting: {e}");
                        NEEDS_CONNECT.store(true, Ordering::Relaxed);
                    }
                }
            }

            if NEEDS_DISCOVER.swap(false, Ordering::Relaxed) {
                if let Some(conn) = *CONN.lock().unwrap() {
                    if let Err(e) = driver.discover_services(conn) {
                        warn!("failed to start service discovery: {e}");
                    }
                }
            }
        }
    }
}
