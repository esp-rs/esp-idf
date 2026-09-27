use core::cell::UnsafeCell;
use core::fmt::{self, Debug};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::boxed::Box;
use alloc::sync::Arc;

use ::log::info;

use num_enum::TryFromPrimitive;

use crate::hal::modem::BluetoothModemPeripheral;

use crate::private::mutex::{self, Mutex};
use crate::sys::*;

#[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
use crate::nvs::EspDefaultNvsPartition;

extern crate alloc;

#[cfg(all(
    any(esp32, esp32s31),
    esp_idf_bt_classic_enabled,
    esp_idf_bt_a2dp_enable
))]
pub mod a2dp;
#[cfg(all(
    any(esp32, esp32s31),
    esp_idf_bt_classic_enabled,
    esp_idf_bt_a2dp_enable
))]
pub mod avrc;
#[cfg(esp_idf_bt_ble_enabled)]
pub mod ble;
#[cfg(all(any(esp32, esp32s31), esp_idf_bt_classic_enabled))]
pub mod gap;
#[cfg(all(
    any(esp32, esp32s31),
    esp_idf_bt_classic_enabled,
    esp_idf_bt_hfp_enable
))]
pub mod hfp;
#[cfg(all(
    any(esp32, esp32s31),
    esp_idf_bt_classic_enabled,
    esp_idf_bt_spp_enabled
))]
pub mod spp;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct BdAddr(esp_bd_addr_t);

impl BdAddr {
    pub const fn raw(&self) -> &esp_bd_addr_t {
        &self.0
    }

    pub const fn from_bytes(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const fn addr(&self) -> [u8; 6] {
        self.0
    }
}

impl fmt::Display for BdAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl From<BdAddr> for esp_bd_addr_t {
    fn from(value: BdAddr) -> Self {
        value.0
    }
}

impl From<esp_bd_addr_t> for BdAddr {
    fn from(value: esp_bd_addr_t) -> Self {
        Self(value)
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct BtUuid(esp_bt_uuid_t);

impl BtUuid {
    pub const fn raw(&self) -> &esp_bt_uuid_t {
        &self.0
    }

    pub const fn into_raw(self) -> esp_bt_uuid_t {
        self.0
    }

    pub const fn uuid16(uuid: u16) -> Self {
        let esp_uuid = esp_bt_uuid_t {
            len: 2,
            uuid: esp_bt_uuid_t__bindgen_ty_1 { uuid16: uuid },
        };

        Self(esp_uuid)
    }

    pub const fn uuid32(uuid: u32) -> Self {
        let esp_uuid = esp_bt_uuid_t {
            len: 4,
            uuid: esp_bt_uuid_t__bindgen_ty_1 { uuid32: uuid },
        };

        Self(esp_uuid)
    }

    pub const fn uuid128(uuid: u128) -> Self {
        let esp_uuid = esp_bt_uuid_t {
            len: 16,
            uuid: esp_bt_uuid_t__bindgen_ty_1 {
                uuid128: uuid.to_le_bytes(),
            },
        };

        Self(esp_uuid)
    }

    pub const fn as_bytes(&self) -> &[u8] {
        match self.0.len {
            2 => unsafe {
                core::slice::from_raw_parts(&self.0.uuid.uuid128 as *const _ as *const _, 2)
            },
            4 => unsafe {
                core::slice::from_raw_parts(&self.0.uuid.uuid128 as *const _ as *const _, 4)
            },
            16 => unsafe { &self.0.uuid.uuid128 },
            _ => unreachable!(),
        }
    }
}

impl Debug for BtUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BtUuid {{{:?}}}", self.as_bytes())
    }
}

impl PartialEq for BtUuid {
    fn eq(&self, other: &BtUuid) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for BtUuid {}

impl From<BtUuid> for esp_bt_uuid_t {
    fn from(uuid: BtUuid) -> Self {
        uuid.0
    }
}

impl From<esp_bt_uuid_t> for BtUuid {
    fn from(uuid: esp_bt_uuid_t) -> Self {
        Self(uuid)
    }
}

#[allow(dead_code)]
#[allow(clippy::type_complexity)]
pub(crate) struct BtSingleton<A, R> {
    initialized: AtomicBool,
    callback: Mutex<Option<Arc<UnsafeCell<Box<dyn FnMut(A) -> R>>>>>,
    default_result: R,
}

#[allow(dead_code)]
impl<A, R> BtSingleton<A, R>
where
    R: Clone,
{
    pub const fn new(default_result: R) -> Self {
        Self {
            initialized: AtomicBool::new(false),
            callback: Mutex::new(None),
            default_result,
        }
    }

    pub fn release(&self) -> Result<(), EspError> {
        self.unsubscribe();

        self.initialized
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;

        Ok(())
    }

    pub fn take(&self) -> Result<(), EspError> {
        self.initialized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;

        Ok(())
    }

    pub fn subscribe<'d, F>(&self, callback: F)
    where
        F: FnMut(A) -> R + Send + 'd,
    {
        let callback = unsafe {
            core::mem::transmute::<
                Box<dyn FnMut(A) -> R + Send + 'd>,
                Box<dyn FnMut(A) -> R + Send + 'static>,
            >(Box::new(callback))
        };

        *self.callback.lock() = Some(Arc::new(UnsafeCell::new(callback)));
    }

    pub fn unsubscribe(&self) {
        *self.callback.lock() = None;
    }

    /// Safe to use only from within the ESP IDF Bluedroid task
    pub unsafe fn call(&self, arg: A) -> R {
        if let Some(callback) = self
            .callback
            .lock()
            .as_ref()
            .map(|callback| callback.clone())
        {
            ((callback.get()).as_mut().unwrap())(arg)
        } else {
            self.default_result.clone()
        }
    }
}

unsafe impl<A, R> Sync for BtSingleton<A, R> {}
unsafe impl<A, R> Send for BtSingleton<A, R> {}

pub trait BtMode: Send {
    fn mode() -> esp_bt_mode_t;
}

pub trait BleEnabled: BtMode {}
pub trait BtClassicEnabled: BtMode {}

#[cfg(any(esp32, esp32s31))]
#[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
#[derive(Clone)]
pub struct BtClassic(());
#[cfg(any(esp32, esp32s31))]
#[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
impl BtClassicEnabled for BtClassic {}

#[cfg(any(esp32, esp32s31))]
#[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
impl BtMode for BtClassic {
    fn mode() -> esp_bt_mode_t {
        #[cfg(not(esp_idf_btdm_ctrl_mode_btdm))]
        let mode = esp_bt_mode_t_ESP_BT_MODE_CLASSIC_BT;

        #[cfg(esp_idf_btdm_ctrl_mode_btdm)]
        let mode = esp_bt_mode_t_ESP_BT_MODE_BTDM;

        mode
    }
}

#[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
#[derive(Clone)]
pub struct Ble(());
#[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
impl BleEnabled for Ble {}

#[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
impl BtMode for Ble {
    fn mode() -> esp_bt_mode_t {
        #[cfg(not(esp_idf_btdm_ctrl_mode_btdm))]
        let mode = esp_bt_mode_t_ESP_BT_MODE_BLE;

        #[cfg(esp_idf_btdm_ctrl_mode_btdm)]
        let mode = esp_bt_mode_t_ESP_BT_MODE_BTDM;

        mode
    }
}

#[cfg(any(esp32, esp32s31))]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
#[derive(Clone)]
pub struct BtDual(());
#[cfg(any(esp32, esp32s31))]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
impl BtClassicEnabled for BtDual {}
#[cfg(any(esp32, esp32s31))]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
impl BleEnabled for BtDual {}

#[cfg(any(esp32, esp32s31))]
#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
impl BtMode for BtDual {
    fn mode() -> esp_bt_mode_t {
        esp_bt_mode_t_ESP_BT_MODE_BTDM
    }
}

// The esp32s31 controller has no PCM/I2S SCO data path (ESP-IDF v6.1 comments it out in Kconfig)
// and Bluedroid does not set the data path on non-esp32 chips, so the Kconfig default (PCM)
// compiles but never carries audio.
#[cfg(all(
    esp32s31,
    esp_idf_bt_hfp_enable,
    not(esp_idf_bt_hfp_audio_data_path_hci)
))]
compile_error!(
    "esp32s31: HFP needs CONFIG_BT_HFP_AUDIO_DATA_PATH_HCI=y (the controller has no PCM SCO \
     data path)"
);

#[derive(Debug, Copy, Clone, Eq, PartialEq, TryFromPrimitive)]
#[repr(u32)]
pub enum BtStatus {
    Success = esp_bt_status_t_ESP_BT_STATUS_SUCCESS,
    Fail = esp_bt_status_t_ESP_BT_STATUS_FAIL,
    NotReady = esp_bt_status_t_ESP_BT_STATUS_NOT_READY,
    NoMem = esp_bt_status_t_ESP_BT_STATUS_NOMEM,
    Busy = esp_bt_status_t_ESP_BT_STATUS_BUSY,
    Done = esp_bt_status_t_ESP_BT_STATUS_DONE,
    Unsupported = esp_bt_status_t_ESP_BT_STATUS_UNSUPPORTED,
    InvalidParam = esp_bt_status_t_ESP_BT_STATUS_PARM_INVALID,
    Unhandled = esp_bt_status_t_ESP_BT_STATUS_UNHANDLED,
    AuthFailure = esp_bt_status_t_ESP_BT_STATUS_AUTH_FAILURE,
    RemoteDeviceDown = esp_bt_status_t_ESP_BT_STATUS_RMT_DEV_DOWN,
    AuthRejected = esp_bt_status_t_ESP_BT_STATUS_AUTH_REJECTED,
    InvalidStaticRandAddr = esp_bt_status_t_ESP_BT_STATUS_INVALID_STATIC_RAND_ADDR,
    Pending = esp_bt_status_t_ESP_BT_STATUS_PENDING,
    UnacceptedConnInterval = esp_bt_status_t_ESP_BT_STATUS_UNACCEPT_CONN_INTERVAL,
    ParamOutOfRange = esp_bt_status_t_ESP_BT_STATUS_PARAM_OUT_OF_RANGE,
    Timeout = esp_bt_status_t_ESP_BT_STATUS_TIMEOUT,
    UnsupportedPeerLeDataLen = esp_bt_status_t_ESP_BT_STATUS_PEER_LE_DATA_LEN_UNSUPPORTED,
    UnsupportedControlLeDataLen = esp_bt_status_t_ESP_BT_STATUS_CONTROL_LE_DATA_LEN_UNSUPPORTED,
    IllegalParamFormat = esp_bt_status_t_ESP_BT_STATUS_ERR_ILLEGAL_PARAMETER_FMT,
    MemoryFull = esp_bt_status_t_ESP_BT_STATUS_MEMORY_FULL,
    EirTooLarge = esp_bt_status_t_ESP_BT_STATUS_EIR_TOO_LARGE,
    HciSuccess = esp_bt_status_t_ESP_BT_STATUS_HCI_SUCCESS,
    HciIllegalCommand = esp_bt_status_t_ESP_BT_STATUS_HCI_ILLEGAL_COMMAND,
    HciNoConnection = esp_bt_status_t_ESP_BT_STATUS_HCI_NO_CONNECTION,
    HciHwFailure = esp_bt_status_t_ESP_BT_STATUS_HCI_HW_FAILURE,
    HciPageTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_PAGE_TIMEOUT,
    HciAuthFailure = esp_bt_status_t_ESP_BT_STATUS_HCI_AUTH_FAILURE,
    HciKeyMissing = esp_bt_status_t_ESP_BT_STATUS_HCI_KEY_MISSING,
    HciMemoryFull = esp_bt_status_t_ESP_BT_STATUS_HCI_MEMORY_FULL,
    HciConnTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_CONNECTION_TOUT,
    HciConnectionsExhausted = esp_bt_status_t_ESP_BT_STATUS_HCI_MAX_NUM_OF_CONNECTIONS,
    HciScosExhausted = esp_bt_status_t_ESP_BT_STATUS_HCI_MAX_NUM_OF_SCOS,
    HciConnectionExists = esp_bt_status_t_ESP_BT_STATUS_HCI_CONNECTION_EXISTS,
    HciCommandDisallowed = esp_bt_status_t_ESP_BT_STATUS_HCI_COMMAND_DISALLOWED,
    HciHostResourcesRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_REJECT_RESOURCES,
    HciHostSecurityRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_REJECT_SECURITY,
    HciHostDevideRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_REJECT_DEVICE,
    HciHostTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_TIMEOUT,
    HciUnsupportedValue = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSUPPORTED_VALUE,
    HciIllegalParamFormat = esp_bt_status_t_ESP_BT_STATUS_HCI_ILLEGAL_PARAMETER_FMT,
    HciPeerUser = esp_bt_status_t_ESP_BT_STATUS_HCI_PEER_USER,
    HciPeerLowResources = esp_bt_status_t_ESP_BT_STATUS_HCI_PEER_LOW_RESOURCES,
    HciPeerPowerOff = esp_bt_status_t_ESP_BT_STATUS_HCI_PEER_POWER_OFF,
    HciConnectionCauseLocalHost = esp_bt_status_t_ESP_BT_STATUS_HCI_CONN_CAUSE_LOCAL_HOST,
    HciRepeatedAttempts = esp_bt_status_t_ESP_BT_STATUS_HCI_REPEATED_ATTEMPTS,
    HciPairingNotAllowed = esp_bt_status_t_ESP_BT_STATUS_HCI_PAIRING_NOT_ALLOWED,
    HciUnkownLmpPdu = esp_bt_status_t_ESP_BT_STATUS_HCI_UNKNOWN_LMP_PDU,
    HciUnsupportedRemFeature = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSUPPORTED_REM_FEATURE,
    HciScoOffsetRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_SCO_OFFSET_REJECTED,
    HciScoInternalRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_SCO_INTERVAL_REJECTED,
    HciScoAirMode = esp_bt_status_t_ESP_BT_STATUS_HCI_SCO_AIR_MODE,
    HciInvalidLmpParam = esp_bt_status_t_ESP_BT_STATUS_HCI_INVALID_LMP_PARAM,
    HciUnspecified = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSPECIFIED,
    HciUnsupportedLmpParameters = esp_bt_status_t_ESP_BT_STATUS_HCI_UNSUPPORTED_LMP_PARAMETERS,
    HciRoleChangeNotAllowed = esp_bt_status_t_ESP_BT_STATUS_HCI_ROLE_CHANGE_NOT_ALLOWED,
    HciLmpResponseTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_LMP_RESPONSE_TIMEOUT,
    HciLmpErrTransactionCollision = esp_bt_status_t_ESP_BT_STATUS_HCI_LMP_ERR_TRANS_COLLISION,
    HciLmpPduNotAllowed = esp_bt_status_t_ESP_BT_STATUS_HCI_LMP_PDU_NOT_ALLOWED,
    HciEntryModeNotAcceptable = esp_bt_status_t_ESP_BT_STATUS_HCI_ENCRY_MODE_NOT_ACCEPTABLE,
    HciUnitKeyUsed = esp_bt_status_t_ESP_BT_STATUS_HCI_UNIT_KEY_USED,
    HciUnsupportedQos = esp_bt_status_t_ESP_BT_STATUS_HCI_QOS_NOT_SUPPORTED,
    HciInstantPassed = esp_bt_status_t_ESP_BT_STATUS_HCI_INSTANT_PASSED,
    HciUnsupportedPairingWithUnitKey =
        esp_bt_status_t_ESP_BT_STATUS_HCI_PAIRING_WITH_UNIT_KEY_NOT_SUPPORTED,
    HciDiffTransactionCollision = esp_bt_status_t_ESP_BT_STATUS_HCI_DIFF_TRANSACTION_COLLISION,
    HciUndefined0x2b = esp_bt_status_t_ESP_BT_STATUS_HCI_UNDEFINED_0x2B,
    HciQosInvalidParam = esp_bt_status_t_ESP_BT_STATUS_HCI_QOS_UNACCEPTABLE_PARAM,
    HciQosRejected = esp_bt_status_t_ESP_BT_STATUS_HCI_QOS_REJECTED,
    HciUnsupportedChanClassification = esp_bt_status_t_ESP_BT_STATUS_HCI_CHAN_CLASSIF_NOT_SUPPORTED,
    HciInsufficientSecurity = esp_bt_status_t_ESP_BT_STATUS_HCI_INSUFFCIENT_SECURITY,
    HciParamOutOfRange = esp_bt_status_t_ESP_BT_STATUS_HCI_PARAM_OUT_OF_RANGE,
    HciUndefined0x31 = esp_bt_status_t_ESP_BT_STATUS_HCI_UNDEFINED_0x31,
    HciRoleSwitchPending = esp_bt_status_t_ESP_BT_STATUS_HCI_ROLE_SWITCH_PENDING,
    HciUndefined0x33 = esp_bt_status_t_ESP_BT_STATUS_HCI_UNDEFINED_0x33,
    HciReservedSlotViolation = esp_bt_status_t_ESP_BT_STATUS_HCI_RESERVED_SLOT_VIOLATION,
    HciRoleSwitchFailed = esp_bt_status_t_ESP_BT_STATUS_HCI_ROLE_SWITCH_FAILED,
    HciInqRespDataTooLarge = esp_bt_status_t_ESP_BT_STATUS_HCI_INQ_RSP_DATA_TOO_LARGE,
    HciSimplePairingNotSupported = esp_bt_status_t_ESP_BT_STATUS_HCI_SIMPLE_PAIRING_NOT_SUPPORTED,
    HciHostBusyPairing = esp_bt_status_t_ESP_BT_STATUS_HCI_HOST_BUSY_PAIRING,
    HciRejNoSuitableChannel = esp_bt_status_t_ESP_BT_STATUS_HCI_REJ_NO_SUITABLE_CHANNEL,
    HciControllerBusy = esp_bt_status_t_ESP_BT_STATUS_HCI_CONTROLLER_BUSY,
    HciUnsupportedConnectionInterval = esp_bt_status_t_ESP_BT_STATUS_HCI_UNACCEPT_CONN_INTERVAL,
    HciDirectedAdvertisingTimeout = esp_bt_status_t_ESP_BT_STATUS_HCI_DIRECTED_ADVERTISING_TIMEOUT,
    HciConnectionTimeoutDueToMiscFailure =
        esp_bt_status_t_ESP_BT_STATUS_HCI_CONN_TOUT_DUE_TO_MIC_FAILURE,
    HciConnectionEstablishmentFailed = esp_bt_status_t_ESP_BT_STATUS_HCI_CONN_FAILED_ESTABLISHMENT,
    HciMacConnectionFailed = esp_bt_status_t_ESP_BT_STATUS_HCI_MAC_CONNECTION_FAILED,
}

static MEM_FREED: mutex::Mutex<bool> = mutex::Mutex::new(false);

pub fn reduce_bt_memory<'d, B: BluetoothModemPeripheral + 'd>(_modem: B) -> Result<(), EspError> {
    let mut mem_freed = MEM_FREED.lock();

    if *mem_freed {
        Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;
    }

    #[cfg(esp_idf_btdm_ctrl_mode_br_edr_only)]
    esp!(unsafe { esp_bt_mem_release(esp_bt_mode_t_ESP_BT_MODE_BLE) })?;

    #[cfg(esp_idf_btdm_ctrl_mode_br_ble_only)]
    esp!(unsafe { esp_bt_mem_release(esp_bt_mode_t_ESP_BT_MODE_CLASSIC_BT) })?;

    *mem_freed = true;

    Ok(())
}

#[cfg(esp_idf_btdm_ctrl_mode_btdm)]
pub fn free_bt_memory<B: BluetoothModemPeripheral>(_modem: B) -> Result<(), EspError> {
    let mut mem_freed = MEM_FREED.lock();

    if *mem_freed {
        Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>())?;
    }

    esp!(unsafe { esp_bt_mem_release(esp_bt_mode_t_ESP_BT_MODE_BTDM) })?;

    *mem_freed = true;

    Ok(())
}

pub struct BtDriver<'d, M>
where
    M: BtMode,
{
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    _nvs: Option<EspDefaultNvsPartition>,
    _p: PhantomData<&'d mut ()>,
    _m: PhantomData<M>,
}

impl<'d, M> BtDriver<'d, M>
where
    M: BtMode,
{
    #[cfg(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled))]
    pub fn new<B: BluetoothModemPeripheral + 'd>(
        _modem: B,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Self::init(nvs.is_some())?;

        Ok(Self {
            _nvs: nvs,
            _p: PhantomData,
            _m: PhantomData,
        })
    }

    #[cfg(not(all(feature = "alloc", esp_idf_comp_nvs_flash_enabled)))]
    pub fn new<B: BluetoothModemPeripheral + 'd>(_modem: B) -> Result<Self, EspError> {
        Self::init(false)?;

        Ok(Self {
            _p: PhantomData,
            _m: PhantomData,
        })
    }

    #[allow(clippy::needless_update)]
    fn init(_nvs_enabled: bool) -> Result<(), EspError> {
        #[cfg(esp32)]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: crate::sys::ESP_BT_CONTROLLER_CONFIG_MAGIC_VAL,
            controller_task_stack_size: crate::sys::ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            hci_uart_no: crate::sys::BT_HCI_UART_NO_DEFAULT as _,
            hci_uart_baudrate: crate::sys::BT_HCI_UART_BAUDRATE_DEFAULT,
            scan_duplicate_mode: crate::sys::SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: crate::sys::SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: crate::sys::NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: crate::sys::MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            send_adv_reserved_size: crate::sys::SCAN_SEND_ADV_RESERVED_SIZE as _,
            controller_debug_flag: crate::sys::CONTROLLER_ADV_LOST_DEBUG_BIT,
            mode: M::mode() as _,
            ble_max_conn: crate::sys::CONFIG_BTDM_CTRL_BLE_MAX_CONN_EFF as _,
            bt_max_acl_conn: crate::sys::CONFIG_BTDM_CTRL_BR_EDR_MAX_ACL_CONN_EFF as _,
            bt_sco_datapath: crate::sys::CONFIG_BTDM_CTRL_BR_EDR_SCO_DATA_PATH_EFF as _,
            auto_latency: crate::sys::BTDM_CTRL_AUTO_LATENCY_EFF != 0,
            bt_legacy_auth_vs_evt: crate::sys::BTDM_CTRL_LEGACY_AUTH_VENDOR_EVT_EFF != 0,
            // See https://github.com/espressif/esp-idf/blob/12f36a021f511cd4de41d3fffff146c5336ac1e7/components/bt/controller/esp32/esp_bredr_cfg.h#L19
            // bt_max_sync_conn: crate::sys::CONFIG_BTDM_CTRL_BR_EDR_MAX_SYNC_CONN_EFF as _,
            bt_max_sync_conn: 1,
            ble_sca: crate::sys::CONFIG_BTDM_BLE_SLEEP_CLOCK_ACCURACY_INDEX_EFF as _,
            pcm_role: crate::sys::CONFIG_BTDM_CTRL_PCM_ROLE_EFF as _,
            pcm_polar: crate::sys::CONFIG_BTDM_CTRL_PCM_POLAR_EFF as _,
            hli: crate::sys::BTDM_CTRL_HLI != 0,
            #[cfg(any(
                esp_idf_version_patch_at_least_5_1_7,
                esp_idf_version_patch_at_least_5_2_6,
                esp_idf_version_patch_at_least_5_3_3,
                esp_idf_version_patch_at_least_5_4_1,
                esp_idf_version_at_least_5_5_0,
            ))]
            enc_key_sz_min: crate::sys::CONFIG_BTDM_CTRL_BR_EDR_MIN_ENC_KEY_SZ_DFT_EFF as _,
            dup_list_refresh_period: crate::sys::SCAN_DUPL_CACHE_REFRESH_PERIOD as _,
            ble_scan_backoff: crate::sys::BTDM_CTRL_SCAN_BACKOFF_UPPERLIMITMAX != 0,
            ble_llcp_disc_flag: crate::sys::BTDM_BLE_LLCP_DISC_FLAG as _,
            ble_aa_check: crate::sys::BTDM_CTRL_CHECK_CONNECT_IND_ACCESS_ADDRESS_ENABLED != 0,
            ble_chan_ass_en: crate::sys::BTDM_BLE_CHAN_ASS_EN as _,
            ble_ping_en: crate::sys::BTDM_BLE_PING_EN as _,
            ..Default::default()
        };

        // esp32s31: dual-mode "BTDM" controller port with a nested config.
        #[cfg(esp32s31)]
        let mut bt_cfg = esp_bt_controller_config_t {
            #[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
            ble: esp_bt_controller_config_t__bindgen_ty_1 {
                config_version: crate::sys::BLE_CONFIG_VERSION as _,
                ble_ll_resolv_list_size: crate::sys::CONFIG_BT_LE_LL_RESOLV_LIST_SIZE as _,
                ble_hci_evt_hi_buf_count: crate::sys::DEFAULT_BT_LE_HCI_EVT_HI_BUF_COUNT as _,
                ble_hci_evt_lo_buf_count: crate::sys::DEFAULT_BT_LE_HCI_EVT_LO_BUF_COUNT as _,
                ble_ll_sync_list_cnt: crate::sys::DEFAULT_BT_LE_MAX_PERIODIC_ADVERTISER_LIST as _,
                ble_ll_sync_cnt: crate::sys::DEFAULT_BT_LE_MAX_PERIODIC_SYNCS as _,
                ble_ll_rsp_dup_list_count: crate::sys::CONFIG_BT_LE_LL_DUP_SCAN_LIST_COUNT as _,
                ble_ll_adv_dup_list_count: crate::sys::CONFIG_BT_LE_LL_DUP_SCAN_LIST_COUNT as _,
                ble_ll_tx_pwr_dbm: 0,
                rtc_freq: 32000,
                ble_ll_sca: crate::sys::CONFIG_BT_LE_LL_SCA as _,
                ble_ll_scan_phy_number: crate::sys::BLE_LL_SCAN_PHY_NUMBER_N as _,
                ble_ll_conn_def_auth_pyld_tmo: crate::sys::BLE_LL_CONN_DEF_AUTH_PYLD_TMO_N as _,
                ble_ll_jitter_usecs: crate::sys::BLE_LL_JITTER_USECS_N as _,
                ble_ll_sched_max_adv_pdu_usecs: crate::sys::BLE_LL_SCHED_MAX_ADV_PDU_USECS_N as _,
                ble_ll_sched_direct_adv_max_usecs: crate::sys::BLE_LL_SCHED_DIRECT_ADV_MAX_USECS_N
                    as _,
                ble_ll_sched_adv_max_usecs: crate::sys::BLE_LL_SCHED_ADV_MAX_USECS_N as _,
                ble_scan_rsp_data_max_len: crate::sys::DEFAULT_BT_LE_SCAN_RSP_DATA_MAX_LEN_N as _,
                ble_ll_cfg_num_hci_cmd_pkts: crate::sys::BLE_LL_CFG_NUM_HCI_CMD_PKTS_N as _,
                ble_ll_ctrl_proc_timeout_ms: crate::sys::BLE_LL_CTRL_PROC_TIMEOUT_MS_N as _,
                nimble_max_connections: crate::sys::DEFAULT_BT_LE_MAX_CONNECTIONS as _,
                ble_whitelist_size: crate::sys::DEFAULT_BT_NIMBLE_WHITELIST_SIZE as _,
                ble_acl_buf_size: crate::sys::DEFAULT_BT_LE_ACL_BUF_SIZE as _,
                ble_acl_buf_count: crate::sys::DEFAULT_BT_LE_ACL_BUF_COUNT as _,
                ble_hci_evt_buf_size: crate::sys::DEFAULT_BT_LE_HCI_EVT_BUF_SIZE as _,
                ble_multi_adv_instances: crate::sys::DEFAULT_BT_LE_MAX_EXT_ADV_INSTANCES as _,
                ble_ext_adv_max_size: crate::sys::DEFAULT_BT_LE_EXT_ADV_MAX_SIZE as _,
                controller_task_stack_size: crate::sys::UC_BT_CTRL_TASK_STACK_SIZE as _,
                controller_task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
                controller_run_cpu: 0,
                enable_qa_test: crate::sys::RUN_QA_TEST as _,
                enable_bqb_test: crate::sys::RUN_BQB_TEST as _,
                enable_tx_cca: crate::sys::DEFAULT_BT_LE_TX_CCA_ENABLED as _,
                cca_rssi_thresh: (256 - crate::sys::DEFAULT_BT_LE_CCA_RSSI_THRESH) as _,
                sleep_en: crate::sys::UC_BT_CTRL_SLEEP_ENABLE as _,
                coex_phy_coded_tx_rx_time_limit:
                    crate::sys::DEFAULT_BT_LE_COEX_PHY_CODED_TX_RX_TLIM_EFF as _,
                dis_scan_backoff: crate::sys::NIMBLE_DISABLE_SCAN_BACKOFF as _,
                ble_scan_classify_filter_enable: 1,
                // `cca_drop_mode` and `cca_low_tx_pwr` are not set by the macro
                main_xtal_freq: crate::sys::CONFIG_XTAL_FREQ as _,
                ignore_wl_for_direct_adv: 0,
                enable_pcl: 0,
                csa2_select: 1,
                enable_csr: 0,
                backoff_rssi: -100,
                iso_enabled: crate::sys::DEFAULT_BT_LE_ISO_ENABLED != 0,
                iso_bqb_test: false,
                iso_fra_unseg: crate::sys::DEFAULT_BT_LE_ISO_FRA_UNSEG != 0,
                iso_nsfc_en: crate::sys::DEFAULT_BT_LE_ISO_NSFC_EN != 0,
                iso_nsfc_num: crate::sys::DEFAULT_BT_LE_ISO_NSFC_NUM as _,
                iso_buf_count: crate::sys::DEFAULT_BT_LE_ISO_BUF_COUNT as _,
                iso_buf_size: crate::sys::DEFAULT_BT_LE_ISO_BUF_SIZE as _,
                iso_big_count: crate::sys::DEFAULT_BT_LE_ISO_BIG as _,
                iso_bis_count: crate::sys::DEFAULT_BT_LE_ISO_BIS as _,
                iso_bis_per_big: crate::sys::DEFAULT_BT_LE_ISO_BIS_PER_BIG as _,
                iso_cig_count: crate::sys::DEFAULT_BT_LE_ISO_CIG as _,
                iso_cis_count: crate::sys::DEFAULT_BT_LE_ISO_CIS as _,
                iso_cis_per_cig: crate::sys::DEFAULT_BT_LE_ISO_CIS_PER_CIG as _,
                config_magic: crate::sys::BLE_CONFIG_MAGIC as _,
                ..Default::default()
            },
            #[cfg(esp_idf_btdm_ctrl_mode_br_edr_only)]
            ble: Default::default(),
            #[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
            bredr: esp_bredr_controller_config_t {
                bredr_version: crate::sys::ESP_BREDR_CTRL_CONFIG_VERSION as _,
                sleep_mode: 0,
                bt_test_mode_en: crate::sys::UC_BR_EDR_TEST_MODE_EN as _,
                acl_cca_en: crate::sys::UC_BR_EDR_TX_CCA_EN as _,
                cca_rssi_thr: -(crate::sys::UC_BR_EDR_CCA_RSSI_THRESH as i8),
                max_acl_conn: crate::sys::UC_BR_EDR_MAX_ACL_CONN as _,
                max_sync_conn: crate::sys::UC_BR_EDR_CTRL_MAX_SYNC_CONN as _,
                cpb_tx_link_num: crate::sys::UC_BR_EDR_CPB_TX_LINK_NB as _,
                cpb_rx_link_num: crate::sys::UC_BR_EDR_CPB_RX_LINK_NB as _,
                bt_sco_datapath: crate::sys::UC_BR_EDR_SCO_DATA_PATH as _,
                hci_tl_type: 0,
                hci_tl_funcs: core::ptr::null_mut(),
                cfg_mask: 0,
                hw_target_code: 0,
                // The macro uses the `CONFIG_` names (not `UC_`) for these five
                acl_min_tx_pwr: crate::sys::CONFIG_BT_CTRL_BR_EDR_TX_PWR_ACL_MIN as _,
                acl_max_tx_pwr: crate::sys::CONFIG_BT_CTRL_BR_EDR_TX_PWR_ACL_MAX as _,
                apb_tx_pwr: crate::sys::UC_BR_EDR_TX_PWR_APB as _,
                page_tx_pwr: crate::sys::CONFIG_BT_CTRL_BR_EDR_TX_PWR_PAGE as _,
                pscan_tx_pwr: crate::sys::CONFIG_BT_CTRL_BR_EDR_TX_PWR_PSCAN as _,
                iscan_tx_pwr: crate::sys::CONFIG_BT_CTRL_BR_EDR_TX_PWR_ISCAN as _,
                cpb_tx_pwr: crate::sys::UC_BR_EDR_TX_PWR_CPB as _,
                strain_tx_pwr: crate::sys::UC_BR_EDR_TX_PWR_STRAIN as _,
                rf_hw_type: 0,
                static_aclu_tx_buf_nb: (crate::sys::UC_BR_EDR_ACLU_TX_BUF_NB
                    - crate::sys::UC_BR_EDR_DYNAMIC_ACLU_TX_BUF_NB)
                    as _,
                dynamic_aclu_tx_buf_nb: crate::sys::UC_BR_EDR_DYNAMIC_ACLU_TX_BUF_NB as _,
                aclu_rx_buf_nb: crate::sys::UC_BR_EDR_ACLU_RX_BUF_NB as _,
                sync_tx_buf_nb: crate::sys::UC_BR_EDR_SYNC_TX_BUF_NB as _,
                sync_rx_buf_nb: crate::sys::UC_BR_EDR_SYNC_RX_BUF_NB as _,
                _bitfield_1: esp_bredr_controller_config_t::new_bitfield_1(
                    crate::sys::UC_BR_EDR_ESCO_EV4_SUPP as _,
                    crate::sys::UC_BR_EDR_ESCO_EV5_SUPP as _,
                    crate::sys::UC_BR_EDR_ESCO_EV3_2_SUPP as _,
                    crate::sys::UC_BR_EDR_ESCO_EV3_3_SUPP as _,
                    crate::sys::UC_BR_EDR_ESCO_3_SLOTS_SUPP as _,
                    crate::sys::UC_BR_EDR_LEGACY_AUTH_VENDOR_EVT as _,
                    0, // inq_filter_en
                    0, // dtm_en
                ),
                // `mempool_size` and `mempool_ops` are not set by the macro
                bredr_magic: crate::sys::ESP_BREDR_CTRL_CONFIG_MAGIC_VAL as _,
                ..Default::default()
            },
            #[cfg(esp_idf_btdm_ctrl_mode_ble_only)]
            bredr: Default::default(),
            btdm: esp_btdm_controller_config_t {
                version: crate::sys::BTDM_CONFIG_VERSION as _,
                task_stack_size: crate::sys::UC_BT_CTRL_TASK_STACK_SIZE as _,
                task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
                task_run_cpu: crate::sys::CONFIG_BT_CTRL_PINNED_TO_CORE as _,
                hci_cmd_num: crate::sys::CONFIG_BT_CTRL_HCI_CMD_NUM as _,
                // The macro sums the connection counts of the enabled halves
                #[allow(clippy::unnecessary_cast)]
                hci_conn_num: {
                    let mut n = 0u32;
                    #[cfg(not(esp_idf_btdm_ctrl_mode_br_edr_only))]
                    {
                        n += crate::sys::DEFAULT_BT_LE_MAX_CONNECTIONS as u32
                            + crate::sys::DEFAULT_BT_LE_ISO_CIS as u32
                            + crate::sys::DEFAULT_BT_LE_ISO_BIS as u32;
                    }
                    #[cfg(not(esp_idf_btdm_ctrl_mode_ble_only))]
                    {
                        n += crate::sys::UC_BR_EDR_MAX_ACL_CONN as u32
                            + crate::sys::UC_BR_EDR_CTRL_MAX_SYNC_CONN as u32;
                    }
                    n as _
                },
                sleep_en: crate::sys::UC_BT_CTRL_SLEEP_ENABLE as _,
                version_num: 0,
                bluetooth_mode: M::mode() as _,
                magic: crate::sys::BTDM_CONFIG_MAGIC_VALUE as _,
            },
        };

        #[cfg(any(esp32c3, esp32s3))]
        let mut bt_cfg = esp_bt_controller_config_t {
            magic: crate::sys::ESP_BT_CTRL_CONFIG_MAGIC_VAL,
            version: crate::sys::ESP_BT_CTRL_CONFIG_VERSION,
            controller_task_stack_size: crate::sys::ESP_TASK_BT_CONTROLLER_STACK as _,
            controller_task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            controller_task_run_cpu: crate::sys::CONFIG_BT_CTRL_PINNED_TO_CORE as _,
            bluetooth_mode: crate::sys::CONFIG_BT_CTRL_MODE_EFF as _,
            ble_max_act: crate::sys::CONFIG_BT_CTRL_BLE_MAX_ACT_EFF as _,
            sleep_mode: crate::sys::CONFIG_BT_CTRL_SLEEP_MODE_EFF as _,
            sleep_clock: crate::sys::CONFIG_BT_CTRL_SLEEP_CLOCK_EFF as _,
            ble_st_acl_tx_buf_nb: crate::sys::CONFIG_BT_CTRL_BLE_STATIC_ACL_TX_BUF_NB as _,
            ble_hw_cca_check: crate::sys::CONFIG_BT_CTRL_HW_CCA_EFF as _,
            ble_adv_dup_filt_max: crate::sys::CONFIG_BT_CTRL_ADV_DUP_FILT_MAX as _,
            ce_len_type: crate::sys::CONFIG_BT_CTRL_CE_LENGTH_TYPE_EFF as _,
            hci_tl_type: crate::sys::CONFIG_BT_CTRL_HCI_TL_EFF as _,
            hci_tl_funcs: core::ptr::null_mut(),
            txant_dft: crate::sys::CONFIG_BT_CTRL_TX_ANTENNA_INDEX_EFF as _,
            rxant_dft: crate::sys::CONFIG_BT_CTRL_RX_ANTENNA_INDEX_EFF as _,
            txpwr_dft: crate::sys::CONFIG_BT_CTRL_DFT_TX_POWER_LEVEL_EFF as _,
            cfg_mask: crate::sys::CFG_MASK,
            scan_duplicate_mode: crate::sys::SCAN_DUPLICATE_MODE as _,
            scan_duplicate_type: crate::sys::SCAN_DUPLICATE_TYPE_VALUE as _,
            normal_adv_size: crate::sys::NORMAL_SCAN_DUPLICATE_CACHE_SIZE as _,
            mesh_adv_size: crate::sys::MESH_DUPLICATE_SCAN_CACHE_SIZE as _,
            coex_phy_coded_tx_rx_time_limit:
                crate::sys::CONFIG_BT_CTRL_COEX_PHY_CODED_TX_RX_TLIM_EFF as _,
            hw_target_code: crate::sys::BLE_HW_TARGET_CODE_CHIP_ECO0 as _,
            slave_ce_len_min: crate::sys::SLAVE_CE_LEN_MIN_DEFAULT as _,
            hw_recorrect_en: crate::sys::AGC_RECORRECT_EN as _,
            cca_thresh: crate::sys::CONFIG_BT_CTRL_HW_CCA_VAL as _,
            coex_param_en: false,
            coex_use_hooks: false,
            scan_backoff_upperlimitmax: crate::sys::BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as _,
            dup_list_refresh_period: crate::sys::DUPL_SCAN_CACHE_REFRESH_PERIOD as _,
            ble_50_feat_supp: crate::sys::BT_CTRL_50_FEATURE_SUPPORT != 0,
            ble_cca_mode: crate::sys::BT_BLE_CCA_MODE as _,
            ble_data_lenth_zero_aux: crate::sys::BT_BLE_ADV_DATA_LENGTH_ZERO_AUX as _,
            ble_chan_ass_en: crate::sys::BT_CTRL_CHAN_ASS_EN as _,
            ble_ping_en: crate::sys::BT_CTRL_LE_PING_EN as _,
            ble_llcp_disc_flag: crate::sys::BT_CTRL_BLE_LLCP_DISC_FLAG as _,
            run_in_flash: crate::sys::BT_CTRL_RUN_IN_FLASH_ONLY != 0,
            dtm_en: crate::sys::BT_CTRL_DTM_ENABLE != 0,
            enc_en: crate::sys::BLE_SECURITY_ENABLE != 0,
            qa_test: crate::sys::BT_CTRL_BLE_TEST != 0,
            #[cfg(any(
                esp_idf_version_patch_at_least_5_1_7,
                esp_idf_version_patch_at_least_5_2_6,
                esp_idf_version_patch_at_least_5_3_4,
                esp_idf_version_patch_at_least_5_4_2,
                esp_idf_version_at_least_5_5_0,
            ))]
            connect_en: crate::sys::BT_CTRL_BLE_MASTER != 0,
            scan_en: crate::sys::BT_CTRL_BLE_SCAN != 0,
            ble_aa_check: crate::sys::BLE_CTRL_CHECK_CONNECT_IND_ACCESS_ADDRESS_ENABLED != 0,
            #[cfg(any(
                all(
                    esp_idf_version_patch_at_least_5_2_6,
                    esp_idf_version_patch_at_most_5_2_6
                ),
                all(
                    esp_idf_version_patch_at_least_5_3_3,
                    esp_idf_version_patch_at_most_5_3_4
                ),
                all(
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_patch_at_most_5_4_3
                ),
                all(
                    esp_idf_version_patch_at_least_5_5_0,
                    esp_idf_version_patch_at_most_5_5_1
                ),
            ))]
            ble_log_mode_en: crate::sys::BLE_LOG_MODE_EN,
            #[cfg(any(
                all(
                    esp_idf_version_patch_at_least_5_2_6,
                    esp_idf_version_patch_at_most_5_2_6
                ),
                all(
                    esp_idf_version_patch_at_least_5_3_3,
                    esp_idf_version_patch_at_most_5_3_4
                ),
                all(
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_patch_at_most_5_4_3
                ),
                all(
                    esp_idf_version_patch_at_least_5_5_0,
                    esp_idf_version_patch_at_most_5_5_1
                ),
            ))]
            ble_log_level: crate::sys::BLE_LOG_LEVEL as _,
            #[cfg(any(
                esp_idf_version_patch_at_least_5_1_7,
                esp_idf_version_patch_at_least_5_2_6,
                esp_idf_version_patch_at_least_5_3_4,
                esp_idf_version_patch_at_least_5_4_2,
                esp_idf_version_at_least_5_5_0,
            ))]
            adv_en: crate::sys::BT_CTRL_BLE_ADV != 0,
            ..Default::default()
        };

        #[cfg(not(any(esp32, esp32s3, esp32c3, esp32s31)))]
        let mut bt_cfg = esp_bt_controller_config_t {
            config_version: CONFIG_VERSION as _,
            ble_ll_resolv_list_size: crate::sys::CONFIG_BT_LE_LL_RESOLV_LIST_SIZE as _,
            ble_hci_evt_hi_buf_count: crate::sys::DEFAULT_BT_LE_HCI_EVT_HI_BUF_COUNT as _,
            ble_hci_evt_lo_buf_count: crate::sys::DEFAULT_BT_LE_HCI_EVT_LO_BUF_COUNT as _,
            ble_ll_sync_list_cnt: crate::sys::DEFAULT_BT_LE_MAX_PERIODIC_ADVERTISER_LIST as _,
            ble_ll_sync_cnt: crate::sys::DEFAULT_BT_LE_MAX_PERIODIC_SYNCS as _,
            ble_ll_rsp_dup_list_count: crate::sys::CONFIG_BT_LE_LL_DUP_SCAN_LIST_COUNT as _,
            ble_ll_adv_dup_list_count: crate::sys::CONFIG_BT_LE_LL_DUP_SCAN_LIST_COUNT as _,
            ble_ll_tx_pwr_dbm: crate::sys::BLE_LL_TX_PWR_DBM_N as _,
            rtc_freq: crate::sys::RTC_FREQ_N as _,
            ble_ll_sca: crate::sys::CONFIG_BT_LE_LL_SCA as _,
            ble_ll_scan_phy_number: crate::sys::BLE_LL_SCAN_PHY_NUMBER_N as _,
            ble_ll_conn_def_auth_pyld_tmo: crate::sys::BLE_LL_CONN_DEF_AUTH_PYLD_TMO_N as _,
            ble_ll_jitter_usecs: crate::sys::BLE_LL_JITTER_USECS_N as _,
            ble_ll_sched_max_adv_pdu_usecs: crate::sys::BLE_LL_SCHED_MAX_ADV_PDU_USECS_N as _,
            ble_ll_sched_direct_adv_max_usecs: crate::sys::BLE_LL_SCHED_DIRECT_ADV_MAX_USECS_N as _,
            ble_ll_sched_adv_max_usecs: crate::sys::BLE_LL_SCHED_ADV_MAX_USECS_N as _,
            ble_scan_rsp_data_max_len: crate::sys::DEFAULT_BT_LE_SCAN_RSP_DATA_MAX_LEN_N as _,
            ble_ll_cfg_num_hci_cmd_pkts: crate::sys::BLE_LL_CFG_NUM_HCI_CMD_PKTS_N as _,
            ble_ll_ctrl_proc_timeout_ms: crate::sys::BLE_LL_CTRL_PROC_TIMEOUT_MS_N,
            nimble_max_connections: crate::sys::DEFAULT_BT_LE_MAX_CONNECTIONS as _,
            ble_whitelist_size: crate::sys::DEFAULT_BT_NIMBLE_WHITELIST_SIZE as _,
            ble_acl_buf_size: crate::sys::DEFAULT_BT_LE_ACL_BUF_SIZE as _,
            ble_acl_buf_count: crate::sys::DEFAULT_BT_LE_ACL_BUF_COUNT as _,
            ble_hci_evt_buf_size: crate::sys::DEFAULT_BT_LE_HCI_EVT_BUF_SIZE as _,
            ble_multi_adv_instances: crate::sys::DEFAULT_BT_LE_MAX_EXT_ADV_INSTANCES as _,
            ble_ext_adv_max_size: crate::sys::DEFAULT_BT_LE_EXT_ADV_MAX_SIZE as _,
            controller_task_stack_size: crate::sys::NIMBLE_LL_STACK_SIZE as _,
            controller_task_prio: crate::sys::ESP_TASK_BT_CONTROLLER_PRIO as _,
            controller_run_cpu: 0,
            enable_qa_test: crate::sys::RUN_QA_TEST as _,
            enable_bqb_test: crate::sys::RUN_BQB_TEST as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            enable_uart_hci: crate::sys::HCI_UART_EN as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            ble_hci_uart_port: crate::sys::DEFAULT_BT_LE_HCI_UART_PORT as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            ble_hci_uart_baud: crate::sys::DEFAULT_BT_LE_HCI_UART_BAUD,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            ble_hci_uart_data_bits: crate::sys::DEFAULT_BT_LE_HCI_UART_DATA_BITS as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            ble_hci_uart_stop_bits: crate::sys::DEFAULT_BT_LE_HCI_UART_STOP_BITS as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            ble_hci_uart_flow_ctrl: crate::sys::DEFAULT_BT_LE_HCI_UART_FLOW_CTRL as _,
            #[cfg(any(
                esp_idf_version_major = "4",
                esp_idf_version = "5.0",
                all(esp_idf_version = "5.1", not(esp_idf_version_patch_at_least_5_1_5)),
                all(esp_idf_version = "5.2", not(esp_idf_version_patch_at_least_5_2_3)),
                all(esp_idf_version = "5.3", not(esp_idf_version_patch_at_least_5_3_1)),
                not(any(esp32c5, esp32c6, esp32c61, esp32h2))
            ))]
            ble_hci_uart_uart_parity: crate::sys::DEFAULT_BT_LE_HCI_UART_PARITY as _,
            enable_tx_cca: crate::sys::DEFAULT_BT_LE_TX_CCA_ENABLED as _,
            cca_rssi_thresh: (256 - crate::sys::DEFAULT_BT_LE_CCA_RSSI_THRESH) as _,
            sleep_en: crate::sys::NIMBLE_SLEEP_ENABLE as _,
            coex_phy_coded_tx_rx_time_limit: crate::sys::DEFAULT_BT_LE_COEX_PHY_CODED_TX_RX_TLIM_EFF
                as _,
            dis_scan_backoff: crate::sys::NIMBLE_DISABLE_SCAN_BACKOFF as _,
            // `esp32c2` defaults this to 0; every other chip in this block defaults to 1
            #[cfg(esp32c2)]
            ble_scan_classify_filter_enable: 0,
            #[cfg(not(esp32c2))]
            ble_scan_classify_filter_enable: 1,
            main_xtal_freq: crate::sys::CONFIG_XTAL_FREQ as _,
            #[cfg(esp32c2)]
            version_num: unsafe { crate::sys::esp_ble_get_chip_rev_version() },
            #[cfg(esp32c6)]
            #[allow(clippy::unnecessary_cast)]
            version_num: unsafe { crate::sys::efuse_hal_chip_revision() as _ },
            #[cfg(not(esp32c2))]
            cpu_freq_mhz: crate::sys::CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ as _,
            ignore_wl_for_direct_adv: 0,
            #[cfg(not(esp32c2))]
            enable_pcl: crate::sys::DEFAULT_BT_LE_POWER_CONTROL_ENABLED as _,
            #[cfg(all(
                not(esp_idf_version_major = "4"),
                not(esp_idf_version = "5.0"),
                not(esp_idf_version = "5.1")
            ))]
            csa2_select: crate::sys::DEFAULT_BT_LE_50_FEATURE_SUPPORT as _,
            // The fields below were added to the modern NimBLE controller config (shared by
            // c2/c5/c6/h2) across IDF 5.3.4 / 5.4.2 / 5.5.0. They are absent from `Default`
            // (which zeroes them), and several have non-zero upstream defaults - notably
            // `vhci_enabled`, without which Bluedroid's HCI host layer fails to start. Field
            // availability differs per chip/version, hence the gating below:
            //  - c6/h2 got Group A/B at 5.3.4; c5 got them one minor later, at 5.4.2.
            //  - c2 (minimal controller) has Group A but none of the Group B fields.
            //
            // Group A - c2, c6, h2 (>= 5.3.4) and c5 (>= 5.4.2)
            #[cfg(all(
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            vhci_enabled: crate::sys::DEFAULT_BT_LE_VHCI_ENABLED as _,
            #[cfg(all(
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            ble_aa_check: crate::sys::DEFAULT_BT_LE_CTRL_CHECK_CONNECT_IND_ACCESS_ADDRESS as _,
            #[cfg(all(
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            ble_llcp_disc_flag: crate::sys::BT_LE_CTRL_LLCP_DISC_FLAG as _,
            #[cfg(all(
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            scan_backoff_upperlimitmax: crate::sys::BT_CTRL_SCAN_BACKOFF_UPPERLIMITMAX as _,
            // Group B - as Group A, but not present on esp32c2
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            ble_chan_ass_en: crate::sys::DEFAULT_BT_LE_CTRL_CHAN_ASS_EN as _,
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            ble_data_lenth_zero_aux: crate::sys::DEFAULT_BT_LE_CTRL_ADV_DATA_LENGTH_ZERO_AUX as _,
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            ptr_check_enabled: crate::sys::DEFAULT_BT_LE_PTR_CHECK_ENABLED as _,
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            fast_conn_data_tx_en: crate::sys::DEFAULT_BT_LE_CTRL_FAST_CONN_DATA_TX_EN as _,
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            ch39_txpwr: crate::sys::BLE_LL_TX_PWR_DBM_N as _,
            // `adv_rsv_cnt` / `conn_rsv_cnt` mirror the controller's `MIN(...)` macros, which
            // bindgen cannot capture as constants, so they are reproduced from their operands.
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            #[allow(clippy::unnecessary_cast)]
            adv_rsv_cnt: core::cmp::min(
                crate::sys::DEFAULT_BT_LE_MAX_EXT_ADV_INSTANCES as u32,
                crate::sys::CONFIG_BT_LE_EXT_ADV_RESERVED_MEMORY_COUNT as u32,
            ) as _,
            #[cfg(all(
                not(esp32c2),
                any(
                    esp_idf_version_patch_at_least_5_3_4,
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
                any(
                    not(esp32c5),
                    esp_idf_version_patch_at_least_5_4_2,
                    esp_idf_version_at_least_5_5_0,
                ),
            ))]
            #[allow(clippy::unnecessary_cast)]
            conn_rsv_cnt: core::cmp::min(
                crate::sys::DEFAULT_BT_LE_MAX_CONNECTIONS as u32,
                crate::sys::CONFIG_BT_LE_CONN_RESERVED_MEMORY_COUNT as u32,
            ) as _,
            // Group C - `priority_level_cfg`, added in 5.5.0 (still present on 6.0+), not on c2
            #[cfg(all(not(esp32c2), esp_idf_version_at_least_5_5_0))]
            priority_level_cfg: crate::sys::BT_LL_CTRL_PRIO_LVL_CFG as _,
            // Group D - added in 5.5.0 and removed again in 6.0, not on c2
            #[cfg(all(
                not(esp32c2),
                esp_idf_version_at_least_5_5_0,
                not(esp_idf_version_at_least_6_0_0),
            ))]
            slv_fst_rx_lat_en: crate::sys::DEFAULT_BT_LE_CTRL_SLV_FAST_RX_CONN_DATA_EN as _,
            #[cfg(all(
                not(esp32c2),
                esp_idf_version_at_least_5_5_0,
                not(esp_idf_version_at_least_6_0_0),
            ))]
            dl_itvl_phy_sync_en: crate::sys::DEFAULT_BT_LE_CTRL_DL_ITVL_PHY_SYNC_EN as _,
            #[cfg(all(
                not(esp32c2),
                esp_idf_version_at_least_5_5_0,
                not(esp_idf_version_at_least_6_0_0),
            ))]
            scan_allow_adi_filter: crate::sys::DEFAULT_BT_SCAN_ALLOW_ENH_ADI_FILTER as _,
            config_magic: CONFIG_MAGIC as _,
            ..Default::default()
        };

        info!("Init bluetooth controller");
        esp!(unsafe { esp_bt_controller_init(&mut bt_cfg) })?;

        info!("Enable bluetooth controller");
        esp!(unsafe { esp_bt_controller_enable(M::mode()) })?;

        info!("Init bluedroid");
        esp!(unsafe { esp_bluedroid_init() })?;

        info!("Enable bluedroid");
        esp!(unsafe { esp_bluedroid_enable() })?;

        Ok(())
    }

    #[deprecated(
        since = "0.52.0",
        note = "use `EspGap::set_device_name` or `EspBleGap::set_device_name` instead"
    )]
    #[cfg(not(esp_idf_version_at_least_6_0_0))]
    pub fn set_device_name(&self, device_name: &str) -> Result<(), EspError> {
        use crate::private::cstr::to_cstring_arg;

        let device_name = to_cstring_arg(device_name)?;

        esp!(unsafe { esp_bt_dev_set_device_name(device_name.as_ptr()) })
    }
}

impl<M> Drop for BtDriver<'_, M>
where
    M: BtMode,
{
    fn drop(&mut self) {
        let _ = esp!(unsafe { esp_bluedroid_disable() });

        esp!(unsafe { esp_bluedroid_deinit() }).unwrap();

        esp!(unsafe { esp_bt_controller_disable() }).unwrap();

        esp!(unsafe { esp_bt_controller_deinit() }).unwrap();
    }
}

unsafe impl<M> Send for BtDriver<'_, M> where M: BtMode {}
unsafe impl<M> Sync for BtDriver<'_, M> where M: BtMode {}
