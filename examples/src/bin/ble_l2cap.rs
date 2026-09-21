//! Example of a BLE L2CAP CoC (connection-oriented channel) **echo server** using the ESP IDF
//! NimBLE bindings.
//!
//! Requires a NimBLE build with L2CAP CoC enabled (`CONFIG_BT_NIMBLE_L2CAP_COC_MAX_NUM > 0`).
//!
//! The device advertises, and listens for CoC connections on a fixed PSM. Once a central connects
//! (over GAP) and opens an L2CAP channel, every SDU it sends is echoed back. This exercises the full
//! server flow — `Accept` -> `recv_ready`, `Received` -> `send` + re-arm, and the credit-based flow
//! control (`TxUnstalled`). Drive it from a central that opens an L2CAP CoC to `PSM` (e.g. NimBLE's
//! `blecent_l2cap_coc`, or a phone app that supports L2CAP channels).

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

#[cfg(all(
    not(any(esp32s2, esp32p4)),
    esp_idf_bt_nimble_enabled,
    not(esp_idf_bt_nimble_l2cap_coc_max_num = "0")
))]
fn main() -> anyhow::Result<()> {
    example::main()
}

#[cfg(not(all(
    not(any(esp32s2, esp32p4)),
    esp_idf_bt_nimble_enabled,
    not(esp_idf_bt_nimble_l2cap_coc_max_num = "0")
)))]
fn main() -> anyhow::Result<()> {
    panic!("This example requires a NimBLE build with L2CAP CoC enabled (CONFIG_BT_NIMBLE_L2CAP_COC_MAX_NUM > 0) on a chip with a BLE radio");
}

#[cfg(all(
    not(any(esp32s2, esp32p4)),
    esp_idf_bt_nimble_enabled,
    not(esp_idf_bt_nimble_l2cap_coc_max_num = "0")
))]
mod example {
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use esp_idf_svc::ble::gap::{BleAdvFields, BleAdvParams, GapEvent};
    use esp_idf_svc::ble::l2cap::{L2capChan, L2capEvent, SendOutcome};
    use esp_idf_svc::ble::{ensure_addr, BleDriver, BleError, HostEvent};
    use esp_idf_svc::hal::delay::FreeRtos;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::log::EspLogger;

    use log::{info, warn};

    const DEVICE_NAME: &str = "esp-l2cap";
    // The PSM our CoC server listens on. Dynamic (SPSM) range is `0x0080..=0x00ff`; both sides must
    // agree on the value.
    const PSM: u16 = 0x0081;
    // The per-SDU MTU we negotiate and (re)arm receive buffers with.
    const MTU: u16 = 512;

    // Deferred L2CAP actions. The L2CAP hook is `'static`, so it cannot borrow the driver; it only
    // enqueues work here (channels are `Send`), and `main` — which owns the driver — performs the
    // actual `recv_ready` / `send`. (Alternatively, `l2cap_subscribe_nonstatic` would allow calling
    // these straight from the hook, at the cost of `unsafe` and the "don't forget the driver" rule.)
    enum Action {
        // Provide the first receive buffer for a freshly accepted channel.
        Arm(L2capChan),
        // Echo these bytes back on the channel, then re-arm its receive buffer.
        Echo(L2capChan, Vec<u8>),
    }

    static ACTIONS: Mutex<Vec<Action>> = Mutex::new(Vec::new());
    static NEEDS_ADV: AtomicBool = AtomicBool::new(false);

    pub fn main() -> anyhow::Result<()> {
        esp_idf_svc::sys::link_patches();
        EspLogger::initialize_default();

        let peripherals = Peripherals::take()?;

        // An L2CAP endpoint needs no GATT service table, so `S = ()`.
        let driver = BleDriver::new(peripherals.modem)?;

        // Advertise once the stack is "in sync"; re-armed on reset.
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
                } => info!("GAP connected (handle {conn_handle}): {status:?}"),
                GapEvent::Disconnect { reason, .. } => {
                    info!("GAP disconnected ({reason}); re-advertising");
                    NEEDS_ADV.store(true, Ordering::Relaxed);
                }
                _ => {}
            }

            0
        });

        // One hook for the whole L2CAP server. It only enqueues work, so it stays `'static`.
        driver.l2cap_subscribe(|event| {
            match event {
                L2capEvent::Accept {
                    conn_handle, chan, ..
                } => {
                    info!("L2CAP accept on conn {conn_handle}; arming receive");
                    ACTIONS.lock().unwrap().push(Action::Arm(chan));
                }
                L2capEvent::Connected {
                    conn_handle,
                    status,
                    ..
                } => info!("L2CAP channel connected on conn {conn_handle} (status {status})"),
                L2capEvent::Received { chan, data, .. } => {
                    // The SDU mbuf is valid only for this call, so copy it out before returning.
                    let mut sdu = vec![0u8; MTU as usize];
                    let n = data.read(&mut sdu).unwrap_or(0);
                    sdu.truncate(n);
                    info!("L2CAP received {n} bytes; echoing");
                    ACTIONS.lock().unwrap().push(Action::Echo(chan, sdu));
                }
                L2capEvent::Disconnected { conn_handle, .. } => {
                    info!("L2CAP channel disconnected on conn {conn_handle}")
                }
                L2capEvent::TxUnstalled { status, .. } => {
                    info!("L2CAP tx unstalled (status {status})")
                }
                L2capEvent::Reconfigured { .. } => {}
            }

            0 // accept status (0 = accept the incoming channel)
        });

        // Start listening for CoC connections. Servers can be created at runtime; before `start` is
        // fine too.
        driver.l2cap_create_server(PSM, MTU)?;

        driver.start()?;
        info!("NimBLE host started; L2CAP echo server on PSM {PSM:#06x}");

        loop {
            FreeRtos::delay_ms(100);

            if NEEDS_ADV.swap(false, Ordering::Relaxed) {
                match start_advertising(&driver) {
                    Ok(()) => info!("advertising as {DEVICE_NAME:?}"),
                    Err(e) => warn!("failed to start advertising: {e}"),
                }
            }

            // Drain the deferred L2CAP work enqueued by the hook.
            let actions = core::mem::take(&mut *ACTIONS.lock().unwrap());
            for action in actions {
                match action {
                    Action::Arm(chan) => {
                        if let Err(e) = driver.l2cap_recv_ready(chan, MTU) {
                            warn!("recv_ready failed: {e}");
                        }
                    }
                    Action::Echo(chan, data) => {
                        match driver.l2cap_send(chan, &data) {
                            Ok(SendOutcome::Sent) => {}
                            Ok(SendOutcome::Stalled) => {
                                info!("echo stalled; resumes on TxUnstalled")
                            }
                            Err(e) => warn!("echo send failed: {e}"),
                        }
                        // Replenish the peer's credits so it can send the next SDU.
                        if let Err(e) = driver.l2cap_recv_ready(chan, MTU) {
                            warn!("recv_ready failed: {e}");
                        }
                    }
                }
            }
        }
    }

    /// Configure and start a connectable legacy advertisement (the L2CAP channel rides on the GAP
    /// connection this brings up).
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
