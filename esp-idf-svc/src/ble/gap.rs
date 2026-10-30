//! NimBLE GAP: device name, connection events, and advertising.

use core::ffi::c_int;
use core::ptr;

use alloc::ffi::CString;

use crate::sys::*;

use super::{BleAddr, BleDriver, BleError, ConnHandle};

// Advertising parameter and field types.
//
// NimBLE exposes two mutually-exclusive advertising APIs. The legacy API is the default; the
// extended API (the `ext_adv_*` methods) is only compiled when the controller is built with
// `CONFIG_BT_NIMBLE_EXT_ADV=y`.

/// Parameters for a legacy advertising procedure (safe version of `ble_gap_adv_params`).
#[cfg(not(esp_idf_bt_nimble_ext_adv))]
#[derive(Clone, Copy, Default)]
pub struct BleAdvParams {
    pub conn_mode: u8,
    pub disc_mode: u8,
    pub itvl_min: u16,
    pub itvl_max: u16,
    pub channel_map: u8,
    pub filter_policy: u8,
    pub high_duty_cycle: bool,
}

#[cfg(not(esp_idf_bt_nimble_ext_adv))]
impl From<&BleAdvParams> for ble_gap_adv_params {
    fn from(params: &BleAdvParams) -> Self {
        let mut raw: ble_gap_adv_params = unsafe { core::mem::zeroed() };

        raw.conn_mode = params.conn_mode;
        raw.disc_mode = params.disc_mode;
        raw.itvl_min = params.itvl_min;
        raw.itvl_max = params.itvl_max;
        raw.channel_map = params.channel_map;
        raw.filter_policy = params.filter_policy;
        raw.set_high_duty_cycle(params.high_duty_cycle as _);

        raw
    }
}

/// Parameters for an extended advertising instance (safe version of `ble_gap_ext_adv_params`).
///
/// Only available when the controller is built with `CONFIG_BT_NIMBLE_EXT_ADV=y`;
/// the default build exposes the legacy [`BleAdvParams`].
#[cfg(esp_idf_bt_nimble_ext_adv)]
#[derive(Clone, Copy, Default)]
pub struct BleExtAdvParams {
    pub connectable: bool,
    pub scannable: bool,
    pub legacy_pdu: bool,
    pub directed: bool,
    pub anonymous: bool,
    pub high_duty_directed: bool,
    pub include_tx_power: bool,
    pub itvl_min: u32,
    pub itvl_max: u32,
    pub channel_map: u8,
    pub own_addr_type: u8,
    pub primary_phy: u8,
    pub secondary_phy: u8,
    pub tx_power: i8,
    pub sid: u8,
}

#[cfg(esp_idf_bt_nimble_ext_adv)]
impl From<&BleExtAdvParams> for ble_gap_ext_adv_params {
    fn from(params: &BleExtAdvParams) -> Self {
        let mut raw: ble_gap_ext_adv_params = unsafe { core::mem::zeroed() };

        raw.set_connectable(params.connectable as _);
        raw.set_scannable(params.scannable as _);
        raw.set_legacy_pdu(params.legacy_pdu as _);
        raw.set_directed(params.directed as _);
        raw.set_anonymous(params.anonymous as _);
        raw.set_high_duty_directed(params.high_duty_directed as _);
        raw.set_include_tx_power(params.include_tx_power as _);

        raw.itvl_min = params.itvl_min;
        raw.itvl_max = params.itvl_max;
        raw.channel_map = params.channel_map;
        raw.own_addr_type = params.own_addr_type;
        raw.primary_phy = params.primary_phy;
        raw.secondary_phy = params.secondary_phy;
        raw.tx_power = params.tx_power;
        raw.sid = params.sid;

        raw
    }
}

/// Structured advertising payload (safe version of `ble_hs_adv_fields`).
///
/// Which raw-payload setter the driver offers depends on whether the build has extended advertising
/// enabled; in this build it is
#[cfg_attr(esp_idf_bt_nimble_ext_adv, doc = "[`BleDriver::ext_adv_set_data`],")]
#[cfg_attr(not(esp_idf_bt_nimble_ext_adv), doc = "[`BleDriver::adv_set_data`],")]
/// which you can use to set the payload bytes yourself.
#[derive(Clone, Copy, Default)]
pub struct BleAdvFields<'a> {
    pub flags: u8,
    pub name: Option<&'a str>,
    pub tx_power_level: Option<i8>,
    pub appearance: Option<u16>,
    pub service_data_uuid16: Option<&'a [u8]>,
    pub manufacturer_data: Option<&'a [u8]>,
}

impl From<&BleAdvFields<'_>> for ble_hs_adv_fields {
    fn from(fields: &BleAdvFields) -> Self {
        let mut raw: ble_hs_adv_fields = unsafe { core::mem::zeroed() };

        raw.flags = fields.flags;

        if let Some(name) = fields.name {
            raw.name = name.as_ptr();
            raw.name_len = name.len() as _;
            raw.set_name_is_complete(1);
        }

        if let Some(tx_power_level) = fields.tx_power_level {
            raw.tx_pwr_lvl = tx_power_level;
            raw.set_tx_pwr_lvl_is_present(1);
        }

        if let Some(appearance) = fields.appearance {
            raw.appearance = appearance;
            raw.set_appearance_is_present(1);
        }

        if let Some(service_data) = fields.service_data_uuid16 {
            raw.svc_data_uuid16 = service_data.as_ptr();
            raw.svc_data_uuid16_len = service_data.len() as _;
        }

        if let Some(manufacturer_data) = fields.manufacturer_data {
            raw.mfg_data = manufacturer_data.as_ptr();
            raw.mfg_data_len = manufacturer_data.len() as _;
        }

        raw
    }
}

/// Role-agnostic connection events. NimBLE multiplexes *role-specific* events (server:
/// `Subscribe`/`NotifyComplete`; client: `Notify`) onto the same connection callback, but those are
/// demuxed to [`GattsEvent`](super::gatt::server::GattsEvent) /
/// [`GattcEvent`](super::gatt::client::GattcEvent) — so they are not part of this enum.
pub enum GapEvent {
    Connect {
        conn_handle: ConnHandle,
        status: Result<(), BleError>,
    },
    Disconnect {
        conn_handle: ConnHandle,
        reason: BleError,
    },
    Mtu {
        conn_handle: ConnHandle,
        value: u16,
    },
    EncChange {
        conn_handle: ConnHandle,
        status: Result<(), BleError>,
    },
    /// A bonded peer asks to pair again. The callback returns `BLE_GAP_REPEAT_PAIRING_RETRY` after
    /// deleting the old bond to let the pairing continue, or `BLE_GAP_REPEAT_PAIRING_IGNORE`.
    RepeatPairing {
        conn_handle: ConnHandle,
    },
    Other,
}

impl From<&ble_gap_event> for GapEvent {
    fn from(event: &ble_gap_event) -> Self {
        let anon = &event.__bindgen_anon_1;

        match event.type_ as u32 {
            BLE_GAP_EVENT_CONNECT => {
                let connect = unsafe { &anon.connect };
                Self::Connect {
                    conn_handle: connect.conn_handle,
                    status: BleError::from_raw(connect.status),
                }
            }
            BLE_GAP_EVENT_DISCONNECT => {
                let disconnect = unsafe { &anon.disconnect };
                Self::Disconnect {
                    conn_handle: disconnect.conn.conn_handle,
                    reason: BleError::new(disconnect.reason),
                }
            }
            BLE_GAP_EVENT_MTU => {
                let mtu = unsafe { &anon.mtu };
                Self::Mtu {
                    conn_handle: mtu.conn_handle,
                    value: mtu.value,
                }
            }
            BLE_GAP_EVENT_ENC_CHANGE => {
                let enc_change = unsafe { &anon.enc_change };
                Self::EncChange {
                    conn_handle: enc_change.conn_handle,
                    status: BleError::from_raw(enc_change.status),
                }
            }
            BLE_GAP_EVENT_REPEAT_PAIRING => {
                let repeat_pairing = unsafe { &anon.repeat_pairing };
                Self::RepeatPairing {
                    conn_handle: repeat_pairing.conn_handle,
                }
            }
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct BleConnDesc(ble_gap_conn_desc);

impl BleConnDesc {
    pub const fn peer_addr(&self) -> BleAddr {
        BleAddr::new(self.0.peer_id_addr.type_, self.0.peer_id_addr.val)
    }
}

/// Look up a connection descriptor by handle.
///
/// A stateless query against the running host, so it is a free function rather than a [`BleDriver`]
/// method — it needs no live handle and is convenient to call from within a GAP event callback.
pub fn conn_find(conn_handle: ConnHandle) -> Result<BleConnDesc, BleError> {
    let mut desc: ble_gap_conn_desc = unsafe { core::mem::zeroed() };
    BleError::from_raw(unsafe { ble_gap_conn_find(conn_handle, &mut desc) })?;

    Ok(BleConnDesc(desc))
}

/// GAP operations on the [`BleDriver`]: advertising, the device name, and GAP event
/// subscription. Available for any role (`S`). `&self`, so callable re-entrantly from within the
/// GAP event callback.
impl<'d, S> BleDriver<'d, S> {
    /// Set the device name exposed via the GAP service.
    pub fn set_device_name(&self, name: &str) -> Result<(), BleError> {
        let name = CString::new(name).map_err(|_| BleError::new(BLE_HS_EINVAL as c_int))?;

        // NimBLE copies the name into its own buffer, so the drop is safe
        BleError::from_raw(unsafe { ble_svc_gap_device_name_set(name.as_ptr()) })
    }

    /// Subscribe to GAP events (connect / disconnect / subscribe / MTU / notify-tx / — for a
    /// client — notify-rx). The callback runs on the NimBLE host task and returns the GAP status
    /// code (`0` on success). The trampoline is wired at [`adv_start`](Self::adv_start) (server) or
    /// at connect time (client), so this only needs to be set before whichever of those you use.
    pub fn gap_subscribe<F>(&self, callback: F)
    where
        F: FnMut(GapEvent) -> i32 + Send + 'static,
    {
        unsafe { self.gap_subscribe_nonstatic(callback) }
    }

    /// # Safety
    ///
    /// The non-`'static` counterpart of [`gap_subscribe`](Self::gap_subscribe): the callback may
    /// borrow data that lives as long as the [`BleDriver`]. It stays registered until the driver is
    /// dropped, which un-subscribes it, so the driver must not be `core::mem::forget`-ten. See
    /// [`BleDriver::host_subscribe_nonstatic`](crate::ble::BleDriver::host_subscribe_nonstatic).
    pub unsafe fn gap_subscribe_nonstatic<F>(&self, callback: F)
    where
        F: FnMut(GapEvent) -> i32 + Send + 'd,
    {
        unsafe { super::SINGLETON.gap.subscribe_nonstatic(callback) };
    }

    /// Stop delivering GAP events to the subscribed callback.
    pub fn gap_unsubscribe(&self) {
        super::SINGLETON.gap.unsubscribe();
    }

    /// Set the raw advertising payload.
    #[cfg(not(esp_idf_bt_nimble_ext_adv))]
    pub fn adv_set_data(&self, data: &[u8]) -> Result<(), BleError> {
        // NimBLE copies the payload into its own buffer, so `data` need not outlive the call.
        BleError::from_raw(unsafe { ble_gap_adv_set_data(data.as_ptr(), data.len() as c_int) })
    }

    /// Encode `fields` into the advertising payload.
    #[cfg(not(esp_idf_bt_nimble_ext_adv))]
    pub fn adv_set_fields(&self, fields: &BleAdvFields) -> Result<(), BleError> {
        let raw: ble_hs_adv_fields = fields.into();

        BleError::from_raw(unsafe { ble_gap_adv_set_fields(&raw) })
    }

    /// Start a legacy advertising procedure. Drive this from an [`host_subscribe`] closure once the host
    /// has synced, and restart it from a [`GapEvent::Disconnect`] handler. Events for the
    /// resulting connection are delivered to the [`gap_subscribe`](Self::gap_subscribe) callback.
    ///
    /// [`host_subscribe`]: crate::ble::BleDriver::host_subscribe
    #[cfg(not(esp_idf_bt_nimble_ext_adv))]
    pub fn adv_start(&self, own_addr_type: u8, params: &BleAdvParams) -> Result<(), BleError> {
        let raw: ble_gap_adv_params = params.into();

        // bindgen does not emit `BLE_HS_FOREVER`, as its C macro expands to `INT32_MAX` rather than
        // to an integer literal. Advertise with no timeout.
        const BLE_HS_FOREVER: c_int = i32::MAX;

        let rc = unsafe {
            ble_gap_adv_start(
                own_addr_type,
                ptr::null(),
                BLE_HS_FOREVER as _,
                &raw,
                Some(super::BleSingleton::gap_event_cb),
                ptr::null_mut(),
            )
        };
        if rc == BLE_HS_EALREADY as c_int {
            return Ok(());
        }

        BleError::from_raw(rc)
    }

    #[cfg(not(esp_idf_bt_nimble_ext_adv))]
    pub fn adv_stop(&self) -> Result<(), BleError> {
        BleError::from_raw(unsafe { ble_gap_adv_stop() })
    }

    #[cfg(esp_idf_bt_nimble_ext_adv)]
    pub fn ext_adv_configure(
        &self,
        instance: u8,
        params: &BleExtAdvParams,
    ) -> Result<i8, BleError> {
        let raw: ble_gap_ext_adv_params = params.into();
        let mut selected_tx_power: i8 = 0;

        BleError::from_raw(unsafe {
            ble_gap_ext_adv_configure(
                instance,
                &raw,
                &mut selected_tx_power,
                Some(super::BleSingleton::gap_event_cb),
                ptr::null_mut(),
            )
        })?;

        Ok(selected_tx_power)
    }

    #[cfg(esp_idf_bt_nimble_ext_adv)]
    pub fn ext_adv_set_addr(&self, instance: u8, addr: &BleAddr) -> Result<(), BleError> {
        BleError::from_raw(unsafe { ble_gap_ext_adv_set_addr(instance, addr.raw()) })
    }

    #[cfg(esp_idf_bt_nimble_ext_adv)]
    pub fn ext_adv_set_data(&self, instance: u8, data: &[u8]) -> Result<(), BleError> {
        let om = super::mbuf::mbuf_from_slice(data)?;

        // `ble_gap_ext_adv_set_data` takes ownership of `om` and frees it on all paths (no leak, no double-free).
        BleError::from_raw(unsafe { ble_gap_ext_adv_set_data(instance, om) })
    }

    /// Encode `fields` into an advertising payload and install it on `instance`.
    #[cfg(esp_idf_bt_nimble_ext_adv)]
    pub fn ext_adv_set_fields(&self, instance: u8, fields: &BleAdvFields) -> Result<(), BleError> {
        let raw: ble_hs_adv_fields = fields.into();

        let mut buf = [0u8; BLE_HS_ADV_MAX_SZ as usize];
        let mut len: u8 = 0;
        BleError::from_raw(unsafe {
            ble_hs_adv_set_fields(&raw, buf.as_mut_ptr(), &mut len, buf.len() as u8)
        })?;

        self.ext_adv_set_data(instance, &buf[..len as usize])
    }

    #[cfg(esp_idf_bt_nimble_ext_adv)]
    pub fn ext_adv_start(&self, instance: u8) -> Result<(), BleError> {
        let rc = unsafe { ble_gap_ext_adv_start(instance, 0, 0) };
        if rc == BLE_HS_EALREADY as c_int {
            return Ok(());
        }

        BleError::from_raw(rc)
    }

    #[cfg(esp_idf_bt_nimble_ext_adv)]
    pub fn ext_adv_stop(&self, instance: u8) -> Result<(), BleError> {
        BleError::from_raw(unsafe { ble_gap_ext_adv_stop(instance) })
    }
}
