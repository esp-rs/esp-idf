//! Safe interaction with the NimBLE os_mbuf buffer system

use core::ffi::{c_int, c_void};
use core::marker::PhantomData;

use crate::sys::*;

use super::BleError;

// On chips whose BLE controller lives in ROM (`SOC_ESP_NIMBLE_CONTROLLER`) they are the controller's
// `r_<name>` symbols: either `#define`d to them (c2/c5/c6/c61/h2) or, with the dual-mode controller
// architecture (`BT_DUAL_MODE_ARCH`, the esp32s31), wrapped by `static inline` functions that bindgen
// does not emit at all. The latter take a `struct ble_mbuf`, which has the same layout as
// `struct os_mbuf` (as `static_assert`ed in `os_mbuf.h`). On the other chips they are ordinary
// functions.
extern "C" {
    #[cfg_attr(
        all(esp_idf_soc_esp_nimble_controller, esp_idf_bt_controller_enabled),
        link_name = "r_os_mbuf_append"
    )]
    fn os_mbuf_append(m: *mut os_mbuf, data: *const c_void, len: u16) -> c_int;
}

/// View of an os_mbuf, the data buffers used by NimBLE
pub struct Mbuf<'a> {
    om: *mut os_mbuf,
    _p: PhantomData<&'a mut os_mbuf>,
}

impl Mbuf<'_> {
    pub(crate) fn from_raw(om: *mut os_mbuf) -> Self {
        Self {
            om,
            _p: PhantomData,
        }
    }

    /// Copy this Mbuf into `buf`, returning the number of bytes copied or error if buf is too small
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, BleError> {
        // A completion callback delivered with an error status may carry a null mbuf.
        if self.om.is_null() {
            return Ok(0);
        }

        let mut copied: u16 = 0;

        BleError::from_raw(unsafe {
            ble_hs_mbuf_to_flat(
                self.om,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u16,
                &mut copied,
            )
        })?;

        Ok(copied as usize)
    }

    /// Append `buf` to the mbuf.
    pub fn append(&mut self, buf: &[u8]) -> Result<(), BleError> {
        BleError::from_raw(unsafe {
            os_mbuf_append(self.om, buf.as_ptr() as *const c_void, buf.len() as u16)
        })
    }
}

/// Allocate an `os_mbuf` and copy `buf` into it. Errors with `BLE_HS_ENOMEM` if
/// allocation fails.
#[cfg(esp_idf_bt_nimble_gatt_server)]
pub(crate) fn mbuf_from_slice(buf: &[u8]) -> Result<*mut os_mbuf, BleError> {
    let om = unsafe { ble_hs_mbuf_from_flat(buf.as_ptr() as *const c_void, buf.len() as u16) };

    if om.is_null() {
        Err(BleError::new(BLE_HS_ENOMEM as c_int))
    } else {
        Ok(om)
    }
}
