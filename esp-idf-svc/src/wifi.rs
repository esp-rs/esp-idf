//! WiFi support
use core::marker::PhantomData;
use core::str::Utf8Error;
use core::time::Duration;
use core::{cmp, ffi, fmt, ops};

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;

use enumset::*;

use embedded_svc::wifi::Wifi;

use crate::hal::modem::WifiModemPeripheral;

use crate::sys::*;

use crate::eventloop::EspEventLoop;
use crate::eventloop::{
    EspEventDeserializer, EspEventSource, EspSubscription, EspSystemEventLoop, System, Wait,
};
use crate::handle::RawHandle;
#[cfg(esp_idf_comp_esp_netif_enabled)]
use crate::netif::*;
use crate::nvs::EspDefaultNvsPartition;
use crate::private::common::*;
use crate::private::cstr::*;
use crate::private::mutex;
#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
use crate::timer::EspTaskTimerService;

pub use embedded_svc::wifi::{
    AccessPointConfiguration, AccessPointInfo, AuthMethod, Capability, ClientConfiguration,
    Configuration, PmfConfiguration, Protocol, ScanMethod, ScanSortMethod, SecondaryChannel,
};

pub mod config {
    use core::time::Duration;

    use crate::sys::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ScanType {
        Active { min: Duration, max: Duration },
        Passive(Duration),
    }

    impl ScanType {
        pub const fn new() -> Self {
            Self::Active {
                min: Duration::from_secs(0),
                max: Duration::from_secs(0),
            }
        }
    }

    impl Default for ScanType {
        fn default() -> Self {
            Self::new()
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ScanConfig {
        pub bssid: Option<[u8; 6]>,
        pub ssid: Option<heapless::String<32>>,
        pub channel: Option<u8>,
        pub scan_type: ScanType,
        pub show_hidden: bool,
    }

    impl ScanConfig {
        pub const fn new() -> Self {
            Self {
                bssid: None,
                ssid: None,
                channel: None,
                scan_type: ScanType::new(),
                show_hidden: false,
            }
        }
    }

    impl Default for ScanConfig {
        fn default() -> Self {
            Self::new()
        }
    }

    impl From<&ScanConfig> for wifi_scan_config_t {
        fn from(s: &ScanConfig) -> Self {
            #[allow(clippy::needless_update)]
            Self {
                bssid: s.bssid.map_or(core::ptr::null(), |v| v.as_ptr()) as *mut u8,
                ssid: s.ssid.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()) as *mut u8,
                scan_time: wifi_scan_time_t {
                    active: wifi_active_scan_time_t {
                        min: match s.scan_type {
                            ScanType::Active { min, .. } => min.as_millis() as _,
                            _ => 0,
                        },
                        max: match s.scan_type {
                            ScanType::Active { max, .. } => max.as_millis() as _,
                            _ => 0,
                        },
                    },
                    passive: match s.scan_type {
                        ScanType::Passive(time) => time.as_millis() as _,
                        _ => 0,
                    },
                },
                channel: s.channel.unwrap_or_default(),
                scan_type: matches!(s.scan_type, ScanType::Passive { .. }).into(),
                show_hidden: s.show_hidden,
                ..Default::default()
            }
        }
    }
}

impl From<AuthMethod> for Newtype<wifi_auth_mode_t> {
    fn from(method: AuthMethod) -> Self {
        Newtype(match method {
            AuthMethod::None => wifi_auth_mode_t_WIFI_AUTH_OPEN,
            AuthMethod::WEP => wifi_auth_mode_t_WIFI_AUTH_WEP,
            AuthMethod::WPA => wifi_auth_mode_t_WIFI_AUTH_WPA_PSK,
            AuthMethod::WPA2Personal => wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK,
            AuthMethod::WPAWPA2Personal => wifi_auth_mode_t_WIFI_AUTH_WPA_WPA2_PSK,
            AuthMethod::WPA2Enterprise => wifi_auth_mode_t_WIFI_AUTH_WPA2_ENTERPRISE,
            AuthMethod::WPA3Personal => wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK,
            AuthMethod::WPA2WPA3Personal => wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK,
            AuthMethod::WAPIPersonal => wifi_auth_mode_t_WIFI_AUTH_WAPI_PSK,
        })
    }
}

impl From<Newtype<wifi_auth_mode_t>> for Option<AuthMethod> {
    #[allow(non_upper_case_globals)]
    #[allow(non_snake_case)]
    fn from(mode: Newtype<wifi_auth_mode_t>) -> Self {
        match mode.0 {
            wifi_auth_mode_t_WIFI_AUTH_OPEN => Some(AuthMethod::None),
            wifi_auth_mode_t_WIFI_AUTH_WEP => Some(AuthMethod::WEP),
            wifi_auth_mode_t_WIFI_AUTH_WPA_PSK => Some(AuthMethod::WPA),
            wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK => Some(AuthMethod::WPA2Personal),
            wifi_auth_mode_t_WIFI_AUTH_WPA_WPA2_PSK => Some(AuthMethod::WPAWPA2Personal),
            wifi_auth_mode_t_WIFI_AUTH_WPA2_ENTERPRISE => Some(AuthMethod::WPA2Enterprise),
            wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK => Some(AuthMethod::WPA3Personal),
            wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK => Some(AuthMethod::WPA2WPA3Personal),
            wifi_auth_mode_t_WIFI_AUTH_WAPI_PSK => Some(AuthMethod::WAPIPersonal),
            _ => None,
        }
    }
}

impl TryFrom<&ClientConfiguration> for Newtype<wifi_sta_config_t> {
    type Error = EspError;

    fn try_from(conf: &ClientConfiguration) -> Result<Self, Self::Error> {
        let bssid: [u8; 6] = match &conf.bssid {
            Some(bssid_ref) => *bssid_ref,
            None => [0; 6],
        };

        #[allow(clippy::needless_update)]
        let mut result = wifi_sta_config_t {
            ssid: [0; 32],
            password: [0; 64],
            scan_method: match conf.scan_method {
                ScanMethod::CompleteScan(_) => wifi_scan_method_t_WIFI_ALL_CHANNEL_SCAN,
                ScanMethod::FastScan => wifi_scan_method_t_WIFI_FAST_SCAN,
                _ => wifi_scan_method_t_WIFI_ALL_CHANNEL_SCAN,
            },
            bssid_set: conf.bssid.is_some(),
            bssid,
            channel: conf.channel.unwrap_or(0u8),
            listen_interval: 0,
            sort_method: match conf.scan_method {
                ScanMethod::CompleteScan(ScanSortMethod::Signal) => {
                    wifi_sort_method_t_WIFI_CONNECT_AP_BY_SIGNAL
                }
                ScanMethod::CompleteScan(ScanSortMethod::Security) => {
                    wifi_sort_method_t_WIFI_CONNECT_AP_BY_SECURITY
                }
                _ => wifi_sort_method_t_WIFI_CONNECT_AP_BY_SIGNAL,
            },
            threshold: wifi_scan_threshold_t {
                rssi: -127,
                authmode: Newtype::<wifi_auth_mode_t>::from(conf.auth_method).0,
                ..Default::default()
            },
            pmf_cfg: wifi_pmf_config_t {
                capable: false,
                required: false,
            },
            ..Default::default()
        };

        set_str_no_termination_requirement(&mut result.ssid, conf.ssid.as_ref())?;
        set_str_no_termination_requirement(&mut result.password, conf.password.as_ref())?;

        Ok(Newtype(result))
    }
}

impl From<Newtype<wifi_sta_config_t>> for ClientConfiguration {
    fn from(conf: Newtype<wifi_sta_config_t>) -> Self {
        Self {
            ssid: array_to_heapless_string(conf.0.ssid),
            bssid: if conf.0.bssid_set {
                Some(conf.0.bssid)
            } else {
                None
            },
            auth_method: Option::<AuthMethod>::from(Newtype(conf.0.threshold.authmode)).unwrap(),
            password: array_to_heapless_string(conf.0.password),
            channel: if conf.0.channel != 0 {
                Some(conf.0.channel)
            } else {
                None
            },
            #[allow(non_upper_case_globals)]
            #[allow(non_snake_case)]
            scan_method: match conf.0.scan_method {
                wifi_scan_method_t_WIFI_FAST_SCAN => ScanMethod::FastScan,
                wifi_scan_method_t_WIFI_ALL_CHANNEL_SCAN => match conf.0.sort_method {
                    wifi_sort_method_t_WIFI_CONNECT_AP_BY_SIGNAL => {
                        ScanMethod::CompleteScan(ScanSortMethod::Signal)
                    }
                    wifi_sort_method_t_WIFI_CONNECT_AP_BY_SECURITY => {
                        ScanMethod::CompleteScan(ScanSortMethod::Security)
                    }
                    _ => ScanMethod::default(),
                },
                _ => ScanMethod::default(),
            },
            pmf_cfg: match conf.0.pmf_cfg {
                wifi_pmf_config_t {
                    capable: false,
                    required: _,
                } => PmfConfiguration::NotCapable,
                wifi_pmf_config_t {
                    capable: true,
                    required,
                } => PmfConfiguration::Capable { required },
            },
        }
    }
}

impl TryFrom<&AccessPointConfiguration> for Newtype<wifi_ap_config_t> {
    type Error = EspError;

    fn try_from(conf: &AccessPointConfiguration) -> Result<Self, Self::Error> {
        let mut result = wifi_ap_config_t {
            ssid: [0; 32],
            password: [0; 64],
            ssid_len: conf.ssid.len() as u8,
            channel: conf.channel,
            authmode: Newtype::<wifi_auth_mode_t>::from(conf.auth_method).0,
            ssid_hidden: u8::from(conf.ssid_hidden),
            max_connection: cmp::min(conf.max_connections, 16) as u8,
            beacon_interval: 100,
            ..Default::default()
        };

        set_str(&mut result.ssid, conf.ssid.as_ref())?;
        set_str(&mut result.password, conf.password.as_ref())?;

        Ok(Newtype(result))
    }
}

impl From<Newtype<wifi_ap_config_t>> for AccessPointConfiguration {
    fn from(conf: Newtype<wifi_ap_config_t>) -> Self {
        Self {
            ssid: if conf.0.ssid_len == 0 {
                Default::default()
            } else {
                unsafe {
                    core::str::from_utf8_unchecked(&conf.0.ssid[0..conf.0.ssid_len as usize])
                        .try_into()
                        .unwrap()
                }
            },
            ssid_hidden: conf.0.ssid_hidden != 0,
            channel: conf.0.channel,
            secondary_channel: None,
            auth_method: Option::<AuthMethod>::from(Newtype(conf.0.authmode)).unwrap(),
            protocols: EnumSet::<Protocol>::empty(), // TODO
            password: array_to_heapless_string(conf.0.password),
            max_connections: conf.0.max_connection as u16,
        }
    }
}

impl TryFrom<Newtype<&wifi_ap_record_t>> for AccessPointInfo {
    type Error = Utf8Error;

    #[allow(non_upper_case_globals)]
    #[allow(non_snake_case)]
    fn try_from(ap_info: Newtype<&wifi_ap_record_t>) -> Result<Self, Self::Error> {
        let a = ap_info.0;

        Ok(Self {
            ssid: from_cstr_fallible(&a.ssid)?.try_into().unwrap(),
            bssid: a.bssid,
            channel: a.primary,
            secondary_channel: match a.second {
                wifi_second_chan_t_WIFI_SECOND_CHAN_NONE => SecondaryChannel::None,
                wifi_second_chan_t_WIFI_SECOND_CHAN_ABOVE => SecondaryChannel::Above,
                wifi_second_chan_t_WIFI_SECOND_CHAN_BELOW => SecondaryChannel::Below,
                _ => panic!(),
            },
            signal_strength: a.rssi,
            protocols: EnumSet::<Protocol>::empty(), // TODO
            auth_method: Option::<AuthMethod>::from(Newtype::<wifi_auth_mode_t>(a.authmode)),
        })
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WifiDeviceId {
    Ap,
    Sta,
}

impl From<WifiDeviceId> for wifi_interface_t {
    fn from(id: WifiDeviceId) -> Self {
        match id {
            WifiDeviceId::Ap => wifi_interface_t_WIFI_IF_AP,
            WifiDeviceId::Sta => wifi_interface_t_WIFI_IF_STA,
        }
    }
}

#[allow(non_upper_case_globals)]
impl From<wifi_interface_t> for WifiDeviceId {
    fn from(id: wifi_interface_t) -> Self {
        match id {
            wifi_interface_t_WIFI_IF_AP => WifiDeviceId::Ap,
            wifi_interface_t_WIFI_IF_STA => WifiDeviceId::Sta,
            _ => unreachable!(),
        }
    }
}

extern "C" {
    fn esp_wifi_internal_reg_rxcb(
        ifx: wifi_interface_t,
        rxcb: Option<
            unsafe extern "C" fn(
                buffer: *mut ffi::c_void,
                len: u16,
                eb: *mut ffi::c_void,
            ) -> esp_err_t,
        >,
    ) -> esp_err_t;

    fn esp_wifi_internal_free_rx_buffer(buffer: *mut ffi::c_void);

    fn esp_wifi_internal_tx(
        wifi_if: wifi_interface_t,
        buffer: *mut ffi::c_void,
        len: u16,
    ) -> esp_err_t;
}

#[allow(clippy::type_complexity)]
static mut RX_CALLBACK: Option<
    Box<dyn FnMut(WifiDeviceId, WifiFrame) -> Result<(), EspError> + 'static>,
> = None;
#[allow(clippy::type_complexity)]
static mut TX_CALLBACK: Option<Box<dyn FnMut(WifiDeviceId, &[u8], bool) + 'static>> = None;

pub trait NonBlocking {
    fn is_scan_done(&self) -> Result<bool, EspError>;

    fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError>;

    fn stop_scan(&mut self) -> Result<(), EspError>;

    fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError>;

    #[cfg(feature = "alloc")]
    fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError>;

    fn start_wps(&mut self, config: &WpsConfig) -> Result<(), EspError>;

    fn stop_wps(&mut self) -> Result<WpsStatus, EspError>;

    fn is_wps_finished(&self) -> Result<bool, EspError>;
}

impl<T> NonBlocking for &mut T
where
    T: NonBlocking,
{
    fn is_scan_done(&self) -> Result<bool, EspError> {
        (**self).is_scan_done()
    }

    fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        (**self).start_scan(scan_config, blocking)
    }

    fn stop_scan(&mut self) -> Result<(), EspError> {
        (**self).stop_scan()
    }

    fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        (**self).get_scan_result_n()
    }

    #[cfg(feature = "alloc")]
    fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        (**self).get_scan_result()
    }

    fn start_wps(&mut self, config: &WpsConfig) -> Result<(), EspError> {
        (**self).start_wps(config)
    }

    fn stop_wps(&mut self) -> Result<WpsStatus, EspError> {
        (**self).stop_wps()
    }

    fn is_wps_finished(&self) -> Result<bool, EspError> {
        (**self).is_wps_finished()
    }
}

/// This struct provides a safe wrapper over the ESP IDF Wifi C driver.
///
/// The driver works on Layer 2 (Data Link) in the OSI model, in that it provides
/// facilities for sending and receiving ethernet packets over the WiFi radio.
///
/// For most use cases, utilizing `EspWifi` - which provides a networking (IP)
/// layer as well - should be preferred. Using `WifiDriver` directly is beneficial
/// only when one would like to utilize a custom, non-STD network stack like `smoltcp`.
pub struct WifiDriver<'d> {
    status: Arc<mutex::Mutex<WifiDriverStatus>>,
    _subscription: EspSubscription<'static, System>,
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    _nvs: Option<EspDefaultNvsPartition>,
    _p: PhantomData<&'d mut ()>,
}

#[derive(Clone, Debug)]
struct WifiDriverStatus {
    pub sta: WifiStaStatus,
    pub scan: WifiScanStatus,
    pub ap: WifiApStatus,
    pub wps: Option<WpsStatus>,
}

impl<'d> WifiDriver<'d> {
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    pub fn new<M: WifiModemPeripheral + 'd>(
        _modem: M,
        sysloop: EspSystemEventLoop,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Self::init(nvs.is_some())?;

        let (status, subscription) = Self::subscribe(&sysloop)?;

        Ok(Self {
            status,
            _subscription: subscription,
            _nvs: nvs,
            _p: PhantomData,
        })
    }

    #[cfg(not(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled)))]
    pub fn new<M: WifiModemPeripheral + 'd>(
        _modem: M,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        Self::init(false)?;

        let (status, subscription) = Self::subscribe(&sysloop)?;

        Ok(Self {
            status,
            _subscription: subscription,
            _p: PhantomData,
        })
    }

    #[allow(clippy::type_complexity)]
    fn subscribe(
        sysloop: &EspEventLoop<System>,
    ) -> Result<
        (
            Arc<mutex::Mutex<WifiDriverStatus>>,
            EspSubscription<'static, System>,
        ),
        EspError,
    > {
        let status = Arc::new(mutex::Mutex::new(WifiDriverStatus {
            sta: WifiStaStatus::Stopped,
            ap: WifiApStatus::Stopped,
            scan: WifiScanStatus::Idle,
            wps: None,
        }));
        let s_status = status.clone();

        let subscription = sysloop.subscribe::<WifiEvent, _>(move |event: WifiEvent| {
            let mut guard = s_status.lock();

            match event {
                WifiEvent::ApStarted => guard.ap = WifiApStatus::Started,
                WifiEvent::ApStopped => guard.ap = WifiApStatus::Stopped,
                WifiEvent::StaStarted => guard.sta = WifiStaStatus::Started,
                WifiEvent::StaStopped => guard.sta = WifiStaStatus::Stopped,
                WifiEvent::StaConnected(_) => guard.sta = WifiStaStatus::Connected,
                WifiEvent::StaDisconnected(_) => guard.sta = WifiStaStatus::Started,
                WifiEvent::ScanDone(_) => guard.scan = WifiScanStatus::Done,
                WifiEvent::StaWpsSuccess(_)
                | WifiEvent::StaWpsFailed
                | WifiEvent::StaWpsTimeout
                | WifiEvent::StaWpsPin(_)
                | WifiEvent::StaWpsPbcOverlap => guard.wps = Some((&event).try_into().unwrap()),
                _ => (),
            };
        })?;

        Ok((status, subscription))
    }

    fn init(nvs_enabled: bool) -> Result<(), EspError> {
        #[allow(clippy::needless_update)]
        #[allow(unused_unsafe)]
        let cfg = wifi_init_config_t {
            #[cfg(esp_idf_version_major = "4")]
            event_handler: Some(esp_event_send_internal),
            osi_funcs: unsafe { core::ptr::addr_of_mut!(g_wifi_osi_funcs) },
            wpa_crypto_funcs: unsafe { g_wifi_default_wpa_crypto_funcs },
            static_rx_buf_num: CONFIG_ESP32_WIFI_STATIC_RX_BUFFER_NUM as _,
            dynamic_rx_buf_num: CONFIG_ESP32_WIFI_DYNAMIC_RX_BUFFER_NUM as _,
            tx_buf_type: CONFIG_ESP32_WIFI_TX_BUFFER_TYPE as _,
            static_tx_buf_num: WIFI_STATIC_TX_BUFFER_NUM as _,
            dynamic_tx_buf_num: WIFI_DYNAMIC_TX_BUFFER_NUM as _,
            rx_mgmt_buf_type: CONFIG_ESP_WIFI_DYNAMIC_RX_MGMT_BUF as _,
            rx_mgmt_buf_num: WIFI_RX_MGMT_BUF_NUM_DEF as _,
            cache_tx_buf_num: WIFI_CACHE_TX_BUFFER_NUM as _,
            csi_enable: WIFI_CSI_ENABLED as _,
            ampdu_rx_enable: WIFI_AMPDU_RX_ENABLED as _,
            ampdu_tx_enable: WIFI_AMPDU_TX_ENABLED as _,
            amsdu_tx_enable: WIFI_AMSDU_TX_ENABLED as _,
            nvs_enable: i32::from(nvs_enabled),
            nano_enable: WIFI_NANO_FORMAT_ENABLED as _,
            rx_ba_win: WIFI_DEFAULT_RX_BA_WIN as _,
            wifi_task_core_id: WIFI_TASK_CORE_ID as _,
            beacon_max_len: WIFI_SOFTAP_BEACON_MAX_LEN as _,
            mgmt_sbuf_num: WIFI_MGMT_SBUF_NUM as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                esp_idf_version_full = "5.1.0",
                esp_idf_version_full = "5.1.1",
                esp_idf_version_full = "5.1.2"
            ))]
            feature_caps: unsafe { g_wifi_feature_caps },
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
                esp_idf_version_full = "5.1.0",
                esp_idf_version_full = "5.1.1",
                esp_idf_version_full = "5.1.2"
            )))]
            feature_caps: WIFI_FEATURE_CAPS as _,
            sta_disconnected_pm: WIFI_STA_DISCONNECTED_PM_ENABLED != 0,
            // Available since ESP IDF V4.4.4+
            #[cfg(any(
                not(esp_idf_version_major = "4"),
                all(
                    esp_idf_version_major = "4",
                    any(
                        not(esp_idf_version_minor = "4"),
                        all(
                            not(esp_idf_version_patch = "0"),
                            not(esp_idf_version_patch = "1"),
                            not(esp_idf_version_patch = "2"),
                            not(esp_idf_version_patch = "3")
                        )
                    )
                )
            ))]
            espnow_max_encrypt_num: CONFIG_ESP_WIFI_ESPNOW_MAX_ENCRYPT_NUM as i32,
            magic: WIFI_INIT_CONFIG_MAGIC as _,
            // Available since ESP IDF V5.3.0
            #[cfg(any(
                all(not(esp_idf_version_major = "4"), not(esp_idf_version_major = "5")),
                all(
                    esp_idf_version_major = "5",
                    not(esp_idf_version = "5.0"),
                    not(esp_idf_version = "5.1"),
                    not(esp_idf_version = "5.2"),
                ),
            ))]
            tx_hetb_queue_num: WIFI_TX_HETB_QUEUE_NUM as _,
            // Available since ESP IDF V5.3.0
            #[cfg(any(
                all(not(esp_idf_version_major = "4"), not(esp_idf_version_major = "5")),
                all(
                    esp_idf_version_major = "5",
                    not(esp_idf_version = "5.0"),
                    not(esp_idf_version = "5.1"),
                    not(esp_idf_version = "5.2"),
                ),
            ))]
            dump_hesigb_enable: WIFI_DUMP_HESIGB_ENABLED != 0,
            // Available since ESP IDF V5.5.6 and V6.1.1
            #[cfg(any(
                esp_idf_version_patch_at_least_5_5_6,
                esp_idf_version_at_least_6_1_1,
            ))]
            wifi_task_stack_size: WIFI_TASK_STACK_SIZE as _,
            ..Default::default()
        };
        esp!(unsafe { esp_wifi_init(&cfg) })?;

        ::log::debug!("Driver initialized");

        Ok(())
    }

    /// Returns the set of [`Capabilities`] for this driver. In `esp-idf`, all
    /// drivers always have Client, AP and Mixed capabilities.
    pub fn get_capabilities(&self) -> Result<EnumSet<Capability>, EspError> {
        let caps = Capability::Client | Capability::AccessPoint | Capability::Mixed;

        ::log::debug!("Providing capabilities: {caps:?}");

        Ok(caps)
    }

    /// As per [`crate::sys::esp_wifi_start`](crate::sys::esp_wifi_start)
    pub fn start(&mut self) -> Result<(), EspError> {
        ::log::debug!("Start requested");

        esp!(unsafe { esp_wifi_start() })?;

        ::log::debug!("Starting");

        Ok(())
    }

    /// As per [`crate::sys::esp_wifi_stop`](crate::sys::esp_wifi_stop)
    pub fn stop(&mut self) -> Result<(), EspError> {
        ::log::debug!("Stop requested");

        esp!(unsafe { esp_wifi_stop() })?;

        ::log::debug!("Stopping");

        Ok(())
    }

    /// As per [`crate::sys::esp_wifi_connect`](crate::sys::esp_wifi_connect)
    pub fn connect(&mut self) -> Result<(), EspError> {
        ::log::debug!("Connect requested");

        esp!(unsafe { esp_wifi_connect() })?;

        ::log::debug!("Connecting");

        Ok(())
    }

    /// As per [`crate::sys::esp_wifi_disconnect`](crate::sys::esp_wifi_disconnect)
    pub fn disconnect(&mut self) -> Result<(), EspError> {
        ::log::debug!("Disconnect requested");

        esp!(unsafe { esp_wifi_disconnect() })?;

        ::log::debug!("Disconnecting");

        Ok(())
    }

    /// Returns `true` if the driver is in Access Point (AP) mode, as reported by
    /// [`crate::sys::esp_wifi_get_mode`](crate::sys::esp_wifi_get_mode)
    pub fn is_ap_enabled(&self) -> Result<bool, EspError> {
        let mut mode: wifi_mode_t = 0;
        esp!(unsafe { esp_wifi_get_mode(&mut mode) })?;

        Ok(mode == wifi_mode_t_WIFI_MODE_AP || mode == wifi_mode_t_WIFI_MODE_APSTA)
    }

    /// Returns `true` if the driver is in Client (station or STA) mode, as
    /// reported by [`crate::sys::esp_wifi_get_mode`](crate::sys::esp_wifi_get_mode)
    pub fn is_sta_enabled(&self) -> Result<bool, EspError> {
        let mut mode: wifi_mode_t = 0;
        esp!(unsafe { esp_wifi_get_mode(&mut mode) })?;

        Ok(mode == wifi_mode_t_WIFI_MODE_STA || mode == wifi_mode_t_WIFI_MODE_APSTA)
    }

    pub fn is_ap_started(&self) -> Result<bool, EspError> {
        Ok(matches!(self.status.lock().ap, WifiApStatus::Started))
    }

    pub fn is_sta_started(&self) -> Result<bool, EspError> {
        let guard = self.status.lock();

        Ok(matches!(
            guard.sta,
            WifiStaStatus::Started | WifiStaStatus::Connected
        ))
    }

    pub fn is_sta_connected(&self) -> Result<bool, EspError> {
        Ok(matches!(self.status.lock().sta, WifiStaStatus::Connected))
    }

    pub fn is_started(&self) -> Result<bool, EspError> {
        let ap_enabled = self.is_ap_enabled()?;
        let sta_enabled = self.is_sta_enabled()?;

        if !ap_enabled && !sta_enabled {
            Ok(false)
        } else {
            Ok(
                (!ap_enabled || self.is_ap_started()?)
                    && (!sta_enabled || self.is_sta_started()?),
            )
        }
    }

    pub fn is_connected(&self) -> Result<bool, EspError> {
        let ap_enabled = self.is_ap_enabled()?;
        let sta_enabled = self.is_sta_enabled()?;

        if !ap_enabled && !sta_enabled {
            Ok(false)
        } else {
            let guard = self.status.lock();

            Ok((!ap_enabled || matches!(guard.ap, WifiApStatus::Started))
                && (!sta_enabled || matches!(guard.sta, WifiStaStatus::Connected)))
        }
    }

    pub fn is_scan_done(&self) -> Result<bool, EspError> {
        let guard = self.status.lock();

        Ok(matches!(guard.scan, WifiScanStatus::Done))
    }

    #[allow(non_upper_case_globals)]
    #[allow(non_snake_case)]
    /// Returns the <`Configuration`> currently in use
    pub fn get_configuration(&self) -> Result<Configuration, EspError> {
        ::log::debug!("Getting configuration");

        let mut mode: wifi_mode_t = 0;
        esp!(unsafe { esp_wifi_get_mode(&mut mode) })?;

        let conf = match mode {
            wifi_mode_t_WIFI_MODE_NULL => Configuration::None,
            wifi_mode_t_WIFI_MODE_AP => Configuration::AccessPoint(self.get_ap_conf()?),
            wifi_mode_t_WIFI_MODE_STA => Configuration::Client(self.get_sta_conf()?),
            wifi_mode_t_WIFI_MODE_APSTA => {
                Configuration::Mixed(self.get_sta_conf()?, self.get_ap_conf()?)
            }
            _ => panic!(),
        };

        ::log::debug!("Configuration gotten: {:?}", conf);

        Ok(conf)
    }

    /// Sets the <`Configuration`> (SSID, channel, etc). This also defines whether
    /// the driver will work in AP mode, client mode, client+AP mode, or none.
    ///
    /// Calls [`crate::sys::esp_wifi_set_mode`](crate::sys::esp_wifi_set_mode)
    /// and [`crate::sys::esp_wifi_set_config`](crate::sys::esp_wifi_set_config)
    pub fn set_configuration(&mut self, conf: &Configuration) -> Result<(), EspError> {
        ::log::debug!("Setting configuration: {conf:?}");

        match conf {
            Configuration::None => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_NULL))?;
                }
                ::log::debug!("Wifi mode NULL set");
            }
            Configuration::AccessPoint(ap_conf) => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_AP))?;
                }
                ::log::debug!("Wifi mode AP set");

                self.set_ap_conf(ap_conf)?;
            }
            Configuration::Client(client_conf) => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA))?;
                }
                ::log::debug!("Wifi mode STA set");

                self.set_sta_conf(client_conf)?;
            }
            Configuration::Mixed(client_conf, ap_conf) => {
                unsafe {
                    esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_APSTA))?;
                }
                ::log::debug!("Wifi mode APSTA set");

                self.set_sta_conf(client_conf)?;
                self.set_ap_conf(ap_conf)?;
            }
        }

        ::log::debug!("Configuration set");

        Ok(())
    }

    /// Scan for nearby, visible access points.
    ///
    /// It scans for all available access points nearby, but returns only the first `N` access points found.
    /// In addition, it returns the actual amount it found. The function blocks until the scan is done.
    ///
    /// Before calling this function the Wifi driver must be configured and started in either Client or Mixed mode.
    ///
    /// # Example
    /// ```ignore
    /// let mut wifi_driver = WifiDriver::new(peripherals.modem, sysloop.clone());
    /// wifi_driver.set_configuration(
    ///     &Configuration::Client(ClientConfiguration::default())
    /// )
    /// .unwrap();
    /// wifi_driver.start().unwrap();
    ///
    /// let (scan_result, found_aps) = wifi_driver.scan_n::<10>().unwrap();
    /// ```
    // For backwards compatibility
    pub fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.start_scan(&Default::default(), true)?;
        self.get_scan_result_n()
    }

    /// Scan for nearby, visible access points.
    ///
    /// Unlike [`WifiDriver::scan_n()`], it returns all found access points by allocating memory
    /// dynamically.
    ///
    /// For more details see [`WifiDriver::scan_n()`].
    // For backwards compatibility
    #[cfg(feature = "alloc")]
    pub fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.start_scan(&Default::default(), true)?;
        self.get_scan_result()
    }

    /// Start scanning for nearby, visible access points.
    ///
    /// Unlike [`WifiDriver::scan_n()`] or [`WifiDriver::scan()`] it can be called as either blocking or not blocking.
    /// A [`ScanConfig`] can be provided as well. To get the scan result call either [`WifiDriver::get_scan_result_n()`] or
    /// [`WifiDriver::get_scan_result()`].
    ///
    /// This function can be used in `async` context, when the current thread shouldn't be blocked.
    ///
    /// # Example
    ///
    /// This example shows how to use it in a `async` context.
    ///
    /// ```ignore
    /// let mut wifi_driver = WifiDriver::new(peripherals.modem, sysloop.clone());
    /// wifi_driver.set_configuration(
    ///     &Configuration::Client(ClientConfiguration::default())
    /// )
    /// .unwrap();
    /// wifi_driver.start().unwrap();
    ///
    /// let scan_finish_signal = Arc::new(channel_bridge::notification::Notification::new());
    /// let _sub = {
    ///     let scan_finish_signal = scan_finish_signal.clone();
    ///     sysloop.subscribe::<WifiEvent>(move |event| {
    ///         if *event == WifiEvent::ScanDone {
    ///             scan_finish_signal.notify();
    ///         }
    ///     }).unwrap()
    /// };
    ///
    /// wifi_driver.start_scan(&ScanConfig::default(), false).unwrap();
    ///
    /// scan_finish_signal.wait().await;
    ///
    /// let res = wifi_driver.get_scan_result().unwrap();
    /// ```
    pub fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        ::log::debug!("About to scan for access points");

        let scan_config: wifi_scan_config_t = scan_config.into();
        esp!(unsafe { esp_wifi_scan_start(&scan_config as *const wifi_scan_config_t, blocking) })?;

        self.status.lock().scan = WifiScanStatus::Started;

        Ok(())
    }

    /// Stops a previous started access point scan.
    pub fn stop_scan(&mut self) -> Result<(), EspError> {
        ::log::debug!("About to stop scan for access points");

        esp!(unsafe { esp_wifi_scan_stop() })?;

        self.status.lock().scan = WifiScanStatus::Idle;

        Ok(())
    }

    /// Get the results of an access point scan.
    ///
    /// This call returns a list of the first `N` found access points. A scan can be started with [`WifiDriver::start_scan()`].
    /// As [`WifiDriver::scan_n()`] it returns the actual amount of found access points as well.
    pub fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        let scanned_count = self.get_scan_count()?;

        let mut ap_infos_raw: heapless::Vec<wifi_ap_record_t, N> = heapless::Vec::new();
        unsafe {
            ap_infos_raw.set_len(scanned_count.min(N));
        }

        let fetched_count = self.fetch_scan_result(&mut ap_infos_raw)?;

        let result = ap_infos_raw[..fetched_count]
            .iter()
            .map::<Result<AccessPointInfo, Utf8Error>, _>(|ap_info_raw| {
                Newtype(ap_info_raw).try_into()
            })
            .filter_map(|r| r.ok())
            .inspect(|ap_info| ::log::debug!("Found access point {ap_info:?}"))
            .collect();

        Ok((result, scanned_count))
    }

    /// Get the results of an access point scan.
    ///
    /// Unlike [`WifiDriver::get_scan_result_n()`], it returns all found access points by allocating memory
    /// dynamically.
    ///
    /// For more details see [`WifiDriver::get_scan_result_n()`].
    #[cfg(feature = "alloc")]
    pub fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        let scanned_count = self.get_scan_count()?;

        let mut ap_infos_raw: alloc::vec::Vec<wifi_ap_record_t> =
            alloc::vec::Vec::with_capacity(scanned_count);
        #[allow(clippy::uninit_vec)]
        // ... because we are filling it in on the next line and only reading the initialized members
        unsafe {
            ap_infos_raw.set_len(scanned_count)
        };

        let fetched_count = self.fetch_scan_result(&mut ap_infos_raw)?;

        let result = ap_infos_raw[..fetched_count]
            .iter()
            .map::<Result<AccessPointInfo, Utf8Error>, _>(|ap_info_raw| {
                Newtype(ap_info_raw).try_into()
            })
            .filter_map(|r| r.ok())
            .inspect(|ap_info| ::log::debug!("Found access point {ap_info:?}"))
            .collect();

        Ok(result)
    }

    /// Sets callback functions for receiving and sending data, as per
    /// [`crate::sys::esp_wifi_internal_reg_rxcb`](crate::sys::esp_wifi_internal_reg_rxcb) and
    /// [`crate::sys::esp_wifi_set_tx_done_cb`](crate::sys::esp_wifi_set_tx_done_cb)
    pub fn set_callbacks<R, T>(&mut self, rx_callback: R, tx_callback: T) -> Result<(), EspError>
    where
        R: FnMut(WifiDeviceId, WifiFrame) -> Result<(), EspError> + Send + 'static,
        T: FnMut(WifiDeviceId, &[u8], bool) + Send + 'static,
    {
        self.internal_set_callbacks(rx_callback, tx_callback)
    }

    /// Sets callback functions for receiving and sending data, as per
    /// [`crate::sys::esp_wifi_internal_reg_rxcb`](crate::sys::esp_wifi_internal_reg_rxcb) and
    /// [`crate::sys::esp_wifi_set_tx_done_cb`](crate::sys::esp_wifi_set_tx_done_cb)
    ///
    /// # Safety
    ///
    /// This method - in contrast to method `set_callbacks` - allows the user to pass
    /// non-static callbacks/closures. This enables users to borrow
    /// - in the closure - variables that live on the stack - or more generally - in the same
    ///   scope where the service is created.
    ///
    /// HOWEVER: care should be taken NOT to call `core::mem::forget()` on the service,
    /// as that would immediately lead to an UB (crash).
    /// Also note that forgetting the service might happen with `Rc` and `Arc`
    /// when circular references are introduced: <https://github.com/rust-lang/rust/issues/24456>
    ///
    /// The reason is that the closure is actually sent to a hidden ESP IDF thread.
    /// This means that if the service is forgotten, Rust is free to e.g. unwind the stack
    /// and the closure now owned by this other thread will end up with references to variables that no longer exist.
    ///
    /// The destructor of the service takes care - prior to the service being dropped and e.g.
    /// the stack being unwind - to remove the closure from the hidden thread and destroy it.
    /// Unfortunately, when the service is forgotten, the un-subscription does not happen
    /// and invalid references are left dangling.
    ///
    /// This "local borrowing" will only be possible to express in a safe way once/if `!Leak` types
    /// are introduced to Rust (i.e. the impossibility to "forget" a type and thus not call its destructor).
    pub unsafe fn set_nonstatic_callbacks<R, T>(
        &mut self,
        rx_callback: R,
        tx_callback: T,
    ) -> Result<(), EspError>
    where
        R: FnMut(WifiDeviceId, WifiFrame) -> Result<(), EspError> + Send + 'd,
        T: FnMut(WifiDeviceId, &[u8], bool) + Send + 'd,
    {
        self.internal_set_callbacks(rx_callback, tx_callback)
    }

    fn internal_set_callbacks<R, T>(
        &mut self,
        mut rx_callback: R,
        mut tx_callback: T,
    ) -> Result<(), EspError>
    where
        R: FnMut(WifiDeviceId, WifiFrame) -> Result<(), EspError> + Send + 'd,
        T: FnMut(WifiDeviceId, &[u8], bool) + Send + 'd,
    {
        let _ = self.disconnect();
        let _ = self.stop();

        #[allow(clippy::type_complexity)]
        let rx_callback: Box<
            Box<dyn FnMut(WifiDeviceId, WifiFrame) -> Result<(), EspError> + Send + 'd>,
        > = Box::new(Box::new(move |device_id, data| {
            rx_callback(device_id, data)
        }));

        #[allow(clippy::type_complexity)]
        let tx_callback: Box<Box<dyn FnMut(WifiDeviceId, &[u8], bool) + Send + 'd>> =
            Box::new(Box::new(move |device_id, data, status| {
                tx_callback(device_id, data, status)
            }));

        #[allow(clippy::type_complexity)]
        let rx_callback: Box<
            Box<dyn FnMut(WifiDeviceId, WifiFrame) -> Result<(), EspError> + Send + 'static>,
        > = unsafe { core::mem::transmute(rx_callback) };

        #[allow(clippy::type_complexity)]
        let tx_callback: Box<Box<dyn FnMut(WifiDeviceId, &[u8], bool) + Send + 'static>> =
            unsafe { core::mem::transmute(tx_callback) };

        unsafe {
            RX_CALLBACK = Some(rx_callback);
            TX_CALLBACK = Some(tx_callback);

            esp!(esp_wifi_internal_reg_rxcb(
                WifiDeviceId::Ap.into(),
                Some(Self::handle_rx_ap),
            ))?;

            esp!(esp_wifi_internal_reg_rxcb(
                WifiDeviceId::Sta.into(),
                Some(Self::handle_rx_sta),
            ))?;

            esp!(esp_wifi_set_tx_done_cb(Some(Self::handle_tx)))?;
        }

        Ok(())
    }

    /// As per [`crate::sys::esp_wifi_internal_tx`](crate::sys::esp_wifi_internal_tx)
    pub fn send(&mut self, device_id: WifiDeviceId, frame: &[u8]) -> Result<(), EspError> {
        esp!(unsafe {
            esp_wifi_internal_tx(device_id.into(), frame.as_ptr() as *mut _, frame.len() as _)
        })
    }

    /// Get information about the AP with which the station is associated.
    /// Useful to get the current signal strength of the AP.
    pub fn get_ap_info(&self) -> Result<AccessPointInfo, EspError> {
        let mut ap_info_raw: wifi_ap_record_t = wifi_ap_record_t::default();
        // If STA is not connected EspError(12303) is returned
        esp!(unsafe { esp_wifi_sta_get_ap_info(&mut ap_info_raw) })?;
        let ap_info: AccessPointInfo = Newtype(&ap_info_raw).try_into().unwrap();

        ::log::debug!("AP Info: {ap_info:?}");
        Ok(ap_info)
    }

    /// Set RSSI threshold below which APP will get an WifiEvent::StaBssRssiLow,
    /// as per [`crate::sys::esp_wifi_set_rssi_threshold`](crate::sys::esp_wifi_set_rssi_threshold)
    /// `rssi_threshold`: threshold value in dbm between -100 to 0
    ///
    /// # Example
    ///
    /// This example shows how to use it in a `async` context.
    ///
    /// ```ignore
    /// let mut wifi_driver = WifiDriver::new(peripherals.modem, sysloop.clone());
    /// wifi_driver.set_configuration(
    ///     &Configuration::Client(ClientConfiguration::default())
    /// )
    /// .unwrap();
    /// wifi_driver.start().unwrap();
    /// wifi_driver.connect().unwrap();
    ///
    /// let rssi_low = -40;
    /// wifi_driver.driver_mut().set_sta_rssi_low(rssi_low).unwrap();
    ///
    /// // Subscribe to RSSI events.
    /// let rssi_low_signal = Arc::new(channel_bridge::notification::Notification::new());
    /// let _sub = {
    ///     let rssi_low_signal = rssi_low_signal.clone();
    ///     sysloop.subscribe::<WifiEvent>(move |event| {
    ///         if *event == WifiEvent::StaBssRssiLow {
    ///             rssi_low_signal.notify();
    ///         }
    ///     }).unwrap()
    /// };
    ///
    /// rssi_low_signal.wait().await;
    /// // do stuff with the information
    ///
    /// // set_rssi_threshold() has to be called again after every StaBssRssiLow event received.
    /// ```
    pub fn set_rssi_threshold(&mut self, rssi_threshold: i8) -> Result<(), EspError> {
        esp!(unsafe { esp_wifi_set_rssi_threshold(rssi_threshold.into()) })
    }

    /// Returns the MAC address of the interface, as per
    /// [`crate::sys::esp_wifi_get_mac`](crate::sys::esp_wifi_get_mac)
    pub fn get_mac(&self, interface: WifiDeviceId) -> Result<[u8; 6], EspError> {
        let mut mac = [0u8; 6];

        esp!(unsafe { esp_wifi_get_mac(interface.into(), mac.as_mut_ptr() as *mut _) })?;

        Ok(mac)
    }

    /// Seta the MAC address of the interface, as per
    /// [`crate::sys::esp_wifi_set_mac`](crate::sys::esp_wifi_set_mac)
    pub fn set_mac(&mut self, interface: WifiDeviceId, mac: [u8; 6]) -> Result<(), EspError> {
        esp!(unsafe { esp_wifi_set_mac(interface.into(), mac.as_ptr() as *mut _) })
    }

    /// Enable and start WPS
    pub fn start_wps(&mut self, config: &WpsConfig) -> Result<(), EspError> {
        let config = Newtype::<esp_wps_config_t>::try_from(config)?;

        match self.get_configuration()? {
            Configuration::None => esp!(unsafe { esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA) })?,
            Configuration::AccessPoint(_) => {
                esp!(unsafe { esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_APSTA) })?
            }
            _ => (),
        }

        esp!(unsafe { esp_wifi_wps_enable(&config.0 as *const _) })?;
        #[cfg(not(esp_idf_version_at_least_6_0_0))]
        esp!(unsafe { esp_wifi_wps_start(0) })?;
        #[cfg(esp_idf_version_at_least_6_0_0)]
        esp!(unsafe { esp_wifi_wps_start() })?;

        self.status.lock().wps = None;

        Ok(())
    }

    /// Enables or disables promiscuous mode for the [`WifiDriver`].
    ///
    /// When promiscuous mode is enabled, the driver captures all Wifi frames
    /// on the network, regardless of their destination MAC address. This is useful for
    /// debugging or monitoring purposes.
    pub fn set_promiscuous(&mut self, state: bool) -> Result<(), EspError> {
        esp!(unsafe { esp_wifi_set_promiscuous(state) })?;

        if state {
            ::log::info!("Driver set in promiscuous mode");
        } else {
            ::log::info!("Driver set in non-promiscuous mode");
        }

        Ok(())
    }

    /// Gets the state of the promiscuous mode for the [`WifiDriver`].
    pub fn is_promiscuous(&self) -> Result<bool, EspError> {
        let mut en: bool = false;

        esp!(unsafe { esp_wifi_get_promiscuous(&mut en) })?;

        Ok(en)
    }

    //TODO: add safe wrappers for these three functions
    //https://docs.rs/esp-idf-sys/latest/esp_idf_sys/fn.esp_wifi_set_promiscuous_ctrl_filter.html
    //https://docs.rs/esp-idf-sys/latest/esp_idf_sys/fn.esp_wifi_set_promiscuous_filter.html
    //https://docs.rs/esp-idf-sys/latest/esp_idf_sys/fn.esp_wifi_set_promiscuous_rx_cb.html

    /// Gets the WPS status as a [`WPS Event`] and disables WPS.
    fn stop_wps(&mut self) -> Result<WpsStatus, EspError> {
        let mut status = self.status.lock();
        if let Some(status) = status.wps.take() {
            esp!(unsafe { esp_wifi_wps_disable() })?;
            Ok(status)
        } else {
            Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())
        }
    }

    fn is_wps_finished(&self) -> Result<bool, EspError> {
        Ok(self.status.lock().wps.is_some())
    }

    fn get_sta_conf(&self) -> Result<ClientConfiguration, EspError> {
        let mut wifi_config: wifi_config_t = Default::default();
        esp!(unsafe { esp_wifi_get_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_config) })?;

        let result: ClientConfiguration = unsafe { Newtype(wifi_config.sta).into() };

        ::log::debug!("Providing STA configuration: {:?}", result);

        Ok(result)
    }

    fn set_sta_conf(&mut self, conf: &ClientConfiguration) -> Result<(), EspError> {
        ::log::debug!("Checking current STA configuration");
        let current_config = self.get_sta_conf()?;

        if current_config != *conf {
            ::log::debug!("Setting STA configuration: {conf:?}");

            let mut wifi_config = wifi_config_t {
                sta: Newtype::<wifi_sta_config_t>::try_from(conf)?.0,
            };

            esp!(unsafe { esp_wifi_set_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_config) })?;
        } else {
            ::log::debug!("Same STA configuration already present");
        }

        ::log::debug!("STA configuration done");

        Ok(())
    }

    fn get_ap_conf(&self) -> Result<AccessPointConfiguration, EspError> {
        let mut wifi_config: wifi_config_t = Default::default();
        esp!(unsafe { esp_wifi_get_config(wifi_interface_t_WIFI_IF_AP, &mut wifi_config) })?;

        let result: AccessPointConfiguration = unsafe { Newtype(wifi_config.ap).into() };

        ::log::debug!("Providing AP configuration: {:?}", result);

        Ok(result)
    }

    fn set_ap_conf(&mut self, conf: &AccessPointConfiguration) -> Result<(), EspError> {
        ::log::debug!("Checking current AP configuration");
        let current_config = self.get_ap_conf()?;

        if current_config != *conf {
            ::log::debug!("Setting AP configuration: {conf:?}");

            let mut wifi_config = wifi_config_t {
                ap: Newtype::<wifi_ap_config_t>::try_from(conf)?.0,
            };

            esp!(unsafe { esp_wifi_set_config(wifi_interface_t_WIFI_IF_AP, &mut wifi_config) })?;
        } else {
            ::log::debug!("Same AP configuration already present");
        }

        ::log::debug!("AP configuration done");

        Ok(())
    }

    fn clear_all(&mut self) -> Result<(), EspError> {
        let _ = self.disconnect();
        let _ = self.stop();

        unsafe {
            esp!(esp_wifi_deinit())?;
        }

        unsafe {
            // Callbacks are already deregistered by `esp_wifi_deinit`, just null-ify our own refs
            RX_CALLBACK = None;
            TX_CALLBACK = None;
        }

        ::log::debug!("Driver deinitialized");

        Ok(())
    }

    fn get_scan_count(&mut self) -> Result<usize, EspError> {
        let mut found_ap: u16 = 0;
        esp!(unsafe { esp_wifi_scan_get_ap_num(&mut found_ap as *mut _) })?;

        ::log::debug!("Found {found_ap} access points");

        Ok(found_ap as usize)
    }

    fn fetch_scan_result(
        &mut self,
        ap_infos_raw: &mut [wifi_ap_record_t],
    ) -> Result<usize, EspError> {
        ::log::debug!("About to get info for found access points");

        let mut ap_count: u16 = ap_infos_raw.len() as u16;

        esp!(unsafe { esp_wifi_scan_get_ap_records(&mut ap_count, ap_infos_raw.as_mut_ptr(),) })?;

        ::log::debug!("Got info for {ap_count} access points");

        Ok(ap_count as usize)
    }

    unsafe extern "C" fn handle_rx_ap(
        buf: *mut ffi::c_void,
        len: u16,
        eb: *mut ffi::c_void,
    ) -> esp_err_t {
        Self::handle_rx(WifiDeviceId::Ap, buf, len, eb)
    }

    unsafe extern "C" fn handle_rx_sta(
        buf: *mut ffi::c_void,
        len: u16,
        eb: *mut ffi::c_void,
    ) -> esp_err_t {
        Self::handle_rx(WifiDeviceId::Sta, buf, len, eb)
    }

    unsafe fn handle_rx(
        device_id: WifiDeviceId,
        buf: *mut ffi::c_void,
        len: u16,
        eb: *mut ffi::c_void,
    ) -> esp_err_t {
        #[allow(static_mut_refs)]
        let res = RX_CALLBACK.as_mut().unwrap()(device_id, WifiFrame::new(buf.cast(), len, eb));

        match res {
            Ok(_) => ESP_OK,
            Err(e) => e.code(),
        }
    }

    unsafe extern "C" fn handle_tx(ifidx: u8, data: *mut u8, len: *mut u16, tx_status: bool) {
        #[allow(static_mut_refs)]
        TX_CALLBACK.as_mut().unwrap()(
            (ifidx as wifi_interface_t).into(),
            core::slice::from_raw_parts(data as *const _, len as usize),
            tx_status,
        );
    }

    pub fn get_rssi(&self) -> Result<i32, EspError> {
        let mut rssi: core::ffi::c_int = 0;
        unsafe {
            esp_wifi_sta_get_rssi(&mut rssi as *mut core::ffi::c_int);
        };
        Ok(rssi as i32)
    }
}

unsafe impl Send for WifiDriver<'_> {}

impl NonBlocking for WifiDriver<'_> {
    fn is_scan_done(&self) -> Result<bool, EspError> {
        WifiDriver::is_scan_done(self)
    }

    fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        WifiDriver::start_scan(self, scan_config, blocking)
    }

    fn stop_scan(&mut self) -> Result<(), EspError> {
        WifiDriver::stop_scan(self)
    }

    fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        WifiDriver::get_scan_result_n(self)
    }

    #[cfg(feature = "alloc")]
    fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        WifiDriver::get_scan_result(self)
    }

    fn start_wps(&mut self, config: &WpsConfig) -> Result<(), EspError> {
        WifiDriver::start_wps(self, config)
    }

    fn stop_wps(&mut self) -> Result<WpsStatus, EspError> {
        WifiDriver::stop_wps(self)
    }

    fn is_wps_finished(&self) -> Result<bool, EspError> {
        WifiDriver::is_wps_finished(self)
    }
}

impl Drop for WifiDriver<'_> {
    fn drop(&mut self) {
        self.clear_all().unwrap();

        ::log::debug!("WifiDriver Dropped");
    }
}

impl Wifi for WifiDriver<'_> {
    type Error = EspError;

    fn get_capabilities(&self) -> Result<EnumSet<Capability>, Self::Error> {
        WifiDriver::get_capabilities(self)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        WifiDriver::is_started(self)
    }

    fn is_connected(&self) -> Result<bool, Self::Error> {
        WifiDriver::is_connected(self)
    }

    fn get_configuration(&self) -> Result<Configuration, Self::Error> {
        WifiDriver::get_configuration(self)
    }

    fn set_configuration(&mut self, conf: &Configuration) -> Result<(), Self::Error> {
        WifiDriver::set_configuration(self, conf)
    }

    fn start(&mut self) -> Result<(), Self::Error> {
        WifiDriver::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        WifiDriver::stop(self)
    }

    fn connect(&mut self) -> Result<(), Self::Error> {
        WifiDriver::connect(self)
    }

    fn disconnect(&mut self) -> Result<(), Self::Error> {
        WifiDriver::disconnect(self)
    }

    fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), Self::Error> {
        WifiDriver::scan_n(self)
    }

    #[cfg(feature = "alloc")]
    fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, Self::Error> {
        WifiDriver::scan(self)
    }
}

pub struct WifiFrame {
    buf: *mut u8,
    len: u16,
    eb: *mut ffi::c_void,
}

unsafe impl Send for WifiFrame {}

impl WifiFrame {
    const unsafe fn new(buf: *mut u8, len: u16, eb: *mut ffi::c_void) -> Self {
        Self { buf, len, eb }
    }

    pub const fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.buf, self.len as _) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.buf, self.len as _) }
    }
}

impl ops::Deref for WifiFrame {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.buf, self.len as _) }
    }
}

impl ops::DerefMut for WifiFrame {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.buf, self.len as _) }
    }
}

impl Drop for WifiFrame {
    fn drop(&mut self) {
        unsafe { esp_wifi_internal_free_rx_buffer(self.eb) };
    }
}

/// `EspWifi` wraps a `WifiDriver` Data Link layer instance, and binds the OSI
/// Layer 3 (network) facilities of ESP IDF to it.
///
/// In other words, it connects the ESP IDF AP and STA Netif interfaces to the
/// Wifi driver. This allows users to utilize the Rust STD APIs for working with
/// TCP and UDP sockets.
///
/// This struct should be the default option for a Wifi driver in all use cases
/// but the niche one where bypassing the ESP IDF Netif and lwIP stacks is
/// desirable. E.g., using `smoltcp` or other custom IP stacks on top of the
/// ESP IDF Wifi radio.
#[cfg(esp_idf_comp_esp_netif_enabled)]
pub struct EspWifi<'d> {
    #[cfg(esp_idf_esp_wifi_softap_support)]
    ap_netif: EspNetif,
    sta_netif: EspNetif,
    driver: WifiDriver<'d>,
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<'d> EspWifi<'d> {
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    pub fn new<M: WifiModemPeripheral + 'd>(
        modem: M,
        sysloop: EspSystemEventLoop,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Self::wrap(WifiDriver::new(modem, sysloop, nvs)?)
    }

    #[cfg(not(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled)))]
    pub fn new<M: WifiModemPeripheral + 'd>(
        modem: M,
        sysloop: EspSystemEventLoop,
    ) -> Result<Self, EspError> {
        Self::wrap(WifiDriver::new(modem, sysloop)?)
    }

    pub fn wrap(driver: WifiDriver<'d>) -> Result<Self, EspError> {
        Self::wrap_all(
            driver,
            EspNetif::new(NetifStack::Sta)?,
            #[cfg(esp_idf_esp_wifi_softap_support)]
            EspNetif::new(NetifStack::Ap)?,
        )
    }

    pub fn wrap_all(
        driver: WifiDriver<'d>,
        sta_netif: EspNetif,
        #[cfg(esp_idf_esp_wifi_softap_support)] ap_netif: EspNetif,
    ) -> Result<Self, EspError> {
        let mut this = Self {
            driver,
            sta_netif,
            #[cfg(esp_idf_esp_wifi_softap_support)]
            ap_netif,
        };

        this.attach_netif()?;

        Ok(this)
    }

    #[cfg(esp_idf_esp_wifi_softap_support)]
    /// Replaces the network interfaces with the given ones. Returns the old ones.
    pub fn swap_netif(
        &mut self,
        sta_netif: EspNetif,
        ap_netif: EspNetif,
    ) -> Result<(EspNetif, EspNetif), EspError> {
        self.detach_netif()?;

        let old_sta = core::mem::replace(&mut self.sta_netif, sta_netif);
        let old_ap = core::mem::replace(&mut self.ap_netif, ap_netif);

        self.attach_netif()?;

        Ok((old_sta, old_ap))
    }

    /// Replaces the STA network interface with the provided one and returns the
    /// existing network interface.
    pub fn swap_netif_sta(&mut self, sta_netif: EspNetif) -> Result<EspNetif, EspError> {
        self.detach_netif()?;

        let old = core::mem::replace(&mut self.sta_netif, sta_netif);

        self.attach_netif()?;

        Ok(old)
    }

    #[cfg(esp_idf_esp_wifi_softap_support)]
    /// Replaces the AP network interface with the provided one and returns the
    /// existing network interface.
    pub fn swap_netif_ap(&mut self, ap_netif: EspNetif) -> Result<EspNetif, EspError> {
        self.detach_netif()?;

        let old = core::mem::replace(&mut self.ap_netif, ap_netif);

        self.attach_netif()?;

        Ok(old)
    }

    /// Returns the underlying [`WifiDriver`]
    pub fn driver(&self) -> &WifiDriver<'d> {
        &self.driver
    }

    /// Returns the underlying [`WifiDriver`], as mutable
    pub fn driver_mut(&mut self) -> &mut WifiDriver<'d> {
        &mut self.driver
    }

    /// Returns the underlying [`EspNetif`] for client mode
    pub fn sta_netif(&self) -> &EspNetif {
        &self.sta_netif
    }

    /// Returns the underlying [`EspNetif`] for client mode, as mutable
    pub fn sta_netif_mut(&mut self) -> &mut EspNetif {
        &mut self.sta_netif
    }

    #[cfg(esp_idf_esp_wifi_softap_support)]
    /// Returns the underlying [`EspNetif`] for AP mode
    pub fn ap_netif(&self) -> &EspNetif {
        &self.ap_netif
    }

    #[cfg(esp_idf_esp_wifi_softap_support)]
    /// Returns the underlying [`EspNetif`] for AP mode, as mutable
    pub fn ap_netif_mut(&mut self) -> &mut EspNetif {
        &mut self.ap_netif
    }

    /// As per [`WifiDriver::get_capabilities()`]
    pub fn get_capabilities(&self) -> Result<EnumSet<Capability>, EspError> {
        self.driver().get_capabilities()
    }

    /// As per [`WifiDriver::is_started()`]
    pub fn is_started(&self) -> Result<bool, EspError> {
        self.driver().is_started()
    }

    /// As per [`WifiDriver::is_connected()`]
    pub fn is_connected(&self) -> Result<bool, EspError> {
        self.driver().is_connected()
    }

    /// Returns `true` when the driver has a connection, it has enabled either
    /// client or AP mode, and either the client or AP network interface is up.
    pub fn is_up(&self) -> Result<bool, EspError> {
        if !self.driver().is_connected()? {
            Ok(false)
        } else {
            let sta_enabled = self.driver().is_sta_enabled()?;
            let ok = !sta_enabled || self.sta_netif().is_up()?;

            #[cfg(esp_idf_esp_wifi_softap_support)]
            let ok = ok && {
                let ap_enabled = self.driver().is_ap_enabled()?;
                !ap_enabled || self.ap_netif().is_up()?
            };
            Ok(ok)
        }
    }

    /// As per [`WifiDriver::get_configuration()`]
    pub fn get_configuration(&self) -> Result<Configuration, EspError> {
        self.driver().get_configuration()
    }

    /// As per [`WifiDriver::set_configuration()`]
    pub fn set_configuration(&mut self, conf: &Configuration) -> Result<(), EspError> {
        self.driver_mut().set_configuration(conf)
    }

    /// As per [`WifiDriver::start()`]
    pub fn start(&mut self) -> Result<(), EspError> {
        self.driver_mut().start()
    }

    /// As per [`WifiDriver::stop()`]
    pub fn stop(&mut self) -> Result<(), EspError> {
        self.driver_mut().stop()
    }

    /// As per [`WifiDriver::connect()`]
    pub fn connect(&mut self) -> Result<(), EspError> {
        self.driver_mut().connect()
    }

    /// As per [`WifiDriver::disconnect()`]
    pub fn disconnect(&mut self) -> Result<(), EspError> {
        self.driver_mut().disconnect()
    }

    /// As per [`WifiDriver::is_scan_done()`]
    pub fn is_scan_done(&self) -> Result<bool, EspError> {
        self.driver().is_scan_done()
    }

    /// As per [`WifiDriver::scan_n()`]
    pub fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.driver_mut().scan_n()
    }

    /// As per [`WifiDriver::scan()`]
    #[cfg(feature = "alloc")]
    pub fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.driver_mut().scan()
    }

    /// As per [`WifiDriver::start_scan()`].
    pub fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        self.driver_mut().start_scan(scan_config, blocking)
    }

    /// As per [`WifiDriver::stop_scan()`].
    pub fn stop_scan(&mut self) -> Result<(), EspError> {
        self.driver_mut().stop_scan()
    }

    /// As per [`WifiDriver::get_scan_result_n()`].
    pub fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.driver_mut().get_scan_result_n()
    }

    /// As per [`WifiDriver::get_scan_result()`].
    #[cfg(feature = "alloc")]
    pub fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.driver_mut().get_scan_result()
    }

    /// As per [`WifiDriver::start_wps()`]
    pub fn start_wps(&mut self, config: &WpsConfig) -> Result<(), EspError> {
        self.driver_mut().start_wps(config)
    }

    pub fn stop_wps(&mut self) -> Result<WpsStatus, EspError> {
        self.driver_mut().stop_wps()
    }

    pub fn is_wps_finished(&self) -> Result<bool, EspError> {
        self.driver().is_wps_finished()
    }

    /// As per [`WifiDriver::get_mac()`].
    pub fn get_mac(&self, interface: WifiDeviceId) -> Result<[u8; 6], EspError> {
        self.driver().get_mac(interface)
    }

    /// As per [`WifiDriver::set_mac()`].
    pub fn set_mac(&mut self, interface: WifiDeviceId, mac: [u8; 6]) -> Result<(), EspError> {
        self.driver_mut().set_mac(interface, mac)
    }

    fn attach_netif(&mut self) -> Result<(), EspError> {
        let _ = self.driver.stop();

        #[cfg(esp_idf_esp_wifi_softap_support)]
        {
            esp!(unsafe { esp_netif_attach_wifi_ap(self.ap_netif.handle()) })?;
            esp!(unsafe { esp_wifi_set_default_wifi_ap_handlers() })?;
        }

        esp!(unsafe { esp_netif_attach_wifi_station(self.sta_netif.handle()) })?;
        esp!(unsafe { esp_wifi_set_default_wifi_sta_handlers() })?;

        Ok(())
    }

    fn detach_netif(&mut self) -> Result<(), EspError> {
        let _ = self.driver.stop();

        #[cfg(esp_idf_esp_wifi_softap_support)]
        esp!(unsafe {
            esp_wifi_clear_default_wifi_driver_and_handlers(
                self.ap_netif.handle() as *mut ffi::c_void
            )
        })?;

        esp!(unsafe {
            esp_wifi_clear_default_wifi_driver_and_handlers(
                self.sta_netif.handle() as *mut ffi::c_void
            )
        })?;

        Ok(())
    }

    pub fn get_rssi(&self) -> Result<i32, EspError> {
        self.driver().get_rssi()
    }

    /// Get information about the AP with which the station is associated.
    /// Useful to get the current signal strength of the AP.
    pub fn get_ap_info(&self) -> Result<AccessPointInfo, EspError> {
        self.driver().get_ap_info()
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl Drop for EspWifi<'_> {
    fn drop(&mut self) {
        self.detach_netif().unwrap();

        ::log::info!("EspWifi dropped");
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
unsafe impl Send for EspWifi<'_> {}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl NonBlocking for EspWifi<'_> {
    fn is_scan_done(&self) -> Result<bool, EspError> {
        EspWifi::is_scan_done(self)
    }

    fn start_scan(
        &mut self,
        scan_config: &config::ScanConfig,
        blocking: bool,
    ) -> Result<(), EspError> {
        EspWifi::start_scan(self, scan_config, blocking)
    }

    fn stop_scan(&mut self) -> Result<(), EspError> {
        EspWifi::stop_scan(self)
    }

    fn get_scan_result_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        EspWifi::get_scan_result_n(self)
    }

    #[cfg(feature = "alloc")]
    fn get_scan_result(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        EspWifi::get_scan_result(self)
    }

    fn start_wps(&mut self, config: &WpsConfig) -> Result<(), EspError> {
        EspWifi::start_wps(self, config)
    }

    fn stop_wps(&mut self) -> Result<WpsStatus, EspError> {
        EspWifi::stop_wps(self)
    }

    fn is_wps_finished(&self) -> Result<bool, EspError> {
        EspWifi::is_wps_finished(self)
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl Wifi for EspWifi<'_> {
    type Error = EspError;

    fn get_capabilities(&self) -> Result<EnumSet<Capability>, Self::Error> {
        EspWifi::get_capabilities(self)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        EspWifi::is_started(self)
    }

    fn is_connected(&self) -> Result<bool, Self::Error> {
        EspWifi::is_connected(self)
    }

    fn get_configuration(&self) -> Result<Configuration, Self::Error> {
        EspWifi::get_configuration(self)
    }

    fn set_configuration(&mut self, conf: &Configuration) -> Result<(), Self::Error> {
        EspWifi::set_configuration(self, conf)
    }

    fn start(&mut self) -> Result<(), Self::Error> {
        EspWifi::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        EspWifi::stop(self)
    }

    fn connect(&mut self) -> Result<(), Self::Error> {
        EspWifi::connect(self)
    }

    fn disconnect(&mut self) -> Result<(), Self::Error> {
        EspWifi::disconnect(self)
    }

    fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), Self::Error> {
        EspWifi::scan_n(self)
    }

    fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, Self::Error> {
        EspWifi::scan(self)
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl NetifStatus for EspWifi<'_> {
    fn is_up(&self) -> Result<bool, EspError> {
        EspWifi::is_up(self)
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct StaScanDoneRef(wifi_event_sta_scan_done_t);

impl StaScanDoneRef {
    /// Whether this scan is a success or not
    pub fn is_successful(&self) -> bool {
        self.0.status == 0
    }

    /// Number of scan results
    pub fn len(&self) -> usize {
        self.0.number as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sequential identifier of the scan
    pub fn id(&self) -> u8 {
        self.0.scan_id
    }
}

impl fmt::Debug for StaScanDoneRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaScanDoneRef")
            .field("is_successful", &self.is_successful())
            .field("len", &self.len())
            .field("id", &self.id())
            .finish()
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct StaConnectedRef(wifi_event_sta_connected_t);

impl StaConnectedRef {
    /// SSID of the AP we connected to
    pub fn ssid(&self) -> &[u8] {
        &self.0.ssid.as_slice()[..self.0.ssid_len as usize]
    }

    /// BSSID (or MAC address) of the AP we connected to
    pub fn bssid(&self) -> [u8; 6] {
        self.0.bssid
    }

    /// Channel used for the connection to the AP
    pub fn channel(&self) -> u8 {
        self.0.channel
    }

    /// Authentication method used for connecting to the AP
    pub fn authmode(&self) -> AuthMethod {
        Option::<AuthMethod>::from(Newtype(self.0.authmode)).unwrap()
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    /// Association ID given by the AP
    pub fn aid(&self) -> u16 {
        self.0.aid
    }
}

impl fmt::Debug for StaConnectedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("StaConnectedRef");

        let d = d
            .field("ssid", &alloc::string::String::from_utf8_lossy(self.ssid()))
            .field("bssid", &self.bssid())
            .field("channel", &self.channel())
            .field("authmode", &self.authmode());

        #[cfg(not(esp_idf_version_major = "4"))]
        let d = d.field("aid", &self.aid());

        d.finish()
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct StaDisconnectedRef(wifi_event_sta_disconnected_t);

impl StaDisconnectedRef {
    /// SSID of the AP we disconnected from
    pub fn ssid(&self) -> &[u8] {
        &self.0.ssid.as_slice()[..self.0.ssid_len as usize]
    }

    /// BSSID (or MAC address) of the AP we disconnected from
    pub fn bssid(&self) -> [u8; 6] {
        self.0.bssid
    }

    /// The reason for disconnection
    pub fn reason(&self) -> u16 {
        self.0.reason as u16
    }

    /// RSSI at the time of disconnection
    pub fn rssi(&self) -> i8 {
        self.0.rssi
    }
}

impl fmt::Debug for StaDisconnectedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaDisconnectedRef")
            .field("ssid", &alloc::string::String::from_utf8_lossy(self.ssid()))
            .field("bssid", &self.bssid())
            .field("reason", &self.reason())
            .field("rssi", &self.rssi())
            .finish()
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ApStaConnectedRef(wifi_event_ap_staconnected_t);

impl ApStaConnectedRef {
    /// MAC address of the station connected to ESP32 soft-AP
    pub fn mac(&self) -> [u8; 6] {
        self.0.mac
    }

    /// the aid that ESP32 soft-AP gives to the station connected to
    pub fn aid(&self) -> u8 {
        self.0.aid
    }

    /// flag to identify mesh child
    pub fn is_mesh_child(&self) -> bool {
        self.0.is_mesh_child
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ApStaDisconnectedRef(wifi_event_ap_stadisconnected_t);

impl ApStaDisconnectedRef {
    /// MAC address of the station disconnects to ESP32 soft-AP
    pub fn mac(&self) -> [u8; 6] {
        self.0.mac
    }

    /// the aid that ESP32 soft-AP gave to the station disconnects to
    pub fn aid(&self) -> u8 {
        self.0.aid
    }

    /// flag to identify mesh child
    pub fn is_mesh_child(&self) -> bool {
        self.0.is_mesh_child
    }

    /// reason of disconnection
    #[cfg(not(any(
        esp_idf_version_major = "4",
        all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
    )))]
    #[allow(clippy::unnecessary_cast)]
    pub fn reason(&self) -> u16 {
        self.0.reason as u16
    }
}

impl fmt::Debug for ApStaDisconnectedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut ds = f.debug_struct("ApStaDisconnectedRef");
        ds.field("mac", &self.mac())
            .field("aid", &self.aid())
            .field("is_mesh_child", &self.is_mesh_child());

        #[cfg(not(any(
            esp_idf_version_major = "4",
            all(esp_idf_version_major = "5", esp_idf_version_minor = "0"),
        )))]
        ds.field("reason", &self.reason());

        ds.finish()
    }
}

#[cfg(not(any(
    esp_idf_version_major = "4",
    all(
        esp_idf_version_major = "5",
        any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
    ),
)))]
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct HomeChannelChange {
    old_chan: u8,
    old_snd: Option<WifiSecondChan>,
    new_chan: u8,
    new_snd: Option<WifiSecondChan>,
}

#[cfg(not(any(
    esp_idf_version_major = "4",
    all(
        esp_idf_version_major = "5",
        any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
    ),
)))]
#[derive(Copy, Clone, Debug)]
enum WifiSecondChan {
    None = 0,
    Above,
    Below,
}

#[cfg(not(any(
    esp_idf_version_major = "4",
    all(
        esp_idf_version_major = "5",
        any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
    ),
)))]
impl TryFrom<u32> for WifiSecondChan {
    type Error = &'static str;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        #![allow(non_upper_case_globals)]
        #[allow(non_snake_case)]
        match value {
            wifi_second_chan_t_WIFI_SECOND_CHAN_NONE => Ok(Self::None),
            wifi_second_chan_t_WIFI_SECOND_CHAN_ABOVE => Ok(Self::Above),
            wifi_second_chan_t_WIFI_SECOND_CHAN_BELOW => Ok(Self::Below),
            _ => Err("Invalid"),
        }
    }
}

/// Payload reference for [`WifiEvent::StaNeighborRep`].
///
/// The neighbor report bytes are stored in a flexible array member that
/// immediately follows the fixed header in memory.
#[cfg(esp_idf_version_at_least_5_3_0)]
#[repr(transparent)]
pub struct StaNeighborRepRef(wifi_event_neighbor_report_t);

#[cfg(esp_idf_version_at_least_5_3_0)]
impl StaNeighborRepRef {
    /// Number of bytes in the neighbor report.
    pub fn report_len(&self) -> usize {
        self.0.report_len as usize
    }

    /// The raw neighbor report bytes received from the AP.
    ///
    /// The slice is reconstructed from the flexible array member
    /// (`n_report`) that immediately follows the fixed header in memory.
    pub fn report(&self) -> &[u8] {
        // SAFETY: `wifi_event_neighbor_report_t` is followed in memory by
        // exactly `report_len` bytes of report data (the `n_report` FAM).
        // The pointer arithmetic is safe because `as_payload()` hands us a
        // reference to the full event payload buffer whose size is at least
        // `sizeof(wifi_event_neighbor_report_t) + report_len`.
        unsafe { core::slice::from_raw_parts(self.0.n_report.as_ptr(), self.report_len()) }
    }
}

#[cfg(esp_idf_version_at_least_5_3_0)]
impl fmt::Debug for StaNeighborRepRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaNeighborRepRef")
            .field("report_len", &self.report_len())
            .field("report", &self.report())
            .finish()
    }
}

/// Payload reference for [`WifiEvent::ApWrongPassword`].
#[cfg(any(
    esp_idf_version_patch_at_least_5_3_3,
    esp_idf_version_patch_at_least_5_4_1,
    esp_idf_version_at_least_5_5_0,
))]
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ApWrongPasswordRef(wifi_event_ap_wrong_password_t);

#[cfg(any(
    esp_idf_version_patch_at_least_5_3_3,
    esp_idf_version_patch_at_least_5_4_1,
    esp_idf_version_at_least_5_5_0,
))]
impl ApWrongPasswordRef {
    /// MAC address of the station that supplied a wrong password.
    pub fn mac(&self) -> [u8; 6] {
        self.0.mac
    }
}

#[cfg(any(
    esp_idf_version_patch_at_least_5_3_3,
    esp_idf_version_patch_at_least_5_4_1,
    esp_idf_version_at_least_5_5_0,
))]
impl fmt::Debug for ApWrongPasswordRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApWrongPasswordRef")
            .field("mac", &self.mac())
            .finish()
    }
}

/// Payload reference for [`WifiEvent::StaBeaconOffsetUnstable`].
#[cfg(esp_idf_version_at_least_5_5_0)]
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct StaBeaconOffsetUnstableRef(wifi_event_sta_beacon_offset_unstable_t);

#[cfg(esp_idf_version_at_least_5_5_0)]
impl StaBeaconOffsetUnstableRef {
    /// Fraction of beacons successfully received during the unstable period.
    pub fn beacon_success_rate(&self) -> f32 {
        self.0.beacon_success_rate
    }
}

#[cfg(esp_idf_version_at_least_5_5_0)]
impl fmt::Debug for StaBeaconOffsetUnstableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaBeaconOffsetUnstableRef")
            .field("beacon_success_rate", &self.beacon_success_rate())
            .finish()
    }
}

/// Payload reference for [`WifiEvent::DppUriReady`].
///
/// The URI string is stored in a flexible array member that immediately
/// follows the fixed header; `uri_data_len` includes the null terminator.
/// [`DppUriReadyRef::uri`] strips the terminator and returns the URI as a `&str`.
#[cfg(esp_idf_version_at_least_5_5_0)]
#[repr(transparent)]
pub struct DppUriReadyRef(wifi_event_dpp_uri_ready_t);

#[cfg(esp_idf_version_at_least_5_5_0)]
impl DppUriReadyRef {
    /// Length of the URI string **excluding** the null terminator.
    pub fn uri_len(&self) -> usize {
        (self.0.uri_data_len as usize).saturating_sub(1)
    }

    /// The DPP bootstrapping URI as a string slice (no null terminator).
    ///
    /// The slice is reconstructed from the flexible array member
    /// (`uri`) that immediately follows the fixed header in memory.
    ///
    /// DPP bootstrapping URIs are defined by the Wi-Fi Alliance DPP
    /// specification to be ASCII-printable, so the bytes are always
    /// valid UTF-8.
    pub fn uri(&self) -> &str {
        // SAFETY: `uri_data_len` includes the null terminator; we subtract 1
        // to get the string length without it. DPP URIs are always ASCII so
        // `from_utf8_unchecked` is safe.
        unsafe {
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                self.0.uri.as_ptr().cast::<u8>(),
                self.uri_len(),
            ))
        }
    }
}

#[cfg(esp_idf_version_at_least_5_5_0)]
impl fmt::Debug for DppUriReadyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DppUriReadyRef")
            .field("uri", &self.uri())
            .finish()
    }
}

/// Payload reference for [`WifiEvent::DppCfgRecvd`].
///
/// Not `Copy`/`Clone`: recent ESP-IDF added a flexible-array-member
/// field (`configs[]`) to `wifi_event_dpp_config_received_t`, which
/// bindgen surfaces as `__IncompleteArrayField` (intentionally not
/// `Clone`). The newtype is always borrowed via `&'a DppCfgRecvdRef`
/// inside event callbacks, so the derive was never load-bearing.
#[cfg(esp_idf_version_at_least_5_5_0)]
#[repr(transparent)]
pub struct DppCfgRecvdRef(wifi_event_dpp_config_received_t);

#[cfg(esp_idf_version_at_least_5_5_0)]
impl DppCfgRecvdRef {
    /// The WiFi configuration received via DPP authentication.
    pub fn wifi_cfg(&self) -> &wifi_config_t {
        &self.0.wifi_cfg
    }
}

#[cfg(esp_idf_version_at_least_5_5_0)]
impl fmt::Debug for DppCfgRecvdRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DppCfgRecvdRef").finish_non_exhaustive()
    }
}

/// Payload reference for [`WifiEvent::DppFailed`].
#[cfg(esp_idf_version_at_least_5_5_0)]
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct DppFailedRef(wifi_event_dpp_failed_t);

#[cfg(esp_idf_version_at_least_5_5_0)]
impl DppFailedRef {
    /// The DPP failure reason code.
    pub fn failure_reason(&self) -> i32 {
        self.0.failure_reason
    }
}

#[cfg(esp_idf_version_at_least_5_5_0)]
impl fmt::Debug for DppFailedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DppFailedRef")
            .field("failure_reason", &self.failure_reason())
            .finish()
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct WpsCredentialsRef(wifi_event_sta_wps_er_success_t__bindgen_ty_1);

impl WpsCredentialsRef {
    pub fn ssid(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.0.ssid.as_ptr() as *const _) }
    }

    pub fn passphrase(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.0.passphrase.as_ptr() as *const _) }
    }
}

impl fmt::Debug for WpsCredentialsRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WpsCredentialsRef")
            .field("ssid", &self.ssid())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WpsCredentials {
    pub ssid: heapless::String<32>,
    pub passphrase: heapless::String<64>,
}

impl TryFrom<&WpsCredentialsRef> for WpsCredentials {
    type Error = EspError;

    fn try_from(credentials: &WpsCredentialsRef) -> Result<Self, Self::Error> {
        let err = EspError::from_infallible::<ESP_ERR_INVALID_ARG>();

        Ok(Self {
            ssid: credentials
                .ssid()
                .to_str()
                .map_err(|_| err)
                .and_then(|credentials| credentials.try_into().map_err(|_| err))?,
            passphrase: credentials
                .passphrase()
                .to_str()
                .map_err(|_| err)
                .and_then(|credentials| credentials.try_into().map_err(|_| err))?,
        })
    }
}

impl fmt::Debug for ApStaConnectedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApStaConnectedRef")
            .field("mac", &self.mac())
            .field("aid", &self.aid())
            .field("is_mesh_child", &self.is_mesh_child())
            .finish()
    }
}

#[derive(Copy, Clone, Debug)]
pub enum WifiEvent<'a> {
    Ready,

    ScanDone(&'a StaScanDoneRef),

    StaStarted,
    StaStopped,
    StaConnected(&'a StaConnectedRef),
    StaDisconnected(&'a StaDisconnectedRef),
    StaAuthmodeChanged,
    StaBssRssiLow,
    StaBeaconTimeout,
    StaWpsSuccess(&'a [WpsCredentialsRef]),
    StaWpsFailed,
    StaWpsTimeout,
    StaWpsPin(Option<u32>),
    StaWpsPbcOverlap,

    ApStarted,
    ApStopped,
    ApStaConnected(&'a ApStaConnectedRef),
    ApStaDisconnected(&'a ApStaDisconnectedRef),
    ApProbeRequestReceived,

    FtmReport,
    ActionTxStatus,
    RocDone,

    #[cfg(not(any(
        esp_idf_version_major = "4",
        all(
            esp_idf_version_major = "5",
            any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
        ),
    )))]
    HomeChannelChange(HomeChannelChange),

    /// Neighbor report received from the AP in response to a neighbor report
    /// request from the station.
    #[cfg(esp_idf_version_at_least_5_3_0)]
    StaNeighborRep(&'a StaNeighborRepRef),

    /// A station attempted to connect to the soft-AP with the wrong password.
    #[cfg(any(
        esp_idf_version_patch_at_least_5_3_3,
        esp_idf_version_patch_at_least_5_4_1,
        esp_idf_version_at_least_5_5_0,
    ))]
    ApWrongPassword(&'a ApWrongPasswordRef),

    /// The station's beacon offset has become unstable.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    StaBeaconOffsetUnstable(&'a StaBeaconOffsetUnstableRef),

    /// A DPP bootstrapping URI is ready to be scanned by the peer.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    DppUriReady(&'a DppUriReadyRef),

    /// DPP authentication succeeded and a WiFi configuration was received.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    DppCfgRecvd(&'a DppCfgRecvdRef),

    /// DPP authentication or configuration exchange failed.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    DppFailed(&'a DppFailedRef),

    /// An event ID not recognised by this version of the library was received.
    ///
    /// This variant is produced instead of panicking when an unknown event ID
    /// arrives, allowing applications to remain forward-compatible with
    /// ESP-IDF versions that introduce new WiFi events.
    Other(i32),
}

unsafe impl EspEventSource for WifiEvent<'_> {
    fn source() -> Option<&'static ffi::CStr> {
        Some(unsafe { ffi::CStr::from_ptr(WIFI_EVENT) })
    }
}

impl EspEventDeserializer for WifiEvent<'_> {
    type Data<'d> = WifiEvent<'d>;

    #[allow(non_upper_case_globals, non_snake_case)]
    fn deserialize<'d>(data: &crate::eventloop::EspEvent<'d>) -> WifiEvent<'d> {
        let event_id = data.event_id as u32;

        match event_id {
            wifi_event_t_WIFI_EVENT_WIFI_READY => WifiEvent::Ready,
            wifi_event_t_WIFI_EVENT_SCAN_DONE => WifiEvent::ScanDone(unsafe { data.as_payload() }),
            wifi_event_t_WIFI_EVENT_STA_START => WifiEvent::StaStarted,
            wifi_event_t_WIFI_EVENT_STA_STOP => WifiEvent::StaStopped,
            wifi_event_t_WIFI_EVENT_STA_CONNECTED => {
                WifiEvent::StaConnected(unsafe { data.as_payload() })
            }
            wifi_event_t_WIFI_EVENT_STA_DISCONNECTED => {
                WifiEvent::StaDisconnected(unsafe { data.as_payload() })
            }
            wifi_event_t_WIFI_EVENT_STA_AUTHMODE_CHANGE => WifiEvent::StaAuthmodeChanged,
            wifi_event_t_WIFI_EVENT_STA_WPS_ER_SUCCESS => {
                let credentials: &[wifi_event_sta_wps_er_success_t__bindgen_ty_1] = data
                    .payload
                    .map(|x| x as *const _ as *const wifi_event_sta_wps_er_success_t)
                    .and_then(|x| unsafe { x.as_ref() })
                    .map(|x| &x.ap_cred[0..x.ap_cred_cnt as usize])
                    .unwrap_or(&[]);
                // SAFETY: transparent representation of target type
                let credentials: &[WpsCredentialsRef] =
                    unsafe { core::mem::transmute(credentials) };

                WifiEvent::StaWpsSuccess(credentials)
            }
            wifi_event_t_WIFI_EVENT_STA_WPS_ER_FAILED => WifiEvent::StaWpsFailed,
            wifi_event_t_WIFI_EVENT_STA_WPS_ER_TIMEOUT => WifiEvent::StaWpsTimeout,
            wifi_event_t_WIFI_EVENT_STA_WPS_ER_PIN => {
                let pin = data
                    .payload
                    .map(|x| x as *const _ as *const wifi_event_sta_wps_er_pin_t)
                    .and_then(|x| unsafe { x.as_ref() })
                    .and_then(|x| core::str::from_utf8(&x.pin_code).ok())
                    .and_then(|x| x.parse().ok());
                WifiEvent::StaWpsPin(pin)
            }
            wifi_event_t_WIFI_EVENT_STA_WPS_ER_PBC_OVERLAP => WifiEvent::StaWpsPbcOverlap,
            wifi_event_t_WIFI_EVENT_AP_START => WifiEvent::ApStarted,
            wifi_event_t_WIFI_EVENT_AP_STOP => WifiEvent::ApStopped,
            wifi_event_t_WIFI_EVENT_AP_STACONNECTED => {
                WifiEvent::ApStaConnected(unsafe { data.as_payload() })
            }
            wifi_event_t_WIFI_EVENT_AP_STADISCONNECTED => {
                WifiEvent::ApStaDisconnected(unsafe { data.as_payload() })
            }
            wifi_event_t_WIFI_EVENT_AP_PROBEREQRECVED => WifiEvent::ApProbeRequestReceived,
            wifi_event_t_WIFI_EVENT_FTM_REPORT => WifiEvent::FtmReport,
            wifi_event_t_WIFI_EVENT_STA_BSS_RSSI_LOW => WifiEvent::StaBssRssiLow,
            wifi_event_t_WIFI_EVENT_ACTION_TX_STATUS => WifiEvent::ActionTxStatus,
            wifi_event_t_WIFI_EVENT_STA_BEACON_TIMEOUT => WifiEvent::StaBeaconTimeout,
            wifi_event_t_WIFI_EVENT_ROC_DONE => WifiEvent::RocDone,
            #[cfg(not(any(
                esp_idf_version_major = "4",
                all(
                    esp_idf_version_major = "5",
                    any(esp_idf_version_minor = "0", esp_idf_version_minor = "1")
                ),
            )))]
            wifi_event_t_WIFI_EVENT_HOME_CHANNEL_CHANGE => {
                #[cfg(esp_idf_soc_wifi_supported)]
                {
                    let payload = unsafe {
                        (data.payload.unwrap() as *const _
                            as *const wifi_event_home_channel_change_t)
                            .as_ref()
                    }
                    .unwrap();

                    WifiEvent::HomeChannelChange(HomeChannelChange {
                        old_chan: payload.old_chan,
                        old_snd: payload.old_snd.try_into().ok(),
                        new_chan: payload.new_chan,
                        new_snd: payload.new_snd.try_into().ok(),
                    })
                }

                // On SoCs without native WiFi (using esp_wifi_remote), this
                // event may fire without a payload.
                #[cfg(not(esp_idf_soc_wifi_supported))]
                {
                    let payload = data
                        .payload
                        .map(|p| p as *const _ as *const wifi_event_home_channel_change_t)
                        .and_then(|p| unsafe { p.as_ref() });

                    WifiEvent::HomeChannelChange(HomeChannelChange {
                        old_chan: payload.map_or(0, |p| p.old_chan),
                        old_snd: payload.and_then(|p| p.old_snd.try_into().ok()),
                        new_chan: payload.map_or(0, |p| p.new_chan),
                        new_snd: payload.and_then(|p| p.new_snd.try_into().ok()),
                    })
                }
            }
            #[cfg(esp_idf_version_at_least_5_3_0)]
            wifi_event_t_WIFI_EVENT_STA_NEIGHBOR_REP => {
                // `wifi_event_neighbor_report_t` contains `__IncompleteArrayField`
                // which prevents deriving `Copy`, so we cannot use `as_payload()`.
                // The cast is safe for the same reason as `as_payload` internaly:
                // we only produce a shared reference into the event buffer.
                WifiEvent::StaNeighborRep(unsafe {
                    (data.payload.unwrap() as *const _ as *const StaNeighborRepRef)
                        .as_ref()
                        .unwrap()
                })
            }
            #[cfg(any(
                esp_idf_version_patch_at_least_5_3_3,
                esp_idf_version_patch_at_least_5_4_1,
                esp_idf_version_at_least_5_5_0,
            ))]
            wifi_event_t_WIFI_EVENT_AP_WRONG_PASSWORD => {
                WifiEvent::ApWrongPassword(unsafe { data.as_payload() })
            }
            #[cfg(esp_idf_version_at_least_5_5_0)]
            wifi_event_t_WIFI_EVENT_STA_BEACON_OFFSET_UNSTABLE => {
                WifiEvent::StaBeaconOffsetUnstable(unsafe { data.as_payload() })
            }
            #[cfg(esp_idf_version_at_least_5_5_0)]
            wifi_event_t_WIFI_EVENT_DPP_URI_READY => {
                // `wifi_event_dpp_uri_ready_t` contains `__IncompleteArrayField`
                // which prevents deriving `Copy`, so we cannot use `as_payload()`.
                WifiEvent::DppUriReady(unsafe {
                    (data.payload.unwrap() as *const _ as *const DppUriReadyRef)
                        .as_ref()
                        .unwrap()
                })
            }
            #[cfg(esp_idf_version_at_least_5_5_0)]
            wifi_event_t_WIFI_EVENT_DPP_CFG_RECVD => {
                // `wifi_event_dpp_config_received_t` contains
                // `__IncompleteArrayField` which prevents deriving `Copy`,
                // so we cannot use `as_payload()`.
                WifiEvent::DppCfgRecvd(unsafe {
                    (data.payload.unwrap() as *const _ as *const DppCfgRecvdRef)
                        .as_ref()
                        .unwrap()
                })
            }
            #[cfg(esp_idf_version_at_least_5_5_0)]
            wifi_event_t_WIFI_EVENT_DPP_FAILED => {
                WifiEvent::DppFailed(unsafe { data.as_payload() })
            }
            _ => {
                ::log::warn!("WifiEvent: unknown event ID {event_id}, ignoring");
                WifiEvent::Other(event_id as i32)
            }
        }
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const WPS_TIMEOUT: Duration = Duration::from_secs(120);

/// Wraps a [`WifiDriver`] or [`EspWifi`], and offers strictly synchronous (blocking)
/// function calls for their functionality.
// TODO: add an example about wrapping an existing instance of wifidriver/espwifi,
// as well as using that instance once the BlockingWifi drops it.
pub struct BlockingWifi<T> {
    wifi: T,
    event_loop: crate::eventloop::EspSystemEventLoop,
}

impl<T> BlockingWifi<T>
where
    T: Wifi<Error = EspError> + NonBlocking,
{
    pub fn wrap(wifi: T, event_loop: EspSystemEventLoop) -> Result<Self, EspError> {
        Ok(Self { wifi, event_loop })
    }

    /// Returns the underlying [`WifiDriver`] or [`EspWifi`]
    pub fn wifi(&self) -> &T {
        &self.wifi
    }

    /// Returns the underlying [`WifiDriver`] or [`EspWifi`], as mutable
    pub fn wifi_mut(&mut self) -> &mut T {
        &mut self.wifi
    }

    /// As per [`WifiDriver::get_capabilities()`]
    pub fn get_capabilities(&self) -> Result<EnumSet<Capability>, EspError> {
        self.wifi.get_capabilities()
    }

    /// As per [`WifiDriver::get_configuration()`]
    pub fn get_configuration(&self) -> Result<Configuration, EspError> {
        self.wifi.get_configuration()
    }

    /// As per [`WifiDriver::set_configuration()`]
    pub fn set_configuration(&mut self, conf: &Configuration) -> Result<(), EspError> {
        self.wifi.set_configuration(conf)
    }

    /// As per [`WifiDriver::is_started()`]
    pub fn is_started(&self) -> Result<bool, EspError> {
        self.wifi.is_started()
    }

    /// As per [`WifiDriver::is_connected()`]
    pub fn is_connected(&self) -> Result<bool, EspError> {
        self.wifi.is_connected()
    }

    /// As per [`WifiDriver::start()`], but as a blocking call that returns
    /// once the wifi driver has started.
    pub fn start(&mut self) -> Result<(), EspError> {
        self.wifi.start()?;
        self.wifi_wait_while(|| self.wifi.is_started().map(|s| !s), None)
    }

    /// As per [`WifiDriver::stop()`], but as a blocking call that returns
    /// once the wifi driver has stopped.
    pub fn stop(&mut self) -> Result<(), EspError> {
        self.wifi.stop()?;
        self.wifi_wait_while(|| self.wifi.is_started(), None)
    }

    /// As per [`WifiDriver::connect()`], but as a blocking call that returns
    /// once the wifi driver is connected.
    pub fn connect(&mut self) -> Result<(), EspError> {
        self.wifi.connect()?;
        self.wifi_wait_while(
            || self.wifi.is_connected().map(|s| !s),
            Some(CONNECT_TIMEOUT),
        )
    }

    /// As per [`WifiDriver::disconnect()`], but as a blocking call that returns
    /// once the wifi driver has disconnected.
    pub fn disconnect(&mut self) -> Result<(), EspError> {
        self.wifi.disconnect()?;
        self.wifi_wait_while(|| self.wifi.is_connected(), None)
    }

    /// As per [`WifiDriver::scan_n()`]
    pub fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.wifi.scan_n()
    }

    /// As per [`WifiDriver::scan()`]
    #[cfg(feature = "alloc")]
    pub fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.wifi.scan()
    }

    /// Performs a blocking wait until certain condition provided by the user
    /// in the form of a `matcher` callback becomes false. Most often than not
    /// that condition would be related to the state of the Wifi driver. In
    /// other words, whether the driver is started, stopped, (dis)connected and
    /// so on.
    ///
    /// Note that the waiting is not done internally using busy-looping and/or
    /// timeouts. Rather, the condition (`matcher`) is evaluated initially, and
    /// if it returns `true`, it is re-evaluated each time the ESP IDF C Wifi
    /// driver posts a Wifi event on the system event loop. The reasoning behind
    /// this is that changes to the state of the Wifi driver are always
    /// accompanied by posting Wifi events.
    pub fn wifi_wait_while<F: Fn() -> Result<bool, EspError>>(
        &self,
        matcher: F,
        timeout: Option<Duration>,
    ) -> Result<(), EspError> {
        let wait = Wait::new::<WifiEvent>(&self.event_loop)?;

        wait.wait_while(matcher, timeout)
    }

    /// Start WPS and perform a blocking wait until it connects, fails or times
    /// out. A [`WpsStatus`] is returned that contains the success status of the
    /// WPS connection. When the credentials of only one access point are
    /// received, the WiFi driver configuration will automatically be set to
    /// that access point. If multiple credentials were received, a
    /// `WpsStatus::Success` will be returned with a vector containing those
    /// credentials. The caller must then handle those credentials and set the
    /// configuration manually.
    pub fn start_wps(&mut self, config: &WpsConfig) -> Result<WpsStatus, EspError> {
        self.wifi.start_wps(config)?;
        self.wifi_wait_while(
            || self.wifi.is_wps_finished().map(|x| !x),
            Some(WPS_TIMEOUT),
        )?;
        Ok(self.wifi.stop_wps().unwrap_or(WpsStatus::Timeout))
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<T> BlockingWifi<T>
where
    T: NetifStatus,
{
    /// As per [`EspWifi::is_up()`].
    pub fn is_up(&self) -> Result<bool, EspError> {
        self.wifi.is_up()
    }

    /// Waits until the underlaying network interface is up.
    pub fn wait_netif_up(&self) -> Result<(), EspError> {
        self.ip_wait_while(|| self.wifi.is_up().map(|s| !s), Some(CONNECT_TIMEOUT))
    }

    /// As [`BlockingWifi::wifi_wait_while()`], but for `EspWifi` events
    /// related to the IP layer, instead of `WifiDriver` events on the data link layer.
    pub fn ip_wait_while<F: Fn() -> Result<bool, EspError>>(
        &self,
        matcher: F,
        timeout: Option<core::time::Duration>,
    ) -> Result<(), EspError> {
        let wait = crate::eventloop::Wait::new::<IpEvent>(&self.event_loop)?;

        wait.wait_while(matcher, timeout)
    }
}

impl<T> Wifi for BlockingWifi<T>
where
    T: Wifi<Error = EspError> + NonBlocking,
{
    type Error = EspError;

    fn get_capabilities(&self) -> Result<EnumSet<Capability>, Self::Error> {
        BlockingWifi::get_capabilities(self)
    }

    fn get_configuration(&self) -> Result<Configuration, Self::Error> {
        BlockingWifi::get_configuration(self)
    }

    fn set_configuration(&mut self, conf: &Configuration) -> Result<(), Self::Error> {
        BlockingWifi::set_configuration(self, conf)
    }

    fn is_started(&self) -> Result<bool, Self::Error> {
        BlockingWifi::is_started(self)
    }

    fn is_connected(&self) -> Result<bool, Self::Error> {
        BlockingWifi::is_connected(self)
    }

    fn start(&mut self) -> Result<(), Self::Error> {
        BlockingWifi::start(self)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        BlockingWifi::stop(self)
    }

    fn connect(&mut self) -> Result<(), Self::Error> {
        BlockingWifi::connect(self)
    }

    fn disconnect(&mut self) -> Result<(), Self::Error> {
        BlockingWifi::disconnect(self)
    }

    fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), Self::Error> {
        BlockingWifi::scan_n(self)
    }

    #[cfg(feature = "alloc")]
    fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, Self::Error> {
        BlockingWifi::scan(self)
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<T> NetifStatus for BlockingWifi<T>
where
    T: NetifStatus,
{
    fn is_up(&self) -> Result<bool, EspError> {
        BlockingWifi::is_up(self)
    }
}

/// Wraps a [`WifiDriver`] or [`EspWifi`], and offers strictly `async`
/// (non-blocking) function calls for their functionality.
#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
pub struct AsyncWifi<T> {
    wifi: T,
    event_loop: crate::eventloop::EspSystemEventLoop,
    timer_service: crate::timer::EspTaskTimerService,
}

#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
impl<T> AsyncWifi<T>
where
    T: Wifi<Error = EspError> + NonBlocking,
{
    pub fn wrap(
        wifi: T,
        event_loop: EspSystemEventLoop,
        timer_service: EspTaskTimerService,
    ) -> Result<Self, EspError> {
        Ok(Self {
            wifi,
            event_loop,
            timer_service,
        })
    }

    /// Returns the underlying [`WifiDriver`] or [`EspWifi`]
    pub fn wifi(&self) -> &T {
        &self.wifi
    }

    /// Returns the underlying [`WifiDriver`] or [`EspWifi`], as mutable
    pub fn wifi_mut(&mut self) -> &mut T {
        &mut self.wifi
    }

    /// As per [`WifiDriver::get_capabilities()`]
    pub fn get_capabilities(&self) -> Result<EnumSet<Capability>, EspError> {
        self.wifi.get_capabilities()
    }

    /// As per [`WifiDriver::get_configuration()`]
    pub fn get_configuration(&self) -> Result<Configuration, EspError> {
        self.wifi.get_configuration()
    }

    /// As per [`WifiDriver::set_configuration()`]
    pub fn set_configuration(&mut self, conf: &Configuration) -> Result<(), EspError> {
        self.wifi.set_configuration(conf)
    }

    /// As per [`WifiDriver::is_started()`]
    pub fn is_started(&self) -> Result<bool, EspError> {
        self.wifi.is_started()
    }

    /// As per [`WifiDriver::is_connected()`]
    pub fn is_connected(&self) -> Result<bool, EspError> {
        self.wifi.is_connected()
    }

    /// As per [`WifiDriver::start()`], but as an async call that awaits until
    /// the wifi driver has started.
    pub async fn start(&mut self) -> Result<(), EspError> {
        self.wifi.start()?;
        self.wifi_wait(|this| this.wifi.is_started().map(|s| !s), None)
            .await
    }

    /// As per [`WifiDriver::stop()`], but as an async call that awaits until
    /// the wifi driver has stopped.
    pub async fn stop(&mut self) -> Result<(), EspError> {
        self.wifi.stop()?;
        self.wifi_wait(|this| this.wifi.is_started(), None).await
    }

    /// As per [`WifiDriver::connect()`], but as an async call that awaits until
    /// the wifi driver has connected.
    pub async fn connect(&mut self) -> Result<(), EspError> {
        self.wifi.connect()?;
        self.wifi_wait(
            |this| this.wifi.is_connected().map(|s| !s),
            Some(CONNECT_TIMEOUT),
        )
        .await
    }

    /// As per [`WifiDriver::disconnect()`], but as an async call that awaits until
    /// the wifi driver has disconnected.
    pub async fn disconnect(&mut self) -> Result<(), EspError> {
        self.wifi.disconnect()?;
        self.wifi_wait(|this| this.wifi.is_connected(), None).await
    }

    /// As per [`WifiDriver::start_scan()`] plus [`WifiDriver::get_scan_result_n()`],
    /// as an async call that awaits until the scan is complete.
    pub async fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), EspError> {
        self.wifi.start_scan(&Default::default(), false)?;

        self.wifi_wait(|this| this.wifi.is_scan_done().map(|s| !s), None)
            .await?;

        self.wifi.get_scan_result_n()
    }

    /// As per [`WifiDriver::start_scan()`] plus [`WifiDriver::get_scan_result()`],
    /// as an async call that awaits until the scan is complete.
    #[cfg(feature = "alloc")]
    pub async fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, EspError> {
        self.wifi.start_scan(&Default::default(), false)?;

        self.wifi_wait(|this| this.wifi.is_scan_done().map(|s| !s), None)
            .await?;

        self.wifi.get_scan_result()
    }

    /// Awaits for a certain condition provided by the user in the form of a
    /// `matcher` callback to become false. Most often than not that condition
    /// would be related to the state of the Wifi driver. In other words,
    /// whether the driver is started, stopped, (dis)connected and so on.
    ///
    /// Note that the waiting is not done internally using busy-looping and/or
    /// timeouts. Rather, the condition (`matcher`) is evaluated initially, and
    /// if it returns `true`, it is re-evaluated each time the ESP IDF C Wifi
    /// driver posts a Wifi event on the system event loop. The reasoning behind
    /// this is that changes to the state of the Wifi driver are always
    /// accompanied by posting Wifi events.
    pub async fn wifi_wait<F: FnMut(&mut Self) -> Result<bool, EspError>>(
        &mut self,
        mut matcher: F,
        timeout: Option<Duration>,
    ) -> Result<(), EspError> {
        let mut wait = crate::eventloop::AsyncWait::<WifiEvent, _>::new(
            &self.event_loop,
            &self.timer_service,
        )?;

        wait.wait_while(|| matcher(self), timeout).await
    }

    /// Start WPS and perform a wait asynchronously until it connects, fails or
    /// times out. A [`WpsStatus`] is returned that contains the success status
    /// of the WPS connection. When the credentials of only one access point are
    /// received, the WiFi driver configuration will automatically be set to
    /// that access point. If multiple credentials were received, a
    /// `WpsStatus::Success` will be returned with a vector containing those
    /// credentials. The caller must then handle those credentials and set the
    /// configuration manually.
    pub async fn start_wps(&mut self, config: &WpsConfig<'_>) -> Result<WpsStatus, EspError> {
        self.wifi.start_wps(config)?;
        self.wifi_wait(
            |this| this.wifi.is_wps_finished().map(|x| !x),
            Some(WPS_TIMEOUT),
        )
        .await?;
        Ok(self.wifi.stop_wps().unwrap_or(WpsStatus::Timeout))
    }
}

#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<T> AsyncWifi<T>
where
    T: NetifStatus,
{
    /// As per [`EspWifi::is_up()`].
    pub fn is_up(&self) -> Result<bool, EspError> {
        self.wifi.is_up()
    }

    /// Waits until the underlaying network interface is up.
    pub async fn wait_netif_up(&mut self) -> Result<(), EspError> {
        self.ip_wait_while(|this| this.wifi.is_up().map(|s| !s), Some(CONNECT_TIMEOUT))
            .await
    }

    /// As [`AsyncWifi::wifi_wait()`], but for `EspWifi` events related to the
    /// IP layer, instead of `WifiDriver` events on the data link layer.
    pub async fn ip_wait_while<F: FnMut(&mut Self) -> Result<bool, EspError>>(
        &mut self,
        mut matcher: F,
        timeout: Option<core::time::Duration>,
    ) -> Result<(), EspError> {
        let mut wait =
            crate::eventloop::AsyncWait::<IpEvent, _>::new(&self.event_loop, &self.timer_service)?;

        wait.wait_while(|| matcher(self), timeout).await
    }
}

#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
impl<T> embedded_svc::wifi::asynch::Wifi for AsyncWifi<T>
where
    T: Wifi<Error = EspError> + NonBlocking,
{
    type Error = T::Error;

    async fn get_capabilities(&self) -> Result<EnumSet<Capability>, Self::Error> {
        AsyncWifi::get_capabilities(self)
    }

    async fn get_configuration(&self) -> Result<Configuration, Self::Error> {
        AsyncWifi::get_configuration(self)
    }

    async fn set_configuration(&mut self, conf: &Configuration) -> Result<(), Self::Error> {
        AsyncWifi::set_configuration(self, conf)
    }

    async fn start(&mut self) -> Result<(), Self::Error> {
        AsyncWifi::start(self).await
    }

    async fn stop(&mut self) -> Result<(), Self::Error> {
        AsyncWifi::stop(self).await
    }

    async fn connect(&mut self) -> Result<(), Self::Error> {
        AsyncWifi::connect(self).await
    }

    async fn disconnect(&mut self) -> Result<(), Self::Error> {
        AsyncWifi::disconnect(self).await
    }

    async fn is_started(&self) -> Result<bool, Self::Error> {
        AsyncWifi::is_started(self)
    }

    async fn is_connected(&self) -> Result<bool, Self::Error> {
        AsyncWifi::is_connected(self)
    }

    async fn scan_n<const N: usize>(
        &mut self,
    ) -> Result<(heapless::Vec<AccessPointInfo, N>, usize), Self::Error> {
        AsyncWifi::scan_n(self).await
    }

    #[cfg(feature = "alloc")]
    async fn scan(&mut self) -> Result<alloc::vec::Vec<AccessPointInfo>, Self::Error> {
        AsyncWifi::scan(self).await
    }
}

#[cfg(esp_idf_comp_esp_netif_enabled)]
impl crate::netif::asynch::NetifStatus for EspWifi<'_> {
    async fn is_up(&self) -> Result<bool, EspError> {
        EspWifi::is_up(self)
    }
}

#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
#[cfg(esp_idf_comp_esp_netif_enabled)]
impl<T> crate::netif::asynch::NetifStatus for AsyncWifi<T>
where
    T: NetifStatus,
{
    async fn is_up(&self) -> Result<bool, EspError> {
        AsyncWifi::is_up(self)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum WifiStaStatus {
    Stopped,
    Started,
    Connected,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum WifiApStatus {
    Stopped,
    Started,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum WifiScanStatus {
    Idle,
    Started,
    Done,
}

#[derive(Debug)]
pub struct WpsConfig<'a> {
    pub wps_type: WpsType,
    pub factory_info: WpsFactoryInfo<'a>,
}

impl TryFrom<&WpsConfig<'_>> for Newtype<esp_wps_config_t> {
    type Error = EspError;

    fn try_from(config: &WpsConfig<'_>) -> Result<Self, Self::Error> {
        let factory_info = Newtype::<wps_factory_information_t>::try_from(&config.factory_info)?.0;
        Ok(Newtype(esp_wps_config_t {
            wps_type: config.wps_type.as_raw_type(),
            factory_info,
            #[cfg(not(esp_idf_version_major = "4"))]
            pin: config.wps_type.as_pin(),
        }))
    }
}

#[derive(Clone, Debug)]
pub struct WpsFactoryInfo<'a> {
    pub manufacturer: &'a str,
    pub model_number: &'a str,
    pub model_name: &'a str,
    pub device_name: &'a str,
}

impl TryFrom<&WpsFactoryInfo<'_>> for Newtype<wps_factory_information_t> {
    type Error = EspError;

    fn try_from(info: &WpsFactoryInfo<'_>) -> Result<Self, Self::Error> {
        let mut result = Newtype(wps_factory_information_t {
            manufacturer: [0; 65],
            model_number: [0; 33],
            model_name: [0; 33],
            device_name: [0; 33],
        });

        set_str(
            c_char_to_u8_slice_mut(&mut result.0.manufacturer),
            info.manufacturer,
        )?;
        set_str(
            c_char_to_u8_slice_mut(&mut result.0.model_number),
            info.model_number,
        )?;
        set_str(
            c_char_to_u8_slice_mut(&mut result.0.model_name),
            info.model_name,
        )?;
        set_str(
            c_char_to_u8_slice_mut(&mut result.0.device_name),
            info.device_name,
        )?;

        Ok(result)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WpsType {
    Pbc,
    Pin(u32),
}

impl WpsType {
    fn as_raw_type(&self) -> wps_type_t {
        match self {
            WpsType::Pbc => wps_type_WPS_TYPE_PBC,
            WpsType::Pin(_) => wps_type_WPS_TYPE_PIN,
        }
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn as_pin(&self) -> [ffi::c_char; 9] {
        match self {
            WpsType::Pbc => [0; 9],
            WpsType::Pin(pin) => {
                let mut result = [0; 9];
                let mut pin = *pin;
                let mut rem: u32;
                for i in 0..8 {
                    rem = pin % 10;
                    pin /= 10;
                    result[7 - i] = (rem as ffi::c_char) + 48;
                }
                result
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum WpsStatus {
    SuccessConnected,
    SuccessMultipleAccessPoints(alloc::vec::Vec<WpsCredentials>),
    Failure,
    Timeout,
    Pin(Option<u32>),
    PbcOverlap,
}

impl TryFrom<&WifiEvent<'_>> for WpsStatus {
    type Error = EspError;

    fn try_from(event: &WifiEvent) -> Result<Self, Self::Error> {
        match event {
            WifiEvent::StaWpsSuccess(credentials) => {
                if credentials.is_empty() {
                    Ok(WpsStatus::SuccessConnected)
                } else {
                    Ok(WpsStatus::SuccessMultipleAccessPoints(
                        credentials
                            .iter()
                            .filter_map(|c| c.try_into().ok())
                            .collect::<alloc::vec::Vec<WpsCredentials>>(),
                    ))
                }
            }
            WifiEvent::StaWpsFailed => Ok(WpsStatus::Failure),
            WifiEvent::StaWpsTimeout => Ok(WpsStatus::Timeout),
            WifiEvent::StaWpsPin(pin) => Ok(WpsStatus::Pin(*pin)),
            WifiEvent::StaWpsPbcOverlap => Ok(WpsStatus::PbcOverlap),
            _ => Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>()),
        }
    }
}
