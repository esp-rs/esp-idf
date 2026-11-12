//! Type safe abstraction for esp-tls

#[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
use core::convert::TryFrom;
use core::fmt::Debug;

use crate::private::cstr::{c_char, CStr};
#[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
use crate::sys::EspError;

#[cfg(all(
    esp_idf_comp_esp_tls_enabled,
    any(esp_idf_esp_tls_using_mbedtls, esp_idf_esp_tls_using_wolfssl)
))]
pub use self::esptls::*;

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct Psk<'a> {
    pub key: &'a [u8],
    pub hint: &'a str,
}

impl Debug for Psk<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        f.debug_struct("Psk")
            .field("hint", &self.hint)
            .finish_non_exhaustive()
    }
}

/// Helper for holding PSK data for lately initialized TLS connections.
///
/// It could be easily converted from the public `Psk` configuration and holds the `psk_hint_key_t`
/// along with its (string) data as this data typically needs to be around after initializing a TLS
/// client until it has been started.
#[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
pub(crate) struct TlsPsk {
    pub(crate) psk: alloc::boxed::Box<crate::hal::sys::psk_hint_key_t>,
    pub(crate) _cstrs: crate::private::cstr::RawCstrs,
}
/// Dummy for maintaining the same internal interface whether TLS PSK support is enabled or not.
#[cfg(not(all(esp_idf_esp_tls_psk_verification, feature = "alloc")))]
#[allow(dead_code)]
pub(crate) struct TlsPsk {}

#[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
impl Debug for TlsPsk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        f.debug_struct("TlsPsk")
            .field("psk", &self.psk)
            .finish_non_exhaustive()
    }
}

#[cfg(all(esp_idf_esp_tls_psk_verification, feature = "alloc"))]
impl<'a> TryFrom<&'a Psk<'a>> for TlsPsk {
    type Error = EspError;

    fn try_from(conf: &Psk) -> Result<Self, EspError> {
        let mut cstrs = crate::private::cstr::RawCstrs::new();
        let psk = alloc::boxed::Box::new(crate::hal::sys::psk_hint_key_t {
            key: conf.key.as_ptr(),
            key_size: conf.key.len(),
            hint: cstrs.as_ptr(conf.hint)?,
        });

        Ok(TlsPsk { psk, _cstrs: cstrs })
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct X509<'a>(&'a [u8]);

impl<'a> X509<'a> {
    pub fn pem(cstr: &'a CStr) -> Self {
        Self(cstr.to_bytes_with_nul())
    }

    pub const fn pem_until_nul(bytes: &'a [u8]) -> Self {
        // TODO: replace with `CStr::from_bytes_until_nul` when stabilized
        let mut nul_pos = 0;
        while nul_pos < bytes.len() {
            if bytes[nul_pos] == 0 {
                // TODO: replace with `<[u8]>::split_at(nul_pos + 1)` when const stabilized
                let slice = unsafe { core::slice::from_raw_parts(bytes.as_ptr(), nul_pos + 1) };
                return Self(slice);
            }
            nul_pos += 1;
        }
        panic!("PEM certificates should end with a NIL (`\\0`) ASCII character.")
    }

    pub const fn der(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub fn data(&self) -> &[u8] {
        self.0
    }

    #[allow(unused)]
    pub(crate) fn as_esp_idf_raw_ptr(&self) -> *const c_char {
        self.data().as_ptr().cast()
    }

    #[allow(unused)]
    pub(crate) fn as_esp_idf_raw_len(&self) -> usize {
        self.data().len()
    }
}

impl Debug for X509<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        f.debug_struct("X509").finish_non_exhaustive()
    }
}

#[cfg(feature = "alloc")]
pub use self::hw_key::*;

#[cfg(feature = "alloc")]
mod hw_key {
    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    use core::fmt::Debug;

    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    use alloc::{boxed::Box, sync::Arc};

    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    use crate::sys::*;

    /// A private key for TLS client authentication which never leaves the hardware.
    ///
    /// It is used instead of a private key in memory, so it is mutually exclusive with the
    /// private key setting of the configuration it is used with. The client certificate
    /// matching the key still has to be configured.
    ///
    /// Which variants are available depends on the chip and on the ESP-IDF configuration.
    #[derive(Clone, Debug)]
    pub enum HwPrivateKey {
        /// An RSA key used through the Digital Signature (DS) peripheral
        #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
        Ds(EspDsKey),
        /// An ECDSA key used through the ECDSA peripheral
        /// (requires `CONFIG_MBEDTLS_HARDWARE_ECDSA_SIGN=y`)
        #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
        Ecdsa(EspEcdsaKey),
    }

    /// An RSA private key, encrypted for the Digital Signature (DS) peripheral of this chip.
    ///
    /// The key is provisioned once, e.g. with `configure_esp_secure_cert.py` of
    /// `esp_secure_cert_mgr`: an HMAC key is burned into an eFuse key block, and the RSA key
    /// is encrypted with a key derived from it. The DS peripheral decrypts and uses the RSA key
    /// internally, and the encrypted key is useless on any other chip.
    ///
    /// Clones share the encrypted key, so cloning is cheap.
    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    #[derive(Clone)]
    pub struct EspDsKey {
        data: Arc<esp_ds_data_t>,
        hmac_key_id: u8,
        rsa_key_bits: u16,
    }

    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    impl EspDsKey {
        /// Create a key from the outputs of its provisioning.
        ///
        /// - `ciphertext`: the encrypted key parameters
        /// - `iv`: the initialization vector they were encrypted with
        /// - `rsa_key_bits`: the length of the RSA key in bits (1024, 2048, 3072 or 4096)
        /// - `hmac_key_id`: the index (0 to 5) of the eFuse key block (`KEY0` to `KEY5`) holding
        ///   the HMAC key
        ///
        /// Returns `ESP_ERR_INVALID_ARG` if any of these does not match the DS peripheral.
        pub fn new(
            ciphertext: &[u8],
            iv: &[u8],
            rsa_key_bits: u16,
            hmac_key_id: u8,
        ) -> Result<Self, EspError> {
            let rsa_length = match rsa_key_bits {
                1024 => esp_digital_signature_length_t_ESP_DS_RSA_1024,
                2048 => esp_digital_signature_length_t_ESP_DS_RSA_2048,
                3072 => esp_digital_signature_length_t_ESP_DS_RSA_3072,
                4096 => esp_digital_signature_length_t_ESP_DS_RSA_4096,
                _ => return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>()),
            };

            let mut data = Arc::new(esp_ds_data_t::default());
            let raw = Arc::get_mut(&mut data).unwrap();

            if ciphertext.len() != raw.c.len()
                || iv.len() != core::mem::size_of_val(&raw.iv)
                || hmac_key_id as u32 >= hmac_key_id_t_HMAC_KEY_MAX
            {
                return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
            }

            raw.rsa_length = rsa_length;
            raw.c.copy_from_slice(ciphertext);

            // The IV is stored as native-endian words; the chips are little-endian
            for (word, bytes) in raw.iv.iter_mut().zip(iv.chunks_exact(4)) {
                *word = u32::from_le_bytes(bytes.try_into().unwrap());
            }

            Ok(Self {
                data,
                hmac_key_id,
                rsa_key_bits,
            })
        }

        /// Return the length of the RSA key in bits.
        pub const fn rsa_key_bits(&self) -> u16 {
            self.rsa_key_bits
        }

        /// Return the index of the eFuse key block holding the HMAC key.
        pub const fn hmac_key_id(&self) -> u8 {
            self.hmac_key_id
        }
    }

    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    impl Debug for EspDsKey {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("EspDsKey")
                .field("rsa_key_bits", &self.rsa_key_bits)
                .field("hmac_key_id", &self.hmac_key_id)
                .finish_non_exhaustive()
        }
    }

    /// An ECDSA (P-256) private key burned into an eFuse key block, used through the ECDSA
    /// peripheral.
    #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub struct EspEcdsaKey {
        efuse_block: u8,
    }

    #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
    impl EspEcdsaKey {
        /// Create a key from the eFuse block holding it, as an `esp_efuse_block_t`
        /// (e.g. `EFUSE_BLK_KEY0`).
        pub const fn new(efuse_block: u8) -> Self {
            Self { efuse_block }
        }

        /// Return the eFuse block holding the key.
        pub const fn efuse_block(&self) -> u8 {
            self.efuse_block
        }
    }

    /// Mirror of ESP-IDF's `esp_ds_data_ctx_t`, which the TLS configurations of ESP-IDF take
    /// as their `ds_data`. It is not part of the bindings, as each ESP-IDF release defines it
    /// in a different internal header of its mbedTLS port; its layout is the same in all of them.
    #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
    #[repr(C)]
    struct DsDataCtx {
        esp_ds_data: *mut esp_ds_data_t,
        efuse_key_id: u8,
        rsa_length_bits: u16,
    }

    /// The C-side state of a [`HwPrivateKey`].
    ///
    /// ESP-IDF keeps a pointer to it in the configuration of the client using the key, so it has
    /// to live as long as that client.
    #[derive(Default)]
    pub(crate) struct RawHwPrivateKey {
        #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
        ds: Option<(EspDsKey, Box<DsDataCtx>)>,
        #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
        ecdsa_efuse_block: Option<u8>,
    }

    impl RawHwPrivateKey {
        #[allow(unused_variables)]
        pub(crate) fn new(key: Option<&HwPrivateKey>) -> Self {
            Self {
                #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
                ds: match key {
                    Some(HwPrivateKey::Ds(key)) => {
                        let ctx = Box::new(DsDataCtx {
                            esp_ds_data: Arc::as_ptr(&key.data) as *mut _,
                            efuse_key_id: key.hmac_key_id,
                            rsa_length_bits: key.rsa_key_bits,
                        });

                        Some((key.clone(), ctx))
                    }
                    #[allow(unreachable_patterns)]
                    _ => None,
                },
                #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
                ecdsa_efuse_block: match key {
                    Some(HwPrivateKey::Ecdsa(key)) => Some(key.efuse_block),
                    #[allow(unreachable_patterns)]
                    _ => None,
                },
            }
        }

        /// The `ds_data` to set in the ESP-IDF configuration; null when not using the DS
        /// peripheral.
        #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
        pub(crate) fn ds_data(&self) -> *mut core::ffi::c_void {
            match &self.ds {
                Some((_, ctx)) => &**ctx as *const DsDataCtx as *mut _,
                None => core::ptr::null_mut(),
            }
        }

        /// The eFuse block to set in the ESP-IDF configuration when using the ECDSA peripheral.
        #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
        pub(crate) fn ecdsa_efuse_block(&self) -> Option<u8> {
            self.ecdsa_efuse_block
        }
    }

    // SAFETY: ESP-IDF only reads the context and the encrypted key it points to, which are
    // owned here and never modified after construction
    unsafe impl Send for RawHwPrivateKey {}
    unsafe impl Sync for RawHwPrivateKey {}
}

#[cfg(all(
    esp_idf_comp_esp_tls_enabled,
    any(esp_idf_esp_tls_using_mbedtls, esp_idf_esp_tls_using_wolfssl)
))]
mod esptls {
    use core::ffi::c_char;
    #[cfg(esp_idf_esp_tls_server_cert_select_hook)]
    use core::ffi::c_int;
    use core::task::{Context, Poll};
    use core::time::Duration;
    #[allow(unused_imports)]
    use core::{pin::Pin, task::ready};

    use embedded_svc::io;

    use super::X509;

    use crate::{
        io::EspIOError,
        private::cstr::{cstr_from_str_truncating, CStr},
        sys::{
            self, EspError, ESP_ERR_NO_MEM, ESP_FAIL, ESP_TLS_ERR_SSL_WANT_READ,
            ESP_TLS_ERR_SSL_WANT_WRITE, EWOULDBLOCK,
        },
    };

    /// see https://www.ietf.org/rfc/rfc3280.txt ub-common-name-length
    const MAX_COMMON_NAME_LENGTH: usize = 64;

    pub struct Config<'a> {
        /// up to 9 ALPNs allowed, with avg 10 bytes for each name
        ///
        /// Needs the `alloc` feature.
        pub alpn_protos: Option<&'a [&'a str]>,
        pub ca_cert: Option<X509<'a>>,
        pub client_cert: Option<X509<'a>>,
        pub client_key: Option<X509<'a>>,
        pub client_key_password: Option<&'a str>,
        pub non_block: bool,
        pub use_secure_element: bool,
        pub timeout_ms: u32,
        pub use_global_ca_store: bool,
        pub common_name: Option<&'a str>,
        pub skip_common_name: bool,
        pub keep_alive_cfg: Option<KeepAliveConfig>,
        pub psk_hint_key: Option<PskHintKey<'a>>,
        /// whether to use esp_crt_bundle_attach, see <https://docs.espressif.com/projects/esp-idf/en/latest/esp32s2/api-reference/protocols/esp_crt_bundle.html>
        #[cfg(esp_idf_mbedtls_certificate_bundle)]
        pub use_crt_bundle_attach: bool,
        /// A private key kept in hardware, used instead of `client_key`
        #[cfg(feature = "alloc")]
        pub hw_private_key: Option<super::HwPrivateKey>,
        pub is_plain_tcp: bool,
    }

    impl Config<'_> {
        pub const fn new() -> Self {
            Self {
                alpn_protos: None,
                ca_cert: None,
                client_cert: None,
                client_key: None,
                client_key_password: None,
                non_block: false,
                use_secure_element: false,
                timeout_ms: 4000,
                use_global_ca_store: false,
                common_name: None,
                skip_common_name: false,
                keep_alive_cfg: None,
                psk_hint_key: None,
                #[cfg(esp_idf_mbedtls_certificate_bundle)]
                use_crt_bundle_attach: true,
                #[cfg(feature = "alloc")]
                hw_private_key: None,
                is_plain_tcp: false,
            }
        }

        fn try_into_raw(
            &self,
            bufs: &mut RawConfigBufs,
            alpn: &mut SessionAlpnProtos,
        ) -> Result<sys::esp_tls_cfg, EspError> {
            let mut rcfg: sys::esp_tls_cfg = Default::default();

            if let Some(ca_cert) = self.ca_cert {
                rcfg.__bindgen_anon_1.cacert_buf = ca_cert.data().as_ptr();
                rcfg.__bindgen_anon_2.cacert_bytes = ca_cert.data().len() as u32;
            }

            if let Some(client_cert) = self.client_cert {
                rcfg.__bindgen_anon_3.clientcert_buf = client_cert.data().as_ptr();
                rcfg.__bindgen_anon_4.clientcert_bytes = client_cert.data().len() as u32;
            }

            if let Some(client_key) = self.client_key {
                rcfg.__bindgen_anon_5.clientkey_buf = client_key.data().as_ptr();
                rcfg.__bindgen_anon_6.clientkey_bytes = client_key.data().len() as u32;
            }

            if let Some(ckp) = self.client_key_password {
                rcfg.clientkey_password = ckp.as_ptr();
                rcfg.clientkey_password_len = ckp.len() as u32;
            }

            #[cfg(feature = "alloc")]
            {
                if self.hw_private_key.is_some() && self.client_key.is_some() {
                    return Err(EspError::from_infallible::<{ sys::ESP_ERR_INVALID_ARG }>());
                }

                bufs.hw_private_key = super::RawHwPrivateKey::new(self.hw_private_key.as_ref());

                #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
                {
                    rcfg.ds_data = bufs.hw_private_key.ds_data();
                }

                #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
                if let Some(efuse_block) = bufs.hw_private_key.ecdsa_efuse_block() {
                    rcfg.use_ecdsa_peripheral = true;
                    rcfg.ecdsa_key_efuse_blk = efuse_block;
                }
            }

            if let Some(protos) = self.alpn_protos {
                rcfg.alpn_protos = alpn.get_or_init(protos)?;
            }

            rcfg.non_block = self.non_block;
            rcfg.use_secure_element = self.use_secure_element;
            rcfg.timeout_ms = self.timeout_ms as i32;
            rcfg.use_global_ca_store = self.use_global_ca_store;

            if let Some(common_name) = self.common_name {
                rcfg.common_name =
                    cstr_from_str_truncating(common_name, &mut bufs.common_name_buf).as_ptr();
            }

            rcfg.skip_common_name = self.skip_common_name;

            if let Some(kac) = &self.keep_alive_cfg {
                bufs.keep_alive_cfg = sys::tls_keep_alive_cfg {
                    keep_alive_enable: kac.enable,
                    keep_alive_idle: kac.idle.as_secs() as i32,
                    keep_alive_interval: kac.interval.as_secs() as i32,
                    keep_alive_count: kac.count as i32,
                };
                rcfg.keep_alive_cfg = &mut bufs.keep_alive_cfg as *mut _;
            }

            #[cfg(any(
                esp_idf_esp_tls_psk_verification,
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                esp_idf_version = "5.1",
                esp_idf_version = "5.2",
                esp_idf_version = "5.3",
                esp_idf_version = "5.4",
            ))]
            if let Some(psk) = &self.psk_hint_key {
                bufs.psk_hint_key = sys::psk_key_hint {
                    key: psk.key.as_ptr(),
                    key_size: psk.key.len(),
                    hint: psk.hint.as_ptr(),
                };
                rcfg.psk_hint_key = &bufs.psk_hint_key as *const _;
            }

            #[cfg(esp_idf_mbedtls_certificate_bundle)]
            if self.use_crt_bundle_attach {
                rcfg.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
            }

            rcfg.is_plain_tcp = self.is_plain_tcp;

            #[cfg(esp_idf_comp_lwip_enabled)]
            {
                rcfg.if_name = core::ptr::null_mut();
            }

            Ok(rcfg)
        }
    }

    impl Default for Config<'_> {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Data which the raw configuration points to, and which ESP-IDF reads while connecting
    struct RawConfigBufs {
        common_name_buf: [u8; MAX_COMMON_NAME_LENGTH + 1],
        keep_alive_cfg: sys::tls_keep_alive_cfg,
        #[cfg(any(
            esp_idf_esp_tls_psk_verification,
            esp_idf_version_major = "4",
            esp_idf_version = "5.0",
            esp_idf_version = "5.1",
            esp_idf_version = "5.2",
            esp_idf_version = "5.3",
            esp_idf_version = "5.4",
        ))]
        psk_hint_key: sys::psk_key_hint,
        #[cfg(feature = "alloc")]
        hw_private_key: super::RawHwPrivateKey,
    }

    unsafe impl Send for RawConfigBufs {}

    impl Default for RawConfigBufs {
        fn default() -> Self {
            RawConfigBufs {
                common_name_buf: [0; MAX_COMMON_NAME_LENGTH + 1],
                keep_alive_cfg: Default::default(),
                #[cfg(any(
                    esp_idf_esp_tls_psk_verification,
                    esp_idf_version_major = "4",
                    esp_idf_version = "5.0",
                    esp_idf_version = "5.1",
                    esp_idf_version = "5.2",
                    esp_idf_version = "5.3",
                    esp_idf_version = "5.4",
                ))]
                psk_hint_key: Default::default(),
                #[cfg(feature = "alloc")]
                hw_private_key: Default::default(),
            }
        }
    }

    /// The ALPN protocol list of a session
    ///
    /// mbedTLS reads the list during the handshake, and points into it until the session is
    /// destroyed. The list is therefore on the heap, where moving the `EspTls` does not move it.
    #[derive(Default)]
    struct SessionAlpnProtos(#[cfg(feature = "alloc")] Option<alloc::boxed::Box<AlpnProtos>>);

    impl SessionAlpnProtos {
        /// Copy `protos` here, unless this session already has a list, and return the list
        /// to hand to ESP-IDF
        ///
        /// ESP-IDF reads the list once per session, so the later calls of a non-blocking
        /// handshake keep the first list.
        #[cfg(feature = "alloc")]
        fn get_or_init(&mut self, protos: &[&str]) -> Result<*mut *const c_char, EspError> {
            if self.0.is_none() {
                let mut alpn = alloc::boxed::Box::new(AlpnProtos {
                    protos: [core::ptr::null(); 10],
                    cbuf: [0; 99],
                });
                alpn.protos =
                    crate::private::cstr::cstr_arr_from_str_slice(protos, &mut alpn.cbuf)?;

                self.0 = Some(alpn);
            }

            Ok(self.0.as_mut().unwrap().protos.as_mut_ptr())
        }

        #[cfg(not(feature = "alloc"))]
        fn get_or_init(&mut self, _protos: &[&str]) -> Result<*mut *const c_char, EspError> {
            Err(EspError::from_infallible::<{ sys::ESP_ERR_NOT_SUPPORTED }>())
        }
    }

    #[cfg(feature = "alloc")]
    struct AlpnProtos {
        /// Up to 9 protocols, then null
        protos: [*const c_char; 10],
        cbuf: [u8; 99],
    }

    type AlpnBuf = [u8; 16];

    #[derive(Clone, Default)]
    pub struct CompletedHandshake {
        alpn: AlpnBuf,
    }

    impl CompletedHandshake {
        pub fn alpn_proto(&self) -> Option<&str> {
            let p = CStr::from_bytes_until_nul(self.alpn.as_slice()).unwrap();
            // Safety: the bytes always come from a user supplied &str.
            let p = unsafe { core::str::from_utf8_unchecked(p.to_bytes()) };

            // A valid protocol is never empty.
            if !p.is_empty() {
                Some(p)
            } else {
                None
            }
        }

        // Safety: Must be called while the configured ALPN protocol strings are valid.
        unsafe fn extract(raw: *mut sys::esp_tls) -> CompletedHandshake {
            CompletedHandshake {
                alpn: unsafe { Self::extract_alpn(raw) }.unwrap_or_default(),
            }
        }

        #[cfg(not(all(
            not(esp_idf_version_major = "4"),
            esp_idf_comp_esp_tls_enabled,
            esp_idf_esp_tls_using_mbedtls,
            esp_idf_mbedtls_ssl_alpn
        )))]
        unsafe fn extract_alpn(_raw: *mut sys::esp_tls) -> Option<AlpnBuf> {
            None
        }

        #[cfg(all(
            not(esp_idf_version_major = "4"),
            esp_idf_comp_esp_tls_enabled,
            esp_idf_esp_tls_using_mbedtls,
            esp_idf_mbedtls_ssl_alpn
        ))]
        #[warn(unsafe_op_in_unsafe_fn)]
        unsafe fn extract_alpn(raw: *mut sys::esp_tls) -> Option<AlpnBuf> {
            let raw: *mut sys::mbedtls_ssl_context =
                unsafe { sys::esp_tls_get_ssl_context(raw) }.cast();

            if raw.is_null() {
                return None;
            }

            let chosen = unsafe { sys::mbedtls_ssl_get_alpn_protocol(raw) };
            if chosen.is_null() {
                return None;
            }

            let mut proto = AlpnBuf::default();
            let chosen = unsafe { CStr::from_ptr(chosen) };
            let chosen_bytes = chosen.to_bytes_with_nul();
            if chosen_bytes.len() > proto.len() {
                return None;
            }

            proto[..chosen_bytes.len()].copy_from_slice(chosen_bytes);

            Some(proto)
        }
    }

    #[derive(Clone, Debug)]
    pub struct KeepAliveConfig {
        /// Enable keep-alive timeout
        pub enable: bool,
        /// Keep-alive idle time (second)
        pub idle: Duration,
        /// Keep-alive interval time (second)
        pub interval: Duration,
        /// Keep-alive packet retry send count
        pub count: u32,
    }

    pub struct PskHintKey<'a> {
        pub key: &'a [u8],
        pub hint: &'a CStr,
    }

    #[cfg(any(
        esp_idf_esp_tls_server,
        all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls),
    ))]
    pub struct ServerConfig<'a> {
        /// up to 9 ALPNs allowed, with avg 10 bytes for each name
        ///
        /// Needs the `alloc` feature.
        pub alpn_protos: Option<&'a [&'a str]>,
        pub ca_cert: Option<X509<'a>>,
        pub server_cert: Option<X509<'a>>,
        pub server_key: Option<X509<'a>>,
        pub server_key_password: Option<&'a str>,
        pub use_secure_element: bool,
        /// Overall TLS handshake timeout in milliseconds.
        ///
        /// `0` means the ESP-TLS default (10 seconds).
        ///
        /// Only honored by the blocking [`EspTls::negotiate_server`] path.
        /// The non-blocking [`EspTls::negotiate_server_init`] /
        /// [`EspTls::negotiate_server_continue`] path (and thus
        /// `EspAsyncTls::negotiate_server`) is not bounded by it, so callers
        /// there should enforce their own deadline.
        ///
        /// Only available on ESP-IDF >= 5.5.0 (`esp_tls_cfg_server::tls_handshake_timeout_ms`).
        #[cfg(esp_idf_version_at_least_5_5_0)]
        pub tls_handshake_timeout_ms: u32,
        #[cfg(esp_idf_esp_tls_server_cert_select_hook)]
        pub handshake_callback: Option<extern "C" fn(*mut sys::mbedtls_ssl_context) -> c_int>,
    }

    #[cfg(any(
        esp_idf_esp_tls_server,
        all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls),
    ))]
    impl<'a> ServerConfig<'a> {
        pub const fn new() -> Self {
            Self {
                alpn_protos: None,
                ca_cert: None,
                server_cert: None,
                server_key: None,
                server_key_password: None,
                use_secure_element: false,
                #[cfg(esp_idf_version_at_least_5_5_0)]
                tls_handshake_timeout_ms: 0,
                #[cfg(esp_idf_esp_tls_server_cert_select_hook)]
                handshake_callback: None,
            }
        }

        fn try_into_raw(
            &self,
            alpn: &mut SessionAlpnProtos,
        ) -> Result<sys::esp_tls_cfg_server, EspError> {
            let mut rcfg: sys::esp_tls_cfg_server = Default::default();

            if let Some(ca_cert) = self.ca_cert {
                rcfg.__bindgen_anon_1.cacert_buf = ca_cert.data().as_ptr();
                rcfg.__bindgen_anon_2.cacert_bytes = ca_cert.data().len() as u32;
            }

            if let Some(server_cert) = self.server_cert {
                rcfg.__bindgen_anon_3.servercert_buf = server_cert.data().as_ptr();
                rcfg.__bindgen_anon_4.servercert_bytes = server_cert.data().len() as u32;
            }

            if let Some(server_key) = self.server_key {
                rcfg.__bindgen_anon_5.serverkey_buf = server_key.data().as_ptr();
                rcfg.__bindgen_anon_6.serverkey_bytes = server_key.data().len() as u32;
            }

            if let Some(ckp) = self.server_key_password {
                rcfg.serverkey_password = ckp.as_ptr();
                rcfg.serverkey_password_len = ckp.len() as u32;
            }

            if let Some(protos) = self.alpn_protos {
                rcfg.alpn_protos = alpn.get_or_init(protos)?;
            }

            rcfg.use_secure_element = self.use_secure_element;
            #[cfg(esp_idf_version_at_least_5_5_0)]
            {
                rcfg.tls_handshake_timeout_ms = self.tls_handshake_timeout_ms;
            }

            #[cfg(esp_idf_esp_tls_server_cert_select_hook)]
            if let Some(cb) = self.handshake_callback {
                rcfg.cert_select_cb = cb;
            }

            Ok(rcfg)
        }
    }

    #[cfg(any(
        esp_idf_esp_tls_server,
        all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls),
    ))]
    impl<'a> Default for ServerConfig<'a> {
        fn default() -> Self {
            Self::new()
        }
    }

    pub trait Socket {
        /// Returns the integer FD.
        fn handle(&self) -> i32;
        /// This is called before cleaning up the the tls context and is responsible
        /// for essentially giving up ownership of the socket such that it can safely
        /// be closed by the ESP IDF.
        fn release(&mut self) -> Result<(), EspError>;
    }

    pub trait PollableSocket: Socket {
        fn poll_readable(&self, ctx: &mut Context) -> Poll<Result<(), EspError>>;
        fn poll_writable(&self, ctx: &mut Context) -> Poll<Result<(), EspError>>;
    }

    pub struct InternalSocket(());

    impl Socket for InternalSocket {
        fn handle(&self) -> i32 {
            unreachable!()
        }

        fn release(&mut self) -> Result<(), EspError> {
            Ok(())
        }
    }

    /// Wrapper for `esp-tls` module. Only supports synchronous operation for now.
    pub struct EspTls<S>
    where
        S: Socket,
    {
        raw: *mut sys::esp_tls,
        socket: S,
        #[cfg(any(
            esp_idf_esp_tls_server,
            all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls,),
        ))]
        server_session: bool,
        /// Dropped after `esp_tls_conn_destroy`, as fields drop after `Drop::drop`
        alpn_protos: SessionAlpnProtos,
    }

    // A single Mbed TLS context itself is safe to send across threads.
    // Require the threading implementation to be enabled since a shared context such as RSA or X509 could be used by multiple threads at once.
    // See https://mbed-tls.readthedocs.io/en/latest/kb/development/thread-safety-and-multi-threading/
    #[cfg(all(
        esp_idf_comp_esp_tls_enabled,
        esp_idf_esp_tls_using_mbedtls,
        esp_idf_mbedtls_threading_c
    ))]
    unsafe impl<S> Send for EspTls<S> where S: Send + Socket {}

    impl EspTls<InternalSocket> {
        /// Create a new `EspTls` instance using internally-managed socket.
        ///
        /// # Errors
        ///
        /// * `ESP_ERR_NO_MEM` if not enough memory to create the TLS connection
        pub fn new() -> Result<Self, EspError> {
            let raw = unsafe { sys::esp_tls_init() };
            if !raw.is_null() {
                Ok(Self {
                    raw,
                    socket: InternalSocket(()),
                    #[cfg(any(
                        esp_idf_esp_tls_server,
                        all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls,),
                    ))]
                    server_session: false,
                    alpn_protos: Default::default(),
                })
            } else {
                Err(EspError::from_infallible::<ESP_ERR_NO_MEM>())
            }
        }

        /// Establish a TLS/SSL connection with the specified host and port, using an internally-managed socket.
        ///
        /// # Errors
        ///
        /// * `ESP_ERR_INVALID_SIZE` if `cfg.alpn_protos` exceeds 9 elements or avg 10 bytes/ALPN
        /// * `ESP_FAIL` if connection could not be established
        /// * `ESP_TLS_ERR_SSL_WANT_READ` if the socket is in non-blocking mode and it is not ready for reading
        /// * `ESP_TLS_ERR_SSL_WANT_WRITE` if the socket is in non-blocking mode and it is not ready for writing
        /// * `EWOULDBLOCK` if the socket is in non-blocking mode and it is not ready either for reading or writing (a peculiarity/bug of the `esp-tls` C module)
        pub fn connect(
            &mut self,
            host: &str,
            port: u16,
            cfg: &Config,
        ) -> Result<CompletedHandshake, EspError> {
            let mut bufs = RawConfigBufs::default();
            let rcfg = cfg.try_into_raw(&mut bufs, &mut self.alpn_protos)?;

            let res = self.internal_connect(host, port, cfg.non_block, &rcfg);

            // Make sure buffers are held long enough
            #[allow(clippy::drop_non_drop)]
            drop(bufs);

            res
        }
    }

    impl<S> EspTls<S>
    where
        S: Socket,
    {
        /// Create a new `EspTls` instance adopting the supplied socket.
        /// The socket should be in a connected state.
        ///
        /// # Errors
        ///
        /// * `ESP_ERR_NO_MEM` if not enough memory to create the TLS connection
        #[cfg(all(
            not(esp_idf_version_major = "4"),
            any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
        ))]
        pub fn adopt(socket: S) -> Result<Self, EspError> {
            let raw = unsafe { sys::esp_tls_init() };
            if !raw.is_null() {
                sys::esp!(unsafe { sys::esp_tls_set_conn_sockfd(raw, socket.handle()) })?;

                sys::esp!(unsafe {
                    sys::esp_tls_set_conn_state(raw, sys::esp_tls_conn_state_ESP_TLS_CONNECTING)
                })?;

                Ok(Self {
                    raw,
                    socket,
                    #[cfg(any(
                        esp_idf_esp_tls_server,
                        all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls,),
                    ))]
                    server_session: false,
                    alpn_protos: Default::default(),
                })
            } else {
                Err(EspError::from_infallible::<ESP_ERR_NO_MEM>())
            }
        }

        /// Establish a TLS/SSL connection using the adopted socket.
        ///
        /// # Errors
        ///
        /// * `ESP_ERR_INVALID_SIZE` if `cfg.alpn_protos` exceeds 9 elements or avg 10 bytes/ALPN
        /// * `ESP_FAIL` if connection could not be established
        /// * `ESP_TLS_ERR_SSL_WANT_READ` if the socket is in non-blocking mode and it is not ready for reading
        /// * `ESP_TLS_ERR_SSL_WANT_WRITE` if the socket is in non-blocking mode and it is not ready for writing
        /// * `EWOULDBLOCK` if the socket is in non-blocking mode and it is not ready either for reading or writing (a peculiarity/bug of the `esp-tls` C module)
        #[cfg(all(
            not(esp_idf_version_major = "4"),
            any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
        ))]
        pub fn negotiate(
            &mut self,
            host: &str,
            cfg: &Config,
        ) -> Result<CompletedHandshake, EspError> {
            let mut bufs = RawConfigBufs::default();
            let rcfg = cfg.try_into_raw(&mut bufs, &mut self.alpn_protos)?;

            let res = self.internal_connect(host, 0, cfg.non_block, &rcfg);

            // Make sure buffers are held long enough
            #[allow(clippy::drop_non_drop)]
            drop(bufs);

            res
        }

        /// Establish a TLS/SSL connection using the adopted connection, acting as the server.
        ///
        /// This call is **blocking**. On ESP-IDF >= 5.5.0 the duration is bounded by
        /// `cfg.tls_handshake_timeout_ms` (or the ESP-TLS default of 10 s when that
        /// field is `0`). When the caller cannot afford to stall, prefer
        /// `EspAsyncTls::negotiate_server`, or drive a non-blocking socket manually
        /// with `negotiate_server_init` + `negotiate_server_continue`
        /// (both ESP-IDF >= 5.5.0).
        ///
        /// # Errors
        ///
        /// * `ESP_FAIL` if connection could not be established
        #[cfg(any(
            esp_idf_esp_tls_server,
            all(esp_idf_version_at_least_5_3_0, esp_idf_esp_tls_using_mbedtls),
        ))]
        pub fn negotiate_server(&mut self, cfg: &ServerConfig) -> Result<(), EspError> {
            let mut rcfg = cfg.try_into_raw(&mut self.alpn_protos)?;

            unsafe {
                let error =
                    sys::esp_tls_server_session_create(&mut rcfg, self.socket.handle(), self.raw);
                if error != 0 {
                    log::error!("failed to create tls server session (error {error})");
                    return Err(EspError::from_infallible::<ESP_FAIL>());
                }
            }
            self.server_session = true;

            Ok(())
        }

        /// Begin a server-side TLS handshake on an already-adopted socket.
        ///
        /// Requires ESP-IDF >= 5.5.0 with mbedTLS. The socket should typically be
        /// non-blocking. Complete the handshake by repeatedly calling
        /// [`Self::negotiate_server_continue`] until it returns `Ok(())`.
        /// Certificate material referenced by `cfg` must remain valid for the
        /// duration of this call (mbedTLS parses it here).
        ///
        /// # Errors
        ///
        /// * `ESP_FAIL` / other ESP-TLS errors if session setup fails
        #[cfg(all(esp_idf_version_at_least_5_5_0, esp_idf_esp_tls_using_mbedtls,))]
        pub fn negotiate_server_init(&mut self, cfg: &ServerConfig) -> Result<(), EspError> {
            let mut rcfg = cfg.try_into_raw(&mut self.alpn_protos)?;

            // esp_tls_server_session_init returns esp_err_t (0 on success).
            sys::esp!(unsafe {
                sys::esp_tls_server_session_init(&mut rcfg, self.socket.handle(), self.raw)
            })?;
            self.server_session = true;

            Ok(())
        }

        /// Continue a server-side handshake started with [`Self::negotiate_server_init`].
        ///
        /// Requires ESP-IDF >= 5.5.0 with mbedTLS.
        ///
        /// Returns `Ok(())` once the handshake is complete and the session is
        /// ready for read/write.
        ///
        /// # Errors
        ///
        /// * `ESP_TLS_ERR_SSL_WANT_READ` if the socket is in non-blocking mode and it is not ready for reading
        /// * `ESP_TLS_ERR_SSL_WANT_WRITE` if the socket is in non-blocking mode and it is not ready for writing
        /// * `ESP_FAIL` if the handshake failed; drop the connection
        #[allow(clippy::unnecessary_cast)]
        #[cfg(all(esp_idf_version_at_least_5_5_0, esp_idf_esp_tls_using_mbedtls,))]
        pub fn negotiate_server_continue(&mut self) -> Result<(), EspError> {
            let ret = unsafe { sys::esp_tls_server_session_continue_async(self.raw) };

            match ret {
                0 => Ok(()),
                ESP_TLS_ERR_SSL_WANT_READ => Err(EspError::from_infallible::<
                    { ESP_TLS_ERR_SSL_WANT_READ as i32 },
                >()),
                ESP_TLS_ERR_SSL_WANT_WRITE => Err(EspError::from_infallible::<
                    { ESP_TLS_ERR_SSL_WANT_WRITE as i32 },
                >()),
                _ => {
                    log::error!("TLS server handshake continue failed (error {ret})");
                    Err(EspError::from_infallible::<ESP_FAIL>())
                }
            }
        }

        #[allow(clippy::unnecessary_cast)]
        fn internal_connect(
            &mut self,
            host: &str,
            port: u16,
            asynch: bool,
            cfg: &sys::esp_tls_cfg,
        ) -> Result<CompletedHandshake, EspError> {
            let ret = unsafe {
                if asynch {
                    sys::esp_tls_conn_new_async(
                        host.as_bytes().as_ptr() as *const c_char,
                        host.len() as i32,
                        port as i32,
                        cfg,
                        self.raw,
                    )
                } else {
                    sys::esp_tls_conn_new_sync(
                        host.as_bytes().as_ptr() as *const c_char,
                        host.len() as i32,
                        port as i32,
                        cfg,
                        self.raw,
                    )
                }
            };

            match ret {
                1 => Ok(unsafe { CompletedHandshake::extract(self.raw) }),
                ESP_TLS_ERR_SSL_WANT_READ => Err(EspError::from_infallible::<
                    { ESP_TLS_ERR_SSL_WANT_READ as i32 },
                >()),
                ESP_TLS_ERR_SSL_WANT_WRITE => Err(EspError::from_infallible::<
                    { ESP_TLS_ERR_SSL_WANT_WRITE as i32 },
                >()),
                0 => Err(EspError::from_infallible::<{ EWOULDBLOCK as i32 }>()),
                _ => Err(EspError::from_infallible::<ESP_FAIL>()),
            }
        }

        /// Read in the supplied buffer. Returns the number of bytes read.
        ///
        ///
        /// # Errors
        /// * `ESP_TLS_ERR_SSL_WANT_READ` if the socket is in non-blocking mode and it is not ready for reading
        /// * `ESP_TLS_ERR_SSL_WANT_WRITE` if the socket is in non-blocking mode and it is not ready for writing
        /// * Any other `EspError` for a general error
        pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspError> {
            if buf.is_empty() {
                return Ok(0);
            }

            let ret = self.read_raw(buf);
            // ESP docs treat 0 as error, but in Rust it's common to return 0 from `Read::read` to indicate eof
            if ret >= 0 {
                Ok(ret as usize)
            } else {
                Err(EspError::from(ret as i32).unwrap())
            }
        }

        #[cfg(esp_idf_version_major = "4")]
        fn read_raw(&mut self, buf: &mut [u8]) -> isize {
            // cannot call esp_tls_conn_read bc it's inline in v4
            let esp_tls = unsafe { core::ptr::read_unaligned(self.raw) };
            let read_func = esp_tls.read.unwrap();
            unsafe { read_func(self.raw, buf.as_mut_ptr() as *mut c_char, buf.len()) }
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        fn read_raw(&mut self, buf: &mut [u8]) -> isize {
            use core::ffi::c_void;

            unsafe { sys::esp_tls_conn_read(self.raw, buf.as_mut_ptr() as *mut c_void, buf.len()) }
        }

        /// Write the supplied buffer. Returns the number of bytes written.
        ///
        /// # Errors
        /// * `ESP_TLS_ERR_SSL_WANT_READ` if the socket is in non-blocking mode and it is not ready for reading
        /// * `ESP_TLS_ERR_SSL_WANT_WRITE` if the socket is in non-blocking mode and it is not ready for writing
        /// * Any other `EspError` for a general error
        pub fn write(&mut self, buf: &[u8]) -> Result<usize, EspError> {
            if buf.is_empty() {
                return Ok(0);
            }

            let ret = self.write_raw(buf);
            if ret >= 0 {
                Ok(ret as usize)
            } else {
                Err(EspError::from(ret as i32).unwrap())
            }
        }

        pub fn write_all(&mut self, buf: &[u8]) -> Result<(), EspError> {
            let mut buf = buf;

            while !buf.is_empty() {
                match self.write(buf) {
                    Ok(0) => panic!("zero-length write."),
                    Ok(n) => buf = &buf[n..],
                    Err(e) => return Err(e),
                }
            }

            Ok(())
        }

        #[cfg(esp_idf_version_major = "4")]
        fn write_raw(&mut self, buf: &[u8]) -> isize {
            // cannot call esp_tls_conn_write bc it's inline
            let esp_tls = unsafe { core::ptr::read_unaligned(self.raw) };
            let write_func = esp_tls.write.unwrap();
            unsafe { write_func(self.raw, buf.as_ptr() as *const c_char, buf.len()) }
        }

        #[cfg(not(esp_idf_version_major = "4"))]
        fn write_raw(&mut self, buf: &[u8]) -> isize {
            use core::ffi::c_void;

            unsafe { sys::esp_tls_conn_write(self.raw, buf.as_ptr() as *const c_void, buf.len()) }
        }

        pub fn context_handle(&self) -> *mut sys::esp_tls {
            self.raw
        }
    }

    impl<S> Drop for EspTls<S>
    where
        S: Socket,
    {
        fn drop(&mut self) {
            let _ = self.socket.release();

            unsafe {
                // use esp_tls_conn_destroy for both client and server
                sys::esp_tls_conn_destroy(self.raw);
            }
        }
    }

    impl<S> io::ErrorType for EspTls<S>
    where
        S: Socket,
    {
        type Error = EspIOError;
    }

    impl<S> io::Read for EspTls<S>
    where
        S: Socket,
    {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspIOError> {
            EspTls::read(self, buf).map_err(EspIOError)
        }
    }

    impl<S> io::Write for EspTls<S>
    where
        S: Socket,
    {
        fn write(&mut self, buf: &[u8]) -> Result<usize, EspIOError> {
            EspTls::write(self, buf).map_err(EspIOError)
        }

        fn flush(&mut self) -> Result<(), EspIOError> {
            Ok(())
        }
    }
    #[cfg(all(
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    pub struct EspAsyncTls<S>(crate::private::mutex::Mutex<EspTls<S>>)
    where
        S: PollableSocket;

    #[cfg(all(
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        /// Create a new `AsyncEspTls` instance adopting the supplied socket.
        /// The socket should be in a connected state.
        ///
        /// # Errors
        ///
        /// * `ESP_ERR_NO_MEM` if not enough memory to create the TLS connection
        pub fn adopt(socket: S) -> Result<Self, EspError> {
            Ok(Self(crate::private::mutex::Mutex::new(EspTls::adopt(
                socket,
            )?)))
        }

        /// Establish a TLS/SSL connection using the adopted socket.
        ///
        /// # Errors
        ///
        /// * `ESP_ERR_INVALID_SIZE` if `cfg.alpn_protos` exceeds 9 elements or avg 10 bytes/ALPN
        /// * `ESP_FAIL` if connection could not be established
        pub async fn negotiate(
            &mut self,
            hostname: &str,
            cfg: &Config<'_>,
        ) -> Result<CompletedHandshake, EspError> {
            struct AssertSend<T>(T);
            unsafe impl<T> Send for AssertSend<T> {}

            let mut bufs = RawConfigBufs::default();
            let mut rcfg: AssertSend<sys::esp_tls_cfg> =
                AssertSend(cfg.try_into_raw(&mut bufs, &mut self.0.get_mut().alpn_protos)?);

            // It is a bit unintuitive, but when an async socket is being adopted, `non_block` should be set to false.
            //
            // Background:
            // `non_block = true` is only used at one place in the ESP IDF code and that is to run
            // a check - with `select` - whether the socket is really connected.
            // However, we want to avoid the `select()` call, as
            // (a) It won't work, because we jump directly into the ESP_TLS_CONNECTING state as we adopt a socket.
            //.    As a side effect, the select() call is not properly initialized.
            // (b) The adopted socket might be registered in a select() loop already.
            //
            // Avoiding the connectivity check with `select()` should be fine, as the adopted socket
            // must be already connected anyway (API requirement).
            rcfg.0.non_block = false;

            let res = loop {
                let res = self
                    .0
                    .get_mut()
                    .internal_connect(hostname, 0, true, &rcfg.0);

                match res {
                    Err(e) => self.wait(e).await?,
                    other => break other,
                }
            };

            // Make sure buffers are held long enough
            #[allow(clippy::drop_non_drop)]
            drop(bufs);

            res
        }

        /// Establish a TLS/SSL connection using the adopted socket, acting as the server.
        ///
        /// Requires ESP-IDF >= 5.5.0 with mbedTLS.
        ///
        /// Note that the handshake is not bounded in time (`cfg.tls_handshake_timeout_ms`
        /// is only honored by the blocking `EspTls::negotiate_server`), so a
        /// misbehaving peer can keep the negotiation going indefinitely.
        /// Callers which cannot afford that should race this future against a timer.
        ///
        /// # Errors
        ///
        /// * `ESP_FAIL` if the connection could not be established
        #[cfg(all(esp_idf_version_at_least_5_5_0, esp_idf_esp_tls_using_mbedtls))]
        pub async fn negotiate_server(&mut self, cfg: &ServerConfig<'_>) -> Result<(), EspError> {
            self.0.get_mut().negotiate_server_init(cfg)?;

            loop {
                let res = self.0.get_mut().negotiate_server_continue();

                match res {
                    Err(e) => self.wait(e).await?,
                    Ok(()) => break Ok(()),
                }
            }
        }

        /// Read in the supplied buffer. Returns the number of bytes read.
        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, EspError> {
            core::future::poll_fn(|ctx| self.poll_read(ctx, buf)).await
        }

        pub fn poll_read(
            &self,
            ctx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<Result<usize, EspError>> {
            loop {
                let res = self.0.lock().read(buf);

                match res {
                    Err(e) => ready!(self.poll_wait(ctx, e))?,
                    Ok(n) => break Poll::Ready(Ok(n)),
                }
            }
        }

        /// Write the supplied buffer. Returns the number of bytes written.
        pub async fn write(&self, buf: &[u8]) -> Result<usize, EspError> {
            core::future::poll_fn(|ctx| self.poll_write(ctx, buf)).await
        }

        pub fn poll_write(
            &self,
            ctx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<Result<usize, EspError>> {
            loop {
                let res = self.0.lock().write(buf);

                match res {
                    Err(e) => ready!(self.poll_wait(ctx, e))?,
                    Ok(n) => break Poll::Ready(Ok(n)),
                }
            }
        }

        pub async fn write_all(&self, buf: &[u8]) -> Result<(), EspError> {
            let mut buf = buf;

            while !buf.is_empty() {
                match self.write(buf).await {
                    Ok(0) => panic!("zero-length write."),
                    Ok(n) => buf = &buf[n..],
                    Err(e) => return Err(e),
                }
            }

            Ok(())
        }

        fn poll_wait(&self, ctx: &mut Context<'_>, error: EspError) -> Poll<Result<(), EspError>> {
            const EWOULDBLOCK_I32: i32 = EWOULDBLOCK as i32;

            match error.code() {
                // EWOULDBLOCK models the "0" return code of esp_mbedtls_handshake() which does not allow us
                // to figure out whether we need the socket to become readable or writable
                // The code below is therefore a hack which just waits with a timeout for the socket to (eventually)
                // become readable as we actually don't even know if that's what esp_tls wants
                EWOULDBLOCK_I32 => {
                    let res = self.0.lock().socket.poll_writable(ctx);
                    crate::hal::delay::FreeRtos::delay_ms(0);
                    res
                }
                ESP_TLS_ERR_SSL_WANT_READ => self.0.lock().socket.poll_readable(ctx),
                ESP_TLS_ERR_SSL_WANT_WRITE => self.0.lock().socket.poll_writable(ctx),
                _ => Poll::Ready(Err(error)),
            }
        }

        async fn wait(&self, error: EspError) -> Result<(), EspError> {
            core::future::poll_fn(|ctx| self.poll_wait(ctx, error)).await
        }

        pub fn context_handle(&self) -> *mut sys::esp_tls {
            self.0.lock().context_handle()
        }
    }

    #[cfg(all(
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> io::ErrorType for EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        type Error = EspIOError;
    }

    #[cfg(all(
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> io::asynch::Read for EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            EspAsyncTls::read(self, buf).await.map_err(EspIOError)
        }
    }

    #[cfg(all(
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> io::asynch::Write for EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            EspAsyncTls::write(self, buf).await.map_err(EspIOError)
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[cfg(all(
        feature = "std",
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> futures_io::AsyncRead for EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        fn poll_read(
            self: Pin<&mut Self>,
            ctx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            self.as_ref()
                .poll_read(ctx, buf)
                .map_err(std::io::Error::other)
        }
    }

    #[cfg(all(
        feature = "std",
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> futures_io::AsyncRead for &EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        fn poll_read(
            self: Pin<&mut Self>,
            ctx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            self.as_ref()
                .poll_read(ctx, buf)
                .map_err(std::io::Error::other)
        }
    }

    #[cfg(all(
        feature = "std",
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> futures_io::AsyncWrite for EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        fn poll_write(
            self: Pin<&mut Self>,
            ctx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.as_ref()
                .poll_write(ctx, buf)
                .map_err(std::io::Error::other)
        }

        fn poll_flush(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[cfg(all(
        feature = "std",
        not(esp_idf_version_major = "4"),
        any(not(esp_idf_version_major = "5"), not(esp_idf_version_minor = "0"))
    ))]
    impl<S> futures_io::AsyncWrite for &EspAsyncTls<S>
    where
        S: PollableSocket,
    {
        fn poll_write(
            self: Pin<&mut Self>,
            ctx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.as_ref()
                .poll_write(ctx, buf)
                .map_err(std::io::Error::other)
        }

        fn poll_flush(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}
