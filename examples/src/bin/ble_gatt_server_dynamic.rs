//! Example of a BLE GATT server using the ESP IDF NimBLE bindings, with the service table built
//! **at runtime** (`BleGattServices`, heap-allocated). For the same server built **statically** at
//! compile time (no heap) with the `gatt_services!` macro, see `ble_gatt_server.rs`.
//!
//! Requires a NimBLE-enabled build with the GATT server (`CONFIG_BT_NIMBLE_GATT_SERVER=y`).

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(not(any(esp32s2, esp32p4)), esp_idf_bt_nimble_gatt_server))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(all(not(any(esp32s2, esp32p4)), esp_idf_bt_nimble_gatt_server)))]
fn main() -> anyhow::Result<()> {
    panic!("This example requires a NimBLE GATT-server build (CONFIG_BT_NIMBLE_GATT_SERVER=y) on a chip with a BLE radio");
}

#[cfg(all(not(any(esp32s2, esp32p4)), esp_idf_bt_nimble_gatt_server))]
mod example {
    use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
    use std::sync::Mutex;

    use esp_idf_svc::ble::gap::{BleAdvFields, BleAdvParams, GapEvent};
    use esp_idf_svc::ble::gatt::server::{
        BleGattCharacteristic, BleGattRegister, BleGattService, BleGattServices, GattsEvent,
    };
    use esp_idf_svc::ble::gatt::BleGattCharFlag;
    use esp_idf_svc::ble::{ensure_addr, BleDriver, BleError, BleUuid, ConnHandle, HostEvent};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;

    use enumset::enum_set;
    use log::{info, warn};

    const DEVICE_NAME: &str = "esp-nimble";

    // Our service UUID
    pub const SERVICE_UUID: u128 = 0xad91b201734740479e173bed82d75f9d;

    /// Our "recv" characteristic - i.e. where clients can send data.
    pub const RECV_CHARACTERISTIC_UUID: u128 = 0xb6fccb5087be44f3ae22f85485ea42c4;
    /// Our "indicate" characteristic - i.e. where clients can receive data if they subscribe to it
    pub const IND_CHARACTERISTIC_UUID: u128 = 0x503de214868246c4828fd59144da41be;

    // Server state. We capture each characteristic's value handle from the `Register` events (see
    // `gatts_subscribe` below); a real server tracking many handles would keep a uuid -> handle map.
    static SUBSCRIBERS: Mutex<Vec<ConnHandle>> = Mutex::new(Vec::new());
    static IND_VAL_HANDLE: AtomicU16 = AtomicU16::new(0);
    static RECV_VAL_HANDLE: AtomicU16 = AtomicU16::new(0);
    // The GAP / sync callbacks run on the host task and only flip flags / touch the statics above,
    // so they stay `'static` and use the plain (safe) subscribe forms. `main` owns the driver and
    // does the advertising / indicating in response.
    static NEEDS_ADV: AtomicBool = AtomicBool::new(false);

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;

        let services = BleGattServices::new(vec![BleGattService::new(
            true,
            BleUuid::uuid128(SERVICE_UUID),
            vec![
                // "recv": clients write here; the single `gatts_subscribe` hook logs it.
                BleGattCharacteristic::new(
                    BleUuid::uuid128(RECV_CHARACTERISTIC_UUID),
                    enum_set!(BleGattCharFlag::Write),
                ),
                // "indicate": clients subscribe and get the counter pushed from the loop below.
                // NimBLE adds the CCCD (0x2902) automatically for this flag.
                BleGattCharacteristic::new(
                    BleUuid::uuid128(IND_CHARACTERISTIC_UUID),
                    enum_set!(BleGattCharFlag::Indicate),
                ),
            ],
        )]);

        // Initialize the host as a GATT server: the service table is registered now (its pointer
        // graph is owned by the driver), the host task starts at `start()`.
        let driver = BleDriver::new_with_services(peripherals.modem, services)?;

        // One hook for the whole GATT server: registration (to learn handles) plus every read and
        // write, dispatched by `attr_handle`. Must be set before `start()`.
        driver.gatts_subscribe(|event| {
            match event {
                GattsEvent::Register(BleGattRegister::Characteristic {
                    uuid, val_handle, ..
                }) => {
                    if uuid == BleUuid::uuid128(IND_CHARACTERISTIC_UUID) {
                        IND_VAL_HANDLE.store(val_handle, Ordering::Relaxed);
                    } else if uuid == BleUuid::uuid128(RECV_CHARACTERISTIC_UUID) {
                        RECV_VAL_HANDLE.store(val_handle, Ordering::Relaxed);
                    }
                }
                GattsEvent::Write {
                    attr_handle, data, ..
                } if attr_handle == RECV_VAL_HANDLE.load(Ordering::Relaxed) => {
                    let mut buf = [0u8; 200];
                    match data.read(&mut buf) {
                        Ok(n) => info!("recv {n} bytes: {:?}", &buf[..n]),
                        Err(e) => warn!("recv read failed: {e}"),
                    }
                }
                // Fires on a CCCD write, on connection teardown, and on a bond restore alike, so
                // just mirroring `cur_indicate` into the subscriber list keeps it correct.
                GattsEvent::SubscriptionChanged {
                    conn_handle,
                    attr_handle,
                    cur_indicate,
                    ..
                } if attr_handle == IND_VAL_HANDLE.load(Ordering::Relaxed) => {
                    let mut subs = SUBSCRIBERS.lock().unwrap();
                    subs.retain(|&c| c != conn_handle);
                    if cur_indicate {
                        subs.push(conn_handle);
                    }
                }
                _ => {}
            }

            0 // ATT status (ignored for `Register` / `SubscriptionChanged`)
        });

        // Advertise once the stack is "in sync"; re-armed on reset (so it can fire again).
        driver.host_subscribe(|event| {
            if let HostEvent::Sync = event {
                NEEDS_ADV.store(true, Ordering::Relaxed);
            }
        });

        driver.gap_subscribe(|event| {
            match event {
                GapEvent::Connect {
                    conn_handle,
                    status,
                } => info!("connected (handle {conn_handle}): {status:?}"),
                GapEvent::Disconnect {
                    conn_handle,
                    reason,
                } => {
                    info!("disconnected ({reason}); re-advertising");
                    SUBSCRIBERS.lock().unwrap().retain(|&c| c != conn_handle);
                    NEEDS_ADV.store(true, Ordering::Relaxed);
                }
                _ => {}
            }

            0
        });

        driver.start()?;
        info!("NimBLE host started");

        let mut counter: u16 = 0;
        loop {
            FreeRtos::delay_ms(1000);

            if NEEDS_ADV.swap(false, Ordering::Relaxed) {
                match start_advertising(&driver) {
                    Ok(()) => info!("advertising as {DEVICE_NAME:?}"),
                    Err(e) => warn!("failed to start advertising: {e}"),
                }
            }

            let ind_handle = IND_VAL_HANDLE.load(Ordering::Relaxed);
            if ind_handle == 0 {
                continue;
            }

            counter = counter.wrapping_add(1);

            // Copy the subscriber list out so the lock isn't held across `indicate`.
            let subs = SUBSCRIBERS.lock().unwrap().clone();
            for conn in subs {
                if let Err(e) = driver.indicate(conn, ind_handle, &counter.to_le_bytes()) {
                    warn!("indicate to {conn} failed: {e}");
                }
            }
        }
    }

    /// Configure and start a connectable legacy advertisement.
    fn start_advertising<S>(driver: &BleDriver<'_, S>) -> Result<(), BleError> {
        ensure_addr(false)?;
        driver.set_device_name(DEVICE_NAME)?;

        let fields = BleAdvFields {
            flags: 0x06, // LE General Discoverable, BR/EDR unsupported
            name: Some(DEVICE_NAME),
            ..Default::default()
        };
        driver.adv_set_fields(&fields)?;

        let params = BleAdvParams {
            conn_mode: 2,   // BLE_GAP_CONN_MODE_UND
            disc_mode: 2,   // BLE_GAP_DISC_MODE_GEN
            itvl_min: 0x30, // 30 ms, in 0.625 ms units
            itvl_max: 0x60, // 60 ms
            ..Default::default()
        };
        driver.adv_start(0 /* BLE_OWN_ADDR_PUBLIC */, &params)
    }
}
