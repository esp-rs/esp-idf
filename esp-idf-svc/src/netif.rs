//! Network abstraction
//!
//! The purpose of ESP-NETIF library is twofold:
//!
//! - It provides an abstraction layer for the application on top of the TCP/IP
//!   stack. This will allow applications to choose between IP stacks in the
//!   future.
//! - The APIs it provides are thread safe, even if the underlying TCP/IP
//!   stack APIs are not.

use core::num::NonZeroU32;
use core::{ffi, fmt, ptr};

use crate::ipv4;
use crate::sys::*;

use ::log::info;

use crate::eventloop::{EspEventDeserializer, EspEventSource};
use crate::handle::RawHandle;
use crate::private::common::*;
use crate::private::cstr::*;
use crate::private::mutex;

#[cfg(feature = "alloc")]
pub use driver::*;
#[cfg(esp_idf_lwip_ppp_support)]
pub use ppp::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(Hash))]
pub enum NetifStack {
    /// Station mode (WiFi client)
    Sta,
    #[cfg(esp_idf_esp_wifi_softap_support)]
    /// Access point mode (WiFi router)
    Ap,
    /// Ethernet
    Eth,
    #[cfg(esp_idf_lwip_ppp_support)]
    /// Point-to-Point Protocol (PPP)
    Ppp,
    #[cfg(esp_idf_lwip_slip_support)]
    /// Serial Line Internet Protocol (SLIP)
    Slip,
    #[cfg(all(esp_idf_comp_openthread_enabled, esp_idf_openthread_enabled,))]
    Thread,
}

impl NetifStack {
    /// Initialize the ESP Netif stack
    ///
    /// This function is called automatically when a new `EspNetif` instance is created,
    /// but it can also be called manually by the user - for example, in cases when some
    /// networking code needs to be started _before_ there is even one active `EspNetif`
    /// interface.
    ///
    /// The function needs to be called only once for the duration of the program - ideally
    /// early on during the app booststraping process. Once initialized, the netif stack cannot
    /// be de-initialized.
    pub fn initialize() -> Result<(), EspError> {
        initialize_netif_stack()
    }

    pub fn default_configuration(&self) -> NetifConfiguration {
        match self {
            Self::Sta => NetifConfiguration::wifi_default_client(),
            #[cfg(esp_idf_esp_wifi_softap_support)]
            Self::Ap => NetifConfiguration::wifi_default_router(),
            Self::Eth => NetifConfiguration::eth_default_client(),
            #[cfg(esp_idf_lwip_ppp_support)]
            Self::Ppp => NetifConfiguration::ppp_default_client(),
            #[cfg(esp_idf_lwip_slip_support)]
            Self::Slip => NetifConfiguration::slip_default_client(),
            #[cfg(all(esp_idf_comp_openthread_enabled, esp_idf_openthread_enabled,))]
            Self::Thread => NetifConfiguration::thread_default(),
        }
    }

    fn default_mac(&self) -> Result<Option<[u8; 6]>, EspError> {
        if let Some(mac_type) = self.default_mac_raw_type() {
            let mut mac = [0; 6];

            let result = esp!(unsafe { esp_read_mac(mac.as_mut_ptr() as *mut _, mac_type) });

            // On SoCs without native WiFi hardware (e.g., ESP32-P4 using
            // esp_wifi_remote), the eFuse has no WiFi MAC address.
            // esp_read_mac will fail for WiFi MAC types. Return None and let
            // esp_wifi_start() set the real MAC from the companion chip later.
            #[cfg(not(esp_idf_soc_wifi_supported))]
            {
                if result.is_ok() {
                    Ok(Some(mac))
                } else {
                    Ok(None)
                }
            }

            #[cfg(esp_idf_soc_wifi_supported)]
            {
                result?;

                Ok(Some(mac))
            }
        } else {
            Ok(None)
        }
    }

    fn default_mac_raw_type(&self) -> Option<esp_mac_type_t> {
        match self {
            Self::Sta => Some(esp_mac_type_t_ESP_MAC_WIFI_STA),
            #[cfg(esp_idf_esp_wifi_softap_support)]
            Self::Ap => Some(esp_mac_type_t_ESP_MAC_WIFI_SOFTAP),
            Self::Eth => Some(esp_mac_type_t_ESP_MAC_ETH),
            #[cfg(all(esp_idf_comp_openthread_enabled, esp_idf_openthread_enabled,))]
            Self::Thread => {
                #[cfg(esp_idf_soc_ieee802154_supported)]
                let mac_type = Some(esp_mac_type_t_ESP_MAC_IEEE802154);

                #[cfg(not(esp_idf_soc_ieee802154_supported))]
                let mac_type = None;

                mac_type
            }
            #[cfg(any(esp_idf_lwip_slip_support, esp_idf_lwip_ppp_support))]
            _ => None,
        }
    }

    fn default_raw_stack(&self) -> *const esp_netif_netstack_config_t {
        unsafe {
            match self {
                Self::Sta => _g_esp_netif_netstack_default_wifi_sta,
                #[cfg(esp_idf_esp_wifi_softap_support)]
                Self::Ap => _g_esp_netif_netstack_default_wifi_ap,
                Self::Eth => _g_esp_netif_netstack_default_eth,
                #[cfg(esp_idf_lwip_ppp_support)]
                Self::Ppp => _g_esp_netif_netstack_default_ppp,
                #[cfg(esp_idf_lwip_slip_support)]
                Self::Slip => _g_esp_netif_netstack_default_slip,
                #[cfg(all(
                    esp_idf_comp_openthread_enabled,
                    esp_idf_openthread_enabled,
                    esp_idf_version_major = "4"
                ))]
                Self::Thread => _g_esp_netif_netstack_default_openthread,
                #[cfg(all(
                    esp_idf_comp_openthread_enabled,
                    esp_idf_openthread_enabled,
                    not(esp_idf_version_major = "4")
                ))]
                Self::Thread => &g_esp_netif_netstack_default_openthread,
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetifConfiguration {
    pub flags: u32,
    pub got_ip_event_id: Option<NonZeroU32>,
    pub lost_ip_event_id: Option<NonZeroU32>,
    pub key: heapless::String<32>,
    pub description: heapless::String<8>,
    pub route_priority: u32,
    pub ip_configuration: Option<ipv4::Configuration>,
    pub stack: NetifStack,
    pub custom_mac: Option<[u8; 6]>,
}

impl NetifConfiguration {
    pub fn eth_default_client() -> Self {
        Self {
            flags: esp_netif_flags_ESP_NETIF_FLAG_GARP
                | esp_netif_flags_ESP_NETIF_FLAG_EVENT_IP_MODIFIED,
            got_ip_event_id: NonZeroU32::new(ip_event_t_IP_EVENT_ETH_GOT_IP as _),
            lost_ip_event_id: NonZeroU32::new(ip_event_t_IP_EVENT_ETH_LOST_IP as _),
            key: "ETH_DEF".try_into().unwrap(),
            description: "eth".try_into().unwrap(),
            route_priority: 60,
            ip_configuration: Some(ipv4::Configuration::Client(Default::default())),
            stack: NetifStack::Eth,
            custom_mac: None,
        }
    }

    pub fn eth_default_router() -> Self {
        Self {
            flags: 0,
            got_ip_event_id: None,
            lost_ip_event_id: None,
            key: "ETH_RT_DEF".try_into().unwrap(),
            description: "ethrt".try_into().unwrap(),
            route_priority: 50,
            ip_configuration: Some(ipv4::Configuration::Router(Default::default())),
            stack: NetifStack::Eth,
            custom_mac: None,
        }
    }

    pub fn wifi_default_client() -> Self {
        Self {
            flags: esp_netif_flags_ESP_NETIF_FLAG_GARP
                | esp_netif_flags_ESP_NETIF_FLAG_EVENT_IP_MODIFIED,
            got_ip_event_id: NonZeroU32::new(ip_event_t_IP_EVENT_STA_GOT_IP as _),
            lost_ip_event_id: NonZeroU32::new(ip_event_t_IP_EVENT_STA_LOST_IP as _),
            key: "WIFI_STA_DEF".try_into().unwrap(),
            description: "sta".try_into().unwrap(),
            route_priority: 100,
            ip_configuration: Some(ipv4::Configuration::Client(Default::default())),
            stack: NetifStack::Sta,
            custom_mac: None,
        }
    }

    #[cfg(esp_idf_esp_wifi_softap_support)]
    pub fn wifi_default_router() -> Self {
        Self {
            flags: 0,
            got_ip_event_id: None,
            lost_ip_event_id: None,
            key: "WIFI_AP_DEF".try_into().unwrap(),
            description: "ap".try_into().unwrap(),
            route_priority: 10,
            ip_configuration: Some(ipv4::Configuration::Router(Default::default())),
            stack: NetifStack::Ap,
            custom_mac: None,
        }
    }

    #[cfg(esp_idf_lwip_ppp_support)]
    pub fn ppp_default_client() -> Self {
        Self {
            flags: esp_netif_flags_ESP_NETIF_FLAG_IS_PPP,
            got_ip_event_id: NonZeroU32::new(ip_event_t_IP_EVENT_PPP_GOT_IP as _),
            lost_ip_event_id: NonZeroU32::new(ip_event_t_IP_EVENT_PPP_LOST_IP as _),
            key: "PPP_CL_DEF".try_into().unwrap(),
            description: "ppp".try_into().unwrap(),
            route_priority: 30,
            // IPv4 addressing and DNS are negotiated by PPP/IPCP. PPP interfaces
            // must not carry the DHCP-client flag; ESP-IDF explicitly rejects DHCP
            // operations for point-to-point interfaces.
            ip_configuration: None,
            stack: NetifStack::Ppp,
            custom_mac: None,
        }
    }

    #[cfg(esp_idf_lwip_ppp_support)]
    pub fn ppp_default_router() -> Self {
        Self {
            flags: esp_netif_flags_ESP_NETIF_FLAG_IS_PPP,
            got_ip_event_id: None,
            lost_ip_event_id: None,
            key: "PPP_RT_DEF".try_into().unwrap(),
            description: "ppprt".try_into().unwrap(),
            route_priority: 20,
            ip_configuration: Some(ipv4::Configuration::Router(Default::default())),
            stack: NetifStack::Ppp,
            custom_mac: None,
        }
    }

    #[cfg(esp_idf_lwip_slip_support)]
    pub fn slip_default_client() -> Self {
        Self {
            flags: 0,
            get_ip_event: None,
            lost_ip_event: None,
            key: "SLIP_CL_DEF".try_into().unwrap(),
            description: "slip".try_into().unwrap(),
            route_priority: 35,
            ip_configuration: Some(ipv4::Configuration::Client(Default::default())),
            stack: NetifStack::Slip,
            custom_mac: None,
        }
    }

    #[cfg(esp_idf_lwip_slip_support)]
    pub fn slip_default_router() -> Self {
        Self {
            flags: 0,
            get_ip_event: None,
            lost_ip_event: None,
            key: "SLIP_RT_DEF".try_into().unwrap(),
            description: "sliprt".try_into().unwrap(),
            route_priority: 25,
            ip_configuration: Some(ipv4::Configuration::Router(Default::default())),
            stack: NetifStack::Slip,
            custom_mac: None,
        }
    }

    #[cfg(all(esp_idf_comp_openthread_enabled, esp_idf_openthread_enabled,))]
    pub fn thread_default() -> Self {
        Self {
            flags: 0,
            got_ip_event_id: None,
            lost_ip_event_id: None,
            key: "OT_DEF".try_into().unwrap(),
            description: "thread".try_into().unwrap(),
            route_priority: 15,
            ip_configuration: None,
            stack: NetifStack::Thread,
            custom_mac: None,
        }
    }
}

static INITALIZED: mutex::Mutex<bool> = mutex::Mutex::new(false);

fn initialize_netif_stack() -> Result<(), EspError> {
    let mut guard = INITALIZED.lock();

    if !*guard {
        esp!(unsafe { esp_netif_init() })?;

        *guard = true;
    }

    Ok(())
}

#[derive(Debug)]
pub struct EspNetif {
    handle: *mut esp_netif_t,
    _got_ip_event_id: Option<NonZeroU32>,
    _lost_ip_event_id: Option<NonZeroU32>,
}

impl EspNetif {
    pub fn new(stack: NetifStack) -> Result<Self, EspError> {
        Self::new_with_conf(&stack.default_configuration())
    }

    #[allow(unused_mut)]
    pub fn new_with_conf(conf: &NetifConfiguration) -> Result<Self, EspError> {
        initialize_netif_stack()?;

        let c_if_key = to_cstring_arg(conf.key.as_str())?;
        let c_if_description = to_cstring_arg(conf.description.as_str())?;

        let initial_mac = if let Some(custom_mac) = conf.custom_mac {
            custom_mac
        } else {
            conf.stack.default_mac()?.unwrap_or([0; 6])
        };

        let (mut esp_inherent_config, ip_info, dhcps, dns, secondary_dns, hostname) = match conf
            .ip_configuration
        {
            Some(ipv4::Configuration::Client(ref ip_conf)) => (
                esp_netif_inherent_config_t {
                    flags: conf.flags
                        | (if matches!(ip_conf, ipv4::ClientConfiguration::DHCP(_)) {
                            esp_netif_flags_ESP_NETIF_DHCP_CLIENT
                        } else {
                            0
                        }),
                    mac: initial_mac,
                    ip_info: ptr::null(),
                    get_ip_event: conf.got_ip_event_id.map(NonZeroU32::get).unwrap_or(0),
                    lost_ip_event: conf.lost_ip_event_id.map(NonZeroU32::get).unwrap_or(0),
                    if_key: c_if_key.as_c_str().as_ptr() as _,
                    if_desc: c_if_description.as_c_str().as_ptr() as _,
                    route_prio: conf.route_priority as _,
                    #[cfg(not(esp_idf_version_major = "4"))]
                    bridge_info: ptr::null_mut(),
                    #[cfg(esp_idf_version_at_least_6_0_0)]
                    mtu: 0,
                },
                match ip_conf {
                    ipv4::ClientConfiguration::DHCP(_) => None,
                    ipv4::ClientConfiguration::Fixed(ref fixed_conf) => Some(esp_netif_ip_info_t {
                        ip: Newtype::<esp_ip4_addr_t>::from(fixed_conf.ip).0,
                        netmask: Newtype::<esp_ip4_addr_t>::from(fixed_conf.subnet.mask).0,
                        gw: Newtype::<esp_ip4_addr_t>::from(fixed_conf.subnet.gateway).0,
                    }),
                },
                false,
                match ip_conf {
                    ipv4::ClientConfiguration::DHCP(_) => None,
                    ipv4::ClientConfiguration::Fixed(ref fixed_conf) => fixed_conf.dns,
                },
                match ip_conf {
                    ipv4::ClientConfiguration::DHCP(_) => None,
                    ipv4::ClientConfiguration::Fixed(ref fixed_conf) => fixed_conf.secondary_dns,
                },
                match ip_conf {
                    ipv4::ClientConfiguration::DHCP(ref dhcp_conf) => dhcp_conf.hostname.as_ref(),
                    ipv4::ClientConfiguration::Fixed(_) => None,
                },
            ),
            Some(ipv4::Configuration::Router(ref ip_conf)) => (
                esp_netif_inherent_config_t {
                    flags: conf.flags
                        | (if ip_conf.dhcp_enabled {
                            esp_netif_flags_ESP_NETIF_DHCP_SERVER
                        } else {
                            0
                        })
                        | esp_netif_flags_ESP_NETIF_FLAG_AUTOUP,
                    mac: initial_mac,
                    ip_info: ptr::null(),
                    get_ip_event: conf.got_ip_event_id.map(NonZeroU32::get).unwrap_or(0),
                    lost_ip_event: conf.lost_ip_event_id.map(NonZeroU32::get).unwrap_or(0),
                    if_key: c_if_key.as_c_str().as_ptr() as _,
                    if_desc: c_if_description.as_c_str().as_ptr() as _,
                    route_prio: conf.route_priority as _,
                    #[cfg(not(esp_idf_version_major = "4"))]
                    bridge_info: ptr::null_mut(),
                    #[cfg(esp_idf_version_at_least_6_0_0)]
                    mtu: 0,
                },
                Some(esp_netif_ip_info_t {
                    ip: Newtype::<esp_ip4_addr_t>::from(ip_conf.subnet.gateway).0,
                    netmask: Newtype::<esp_ip4_addr_t>::from(ip_conf.subnet.mask).0,
                    gw: Newtype::<esp_ip4_addr_t>::from(ip_conf.subnet.gateway).0,
                }),
                ip_conf.dhcp_enabled,
                ip_conf.dns,
                None, /* For APs, ESP-IDF supports setting a primary DNS only ip_conf.secondary_dns */
                None,
            ),
            None => {
                let mut flags = conf.flags | esp_netif_flags_ESP_NETIF_FLAG_AUTOUP;

                #[cfg(esp_idf_lwip_ppp_support)]
                {
                    if matches!(conf.stack, NetifStack::Ppp) {
                        flags &= !esp_netif_flags_ESP_NETIF_FLAG_AUTOUP;
                    }
                }

                (
                    esp_netif_inherent_config_t {
                        flags,
                        mac: initial_mac,
                        ip_info: ptr::null(),
                        get_ip_event: conf.got_ip_event_id.map(NonZeroU32::get).unwrap_or(0),
                        lost_ip_event: conf.lost_ip_event_id.map(NonZeroU32::get).unwrap_or(0),
                        if_key: c_if_key.as_c_str().as_ptr() as _,
                        if_desc: c_if_description.as_c_str().as_ptr() as _,
                        route_prio: conf.route_priority as _,
                        #[cfg(not(esp_idf_version_major = "4"))]
                        bridge_info: ptr::null_mut(),
                        #[cfg(esp_idf_version_at_least_6_0_0)]
                        mtu: 0,
                    },
                    None,
                    false,
                    None,
                    None,
                    None,
                )
            }
        };

        if let Some(ip_info) = ip_info.as_ref() {
            esp_inherent_config.ip_info = ip_info;
        }

        let cfg = esp_netif_config_t {
            base: &esp_inherent_config,
            driver: ptr::null(),
            stack: conf.stack.default_raw_stack(),
        };

        let mut netif = Self {
            handle: unsafe { esp_netif_new(&cfg).as_mut() }
                .ok_or(EspError::from_infallible::<ESP_ERR_INVALID_ARG>())?,
            _got_ip_event_id: conf.got_ip_event_id,
            _lost_ip_event_id: conf.lost_ip_event_id,
        };

        if let Some(dns) = dns {
            netif.set_dns(dns);

            if dhcps {
                #[cfg(esp_idf_version_major = "4")]
                let mut dhcps_dns_value: dhcps_offer_t = dhcps_offer_option_OFFER_DNS as _;

                // Strangely dhcps_offer_t and dhcps_offer_option_* are not included in ESP-IDF V5's bindings
                #[cfg(not(esp_idf_version_major = "4"))]
                let mut dhcps_dns_value: u8 = 2_u8;

                esp!(unsafe {
                    esp_netif_dhcps_option(
                        netif.handle,
                        esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET,
                        esp_netif_dhcp_option_id_t_ESP_NETIF_DOMAIN_NAME_SERVER,
                        &mut dhcps_dns_value as *mut _ as *mut _,
                        core::mem::size_of_val(&dhcps_dns_value) as u32,
                    )
                })?;
            }
        }

        if let Some(secondary_dns) = secondary_dns {
            netif.set_secondary_dns(secondary_dns);
        }

        if let Some(hostname) = hostname {
            netif.set_hostname(hostname)?;
        }

        Ok(netif)
    }

    pub fn is_netif_up(&self) -> Result<bool, EspError> {
        Ok(unsafe { esp_netif_is_netif_up(self.handle) })
    }

    // TODO: Copy and rename to `is_up_ipv4` and deprecate the `is_up` variant in future
    pub fn is_up(&self) -> Result<bool, EspError> {
        if !self.is_netif_up()? {
            Ok(false)
        } else {
            let mut ip_info = Default::default();
            unsafe { esp!(esp_netif_get_ip_info(self.handle, &mut ip_info)) }?;

            Ok(ipv4::IpInfo::from(Newtype(ip_info)).ip != ipv4::Ipv4Addr::new(0, 0, 0, 0))
        }
    }

    pub fn get_ip_info(&self) -> Result<ipv4::IpInfo, EspError> {
        let mut ip_info = Default::default();
        unsafe { esp!(esp_netif_get_ip_info(self.handle, &mut ip_info)) }?;

        Ok(ipv4::IpInfo {
            // Get the DNS information
            dns: Some(self.get_dns()),
            secondary_dns: Some(self.get_secondary_dns()),
            ..Newtype(ip_info).into()
        })
    }

    pub fn get_key(&self) -> heapless::String<32> {
        unsafe { from_cstr_ptr(esp_netif_get_ifkey(self.handle)) }
            .try_into()
            .unwrap()
    }

    pub fn get_index(&self) -> u32 {
        unsafe { esp_netif_get_netif_impl_index(self.handle) as _ }
    }

    pub fn get_name(&self) -> heapless::String<6> {
        let mut netif_name = [0u8; 7];

        esp!(unsafe {
            esp_netif_get_netif_impl_name(self.handle, netif_name.as_mut_ptr() as *mut _)
        })
        .unwrap();

        from_cstr(&netif_name).try_into().unwrap()
    }

    pub fn get_mac(&self) -> Result<[u8; 6], EspError> {
        let mut mac = [0u8; 6];

        esp!(unsafe { esp_netif_get_mac(self.handle, mac.as_mut_ptr() as *mut _) })?;
        Ok(mac)
    }

    pub fn set_mac(&mut self, mac: &[u8; 6]) -> Result<(), EspError> {
        esp!(unsafe { esp_netif_set_mac(self.handle, mac.as_ptr() as *mut _) })?;
        Ok(())
    }

    pub fn get_dns(&self) -> ipv4::Ipv4Addr {
        let mut dns_info = Default::default();

        unsafe {
            esp!(esp_netif_get_dns_info(
                self.handle,
                esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
                &mut dns_info
            ))
            .unwrap();

            Newtype(dns_info.ip.u_addr.ip4).into()
        }
    }

    fn set_dns(&mut self, dns: ipv4::Ipv4Addr) {
        let mut dns_info: esp_netif_dns_info_t = Default::default();

        unsafe {
            dns_info.ip.u_addr.ip4 = Newtype::<esp_ip4_addr_t>::from(dns).0;

            esp!(esp_netif_set_dns_info(
                self.handle,
                esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
                &mut dns_info
            ))
            .unwrap();
        }
    }

    pub fn get_secondary_dns(&self) -> ipv4::Ipv4Addr {
        let mut dns_info = Default::default();

        unsafe {
            esp!(esp_netif_get_dns_info(
                self.handle,
                esp_netif_dns_type_t_ESP_NETIF_DNS_BACKUP,
                &mut dns_info
            ))
            .unwrap();

            Newtype(dns_info.ip.u_addr.ip4).into()
        }
    }

    fn set_secondary_dns(&mut self, secondary_dns: ipv4::Ipv4Addr) {
        let mut dns_info: esp_netif_dns_info_t = Default::default();

        unsafe {
            dns_info.ip.u_addr.ip4 = Newtype::<esp_ip4_addr_t>::from(secondary_dns).0;

            esp!(esp_netif_set_dns_info(
                self.handle,
                esp_netif_dns_type_t_ESP_NETIF_DNS_BACKUP,
                &mut dns_info
            ))
            .unwrap();
        }
    }

    pub fn get_hostname(&self) -> Result<heapless::String<30>, EspError> {
        let mut ptr: *const ffi::c_char = ptr::null();
        esp!(unsafe { esp_netif_get_hostname(self.handle, &mut ptr) })?;

        Ok(unsafe { from_cstr_ptr(ptr) }.try_into().unwrap())
    }

    fn set_hostname(&mut self, hostname: &str) -> Result<(), EspError> {
        let hostname = to_cstring_arg(hostname)?;

        esp!(unsafe { esp_netif_set_hostname(self.handle, hostname.as_ptr() as *const _) })?;

        Ok(())
    }

    #[cfg(esp_idf_lwip_ipv4_napt)]
    pub fn enable_napt(&mut self, enable: bool) {
        unsafe {
            crate::sys::ip_napt_enable_no(
                (esp_netif_get_netif_impl_index(self.handle) - 1) as u8,
                if enable { 1 } else { 0 },
            )
        };
    }
}

impl Drop for EspNetif {
    fn drop(&mut self) {
        unsafe { esp_netif_destroy(self.handle) };

        info!("Dropped");
    }
}

unsafe impl Send for EspNetif {}

impl RawHandle for EspNetif {
    type Handle = *mut esp_netif_t;

    fn handle(&self) -> Self::Handle {
        self.handle
    }
}

#[derive(Copy, Clone)]
pub struct ApStaIpAssignment<'a>(&'a ip_event_ap_staipassigned_t);

impl ApStaIpAssignment<'_> {
    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn netif_handle(&self) -> *mut esp_netif_t {
        self.0.esp_netif
    }

    pub fn ip(&self) -> ipv4::Ipv4Addr {
        ipv4::Ipv4Addr::from(Newtype(self.0.ip))
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    pub fn mac(&self) -> [u8; 6] {
        self.0.mac
    }
}

impl fmt::Debug for ApStaIpAssignment<'_> {
    #[cfg(esp_idf_version_major = "4")]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApStaIpAssignment")
            .field("ip", &self.ip())
            .finish()
    }

    #[cfg(not(esp_idf_version_major = "4"))]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApStaIpAssignment")
            .field("ip", &self.ip())
            .field("mac", &self.mac())
            .finish()
    }
}

#[derive(Copy, Clone)]
pub struct DhcpIpAssignment<'a>(&'a ip_event_got_ip_t);

impl DhcpIpAssignment<'_> {
    pub fn netif_handle(&self) -> *mut esp_netif_t {
        self.0.esp_netif
    }

    pub fn ip(&self) -> ipv4::Ipv4Addr {
        ipv4::Ipv4Addr::from(Newtype(self.0.ip_info.ip))
    }

    pub fn gateway(&self) -> ipv4::Ipv4Addr {
        ipv4::Ipv4Addr::from(Newtype(self.0.ip_info.gw))
    }

    pub fn mask(&self) -> ipv4::Mask {
        Newtype(self.0.ip_info.netmask).try_into().unwrap()
    }

    pub fn ip_info(&self) -> ipv4::IpInfo {
        ipv4::IpInfo {
            ip: self.ip(),
            subnet: ipv4::Subnet {
                gateway: self.gateway(),
                mask: self.mask(),
            },
            dns: None,           // TODO
            secondary_dns: None, // TODO
        }
    }

    pub fn is_ip_changed(&self) -> bool {
        self.0.ip_changed
    }
}

impl fmt::Debug for DhcpIpAssignment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DhcpIpAssignment")
            .field("netif_handle", &self.netif_handle())
            .field("ip", &self.ip())
            .field("gateway", &self.gateway())
            .field("mask", &self.mask())
            .field("is_ip_changed", &self.is_ip_changed())
            .finish()
    }
}

#[derive(Copy, Clone)]
pub struct DhcpIp6Assignment<'a>(&'a ip_event_got_ip6_t);

impl DhcpIp6Assignment<'_> {
    pub fn netif_handle(&self) -> *mut esp_netif_t {
        self.0.esp_netif
    }

    pub fn addr(&self) -> core::net::Ipv6Addr {
        Newtype(self.0.ip6_info.ip).into()
    }

    pub fn ip(&self) -> [u32; 4] {
        self.0.ip6_info.ip.addr
    }

    pub fn ip_zone(&self) -> u8 {
        self.0.ip6_info.ip.zone
    }

    pub fn ip_index(&self) -> u32 {
        self.0.ip_index as _
    }
}

impl fmt::Debug for DhcpIp6Assignment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DhcpIp6Assignment")
            .field("netif_handle", &self.netif_handle())
            .field("addr", &self.addr())
            .field("ip_zone", &self.ip_zone())
            .field("ip_index", &self.ip_index())
            .finish()
    }
}

#[derive(Copy, Clone, Debug)]
pub enum IpEvent<'a> {
    ApStaIpAssigned(ApStaIpAssignment<'a>),
    DhcpIpAssigned(DhcpIpAssignment<'a>),
    DhcpIp6Assigned(DhcpIp6Assignment<'a>),
    DhcpIpDeassigned(*mut esp_netif_t),

    /// An event ID not recognised by this version of the library was received.
    ///
    /// This variant is produced instead of panicking when an unknown event ID
    /// arrives, allowing applications to remain forward-compatible with
    /// ESP-IDF versions that introduce new IP events.
    Other(i32),
}

unsafe impl Send for IpEvent<'_> {}

impl IpEvent<'_> {
    pub fn is_for(&self, raw_handle: &impl RawHandle<Handle = *mut esp_netif_t>) -> bool {
        self.is_for_handle(raw_handle.handle())
    }

    pub fn is_for_handle(&self, handle: *mut esp_netif_t) -> bool {
        self.handle()
            .map(|event_handle| core::ptr::eq(event_handle, handle))
            .unwrap_or(false)
    }

    pub fn handle(&self) -> Option<*mut esp_netif_t> {
        match self {
            #[cfg(not(esp_idf_version_major = "4"))]
            Self::ApStaIpAssigned(assignment) => Some(assignment.netif_handle()),
            #[cfg(esp_idf_version_major = "4")]
            Self::ApStaIpAssigned(_) => None,
            Self::DhcpIpAssigned(assignment) => Some(assignment.netif_handle()),
            Self::DhcpIp6Assigned(assignment) => Some(assignment.netif_handle()),
            Self::DhcpIpDeassigned(handle) => Some(*handle),
            Self::Other(_) => None,
        }
    }
}

unsafe impl EspEventSource for IpEvent<'_> {
    fn source() -> Option<&'static ffi::CStr> {
        Some(unsafe { CStr::from_ptr(IP_EVENT) })
    }
}

impl EspEventDeserializer for IpEvent<'_> {
    type Data<'d> = IpEvent<'d>;

    #[allow(non_upper_case_globals, non_snake_case)]
    fn deserialize<'d>(data: &crate::eventloop::EspEvent<'d>) -> IpEvent<'d> {
        let event_id = data.event_id as u32;

        if event_id == ip_event_t_IP_EVENT_AP_STAIPASSIGNED {
            let event = unsafe {
                (data.payload.unwrap() as *const _ as *const ip_event_ap_staipassigned_t)
                    .as_ref()
                    .unwrap()
            };

            IpEvent::ApStaIpAssigned(ApStaIpAssignment(event))
        } else if event_id == ip_event_t_IP_EVENT_STA_GOT_IP
            || event_id == ip_event_t_IP_EVENT_ETH_GOT_IP
            || event_id == ip_event_t_IP_EVENT_PPP_GOT_IP
        {
            let event = unsafe {
                (data.payload.unwrap() as *const _ as *const ip_event_got_ip_t)
                    .as_ref()
                    .unwrap()
            };

            IpEvent::DhcpIpAssigned(DhcpIpAssignment(event))
        } else if event_id == ip_event_t_IP_EVENT_GOT_IP6 {
            let event = unsafe {
                (data.payload.unwrap() as *const _ as *const ip_event_got_ip6_t)
                    .as_ref()
                    .unwrap()
            };

            IpEvent::DhcpIp6Assigned(DhcpIp6Assignment(event))
        } else if event_id == ip_event_t_IP_EVENT_PPP_LOST_IP {
            // ESP-IDF posts an `ip_event_got_ip_t` for PPP loss as well; only
            // its `esp_netif` member is populated.
            let event = unsafe {
                (data.payload.unwrap() as *const _ as *const ip_event_got_ip_t)
                    .as_ref()
                    .unwrap()
            };

            IpEvent::DhcpIpDeassigned(event.esp_netif)
        } else if event_id == ip_event_t_IP_EVENT_STA_LOST_IP
            || event_id == ip_event_t_IP_EVENT_ETH_LOST_IP
        {
            let netif_handle_mut = unsafe {
                (data.payload.unwrap() as *const _ as *mut esp_netif_t)
                    .as_mut()
                    .unwrap()
            };

            IpEvent::DhcpIpDeassigned(netif_handle_mut as *mut _)
        } else {
            ::log::warn!("IpEvent: unknown event ID {event_id}, ignoring");
            IpEvent::Other(event_id as i32)
        }
    }
}

pub trait NetifStatus {
    fn is_up(&self) -> Result<bool, EspError>;
}

impl<T> NetifStatus for &T
where
    T: NetifStatus,
{
    fn is_up(&self) -> Result<bool, EspError> {
        (**self).is_up()
    }
}

impl<T> NetifStatus for &mut T
where
    T: NetifStatus,
{
    fn is_up(&self) -> Result<bool, EspError> {
        (**self).is_up()
    }
}

impl NetifStatus for EspNetif {
    fn is_up(&self) -> Result<bool, EspError> {
        EspNetif::is_up(self)
    }
}

const UP_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(15);

pub struct BlockingNetif<T> {
    netif: T,
    event_loop: crate::eventloop::EspSystemEventLoop,
}

impl<T> BlockingNetif<T>
where
    T: NetifStatus,
{
    pub fn wrap(netif: T, event_loop: crate::eventloop::EspSystemEventLoop) -> Self {
        Self { netif, event_loop }
    }

    pub fn is_up(&self) -> Result<bool, EspError> {
        self.netif.is_up()
    }

    pub fn wait_netif_up(&self) -> Result<(), EspError> {
        self.ip_wait_while(|| self.netif.is_up().map(|s| !s), Some(UP_TIMEOUT))
    }

    pub fn ip_wait_while<F: Fn() -> Result<bool, EspError>>(
        &self,
        matcher: F,
        timeout: Option<core::time::Duration>,
    ) -> Result<(), EspError> {
        let wait = crate::eventloop::Wait::new::<IpEvent>(&self.event_loop)?;

        wait.wait_while(matcher, timeout)
    }
}

impl<T> NetifStatus for BlockingNetif<T>
where
    T: NetifStatus,
{
    fn is_up(&self) -> Result<bool, EspError> {
        BlockingNetif::is_up(self)
    }
}

#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
pub struct AsyncNetif<T> {
    netif: T,
    event_loop: crate::eventloop::EspSystemEventLoop,
    timer_service: crate::timer::EspTaskTimerService,
}

#[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
impl<T> AsyncNetif<T>
where
    T: NetifStatus,
{
    pub fn wrap(
        netif: T,
        event_loop: crate::eventloop::EspSystemEventLoop,
        timer_service: crate::timer::EspTaskTimerService,
    ) -> Self {
        Self {
            netif,
            event_loop,
            timer_service,
        }
    }

    pub fn is_up(&self) -> Result<bool, EspError> {
        self.netif.is_up()
    }

    pub async fn wait_netif_up(&mut self) -> Result<(), EspError> {
        self.ip_wait_while(|this| this.netif.is_up().map(|s| !s), Some(UP_TIMEOUT))
            .await
    }

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

#[cfg(feature = "alloc")]
mod driver {
    use core::borrow::BorrowMut;

    use ::log::debug;
    use alloc::{sync::Arc, vec::Vec};
    use embassy_futures::select::{select, Either};
    use embassy_sync::channel::Channel;
    use embedded_io_async::{Read, Write};

    use crate::hal::task::embassy_sync::EspRawMutex;
    use crate::handle::RawHandle;
    use crate::sys::*;

    use super::EspNetif;

    pub struct EspNetifDriver<'d, T>
    where
        T: BorrowMut<EspNetif>,
    {
        inner: alloc::boxed::Box<EspNetifDriverInner<'d, T>>,
        started: bool,
    }

    // The C driver retains a pointer to `inner`, whose allocation remains stable
    // when this outer value moves. Both callbacks are Send and `T` owns the netif.
    unsafe impl<T> Send for EspNetifDriver<'_, T> where T: BorrowMut<EspNetif> + Send {}

    /// The default number of outbound packets buffered by [`AsyncEspNetifChannel`].
    pub const ASYNC_NETIF_TX_QUEUE_SIZE: usize = 8;

    /// An error produced while forwarding bytes from an asynchronous transport
    /// into an [`EspNetifDriver`].
    #[derive(Debug)]
    pub enum AsyncEspNetifChannelError<E> {
        /// Reading from the transport failed.
        Read(E),
        /// Writing to the transport failed.
        Write(E),
        /// Passing the received bytes to ESP-NETIF failed.
        Netif(EspError),
    }

    impl<E> core::fmt::Display for AsyncEspNetifChannelError<E>
    where
        E: core::fmt::Display,
    {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Read(error) => write!(f, "transport read failed: {error}"),
                Self::Write(error) => write!(f, "transport write failed: {error}"),
                Self::Netif(error) => write!(f, "ESP-NETIF receive failed: {error}"),
            }
        }
    }

    #[cfg(feature = "std")]
    impl<E> std::error::Error for AsyncEspNetifChannelError<E> where
        E: core::fmt::Debug + core::fmt::Display
    {
    }

    /// Bridges an [`EspNetifDriver`] to asynchronous byte streams.
    ///
    /// This adapter is transport- and modem-agnostic. A modem driver is expected
    /// to complete its command negotiation and enter PPP data mode before its
    /// read and write halves are passed to [`Self::receive`] and
    /// [`Self::transmit`]. The two methods should be driven concurrently.
    ///
    /// ESP-NETIF invokes its transmit callback synchronously, while
    /// [`Write`] is asynchronous. Outbound packets are therefore
    /// copied into a bounded queue. If that queue is full, ESP-NETIF is notified
    /// with `ESP_ERR_NO_MEM` instead of blocking its TCP/IP task.
    ///
    /// A modem-specific driver remains responsible for entering PPP data mode. For
    /// example, after obtaining an `a76xx::PppIo` stream from `connect_ppp`:
    ///
    /// ```ignore
    /// let mut bridge = AsyncEspNetifChannel::<_, 8>::new(
    ///     EspNetif::new(NetifStack::Ppp)?,
    ///     |netif| netif.set_ppp_conf(&PppConfiguration::default()),
    /// )?;
    /// bridge.driver_mut().start()?;
    ///
    /// let mut rx_buffer = [0; 1536];
    /// bridge.run(&mut ppp_io, &mut rx_buffer).await?;
    /// ```
    pub struct AsyncEspNetifChannel<'d, T, const TX_QUEUE_SIZE: usize = ASYNC_NETIF_TX_QUEUE_SIZE>
    where
        T: BorrowMut<EspNetif>,
    {
        driver: EspNetifDriver<'d, T>,
        tx_queue: Arc<Channel<EspRawMutex, Vec<u8>, TX_QUEUE_SIZE>>,
    }

    impl<T, const TX_QUEUE_SIZE: usize> AsyncEspNetifChannel<'static, T, TX_QUEUE_SIZE>
    where
        T: BorrowMut<EspNetif>,
    {
        /// Creates an asynchronous adapter around the provided ESP-NETIF instance.
        pub fn new<P>(netif: T, post_attach_cfg: P) -> Result<Self, EspError>
        where
            P: FnMut(&mut EspNetif) -> Result<(), EspError> + Send + 'static,
        {
            Self::new_nonstatic(netif, post_attach_cfg)
        }
    }

    impl<'d, T, const TX_QUEUE_SIZE: usize> AsyncEspNetifChannel<'d, T, TX_QUEUE_SIZE>
    where
        T: BorrowMut<EspNetif>,
    {
        /// Creates an asynchronous adapter which may borrow non-static data from
        /// `post_attach_cfg`.
        ///
        /// # Safety
        ///
        /// This has the same leak-safety requirements as
        /// [`EspNetifDriver::new_nonstatic`]. The adapter must not be forgotten.
        pub fn new_nonstatic<P>(netif: T, post_attach_cfg: P) -> Result<Self, EspError>
        where
            P: FnMut(&mut EspNetif) -> Result<(), EspError> + Send + 'd,
        {
            if TX_QUEUE_SIZE == 0 {
                return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
            }

            let tx_queue = Arc::new(Channel::new());
            let callback_queue = tx_queue.clone();
            let driver = EspNetifDriver::new_nonstatic(netif, post_attach_cfg, move |data| {
                callback_queue
                    .try_send(Vec::from(data))
                    .map_err(|_| EspError::from_infallible::<ESP_ERR_NO_MEM>())
            })?;

            Ok(Self { driver, tx_queue })
        }

        /// Reads one chunk from a PPP data stream and passes it to ESP-NETIF.
        ///
        /// Repeatedly call this concurrently with [`Self::transmit`]. A return
        /// value of zero has the meaning assigned by the underlying reader.
        pub async fn receive<R>(
            &self,
            mut reader: R,
            buffer: &mut [u8],
        ) -> Result<usize, AsyncEspNetifChannelError<R::Error>>
        where
            R: Read,
        {
            let len = reader
                .read(buffer)
                .await
                .map_err(AsyncEspNetifChannelError::Read)?;

            if len > 0 {
                self.driver
                    .rx(&buffer[..len])
                    .map_err(AsyncEspNetifChannelError::Netif)?;
            }

            Ok(len)
        }

        /// Writes one packet produced by ESP-NETIF to a PPP data stream.
        ///
        /// Repeatedly call this concurrently with [`Self::receive`].
        pub async fn transmit<W>(&self, mut writer:  W) -> Result<usize, W::Error>
        where
            W: Write,
        {
            let packet = self.tx_queue.receive().await;
            let len = packet.len();
            writer.write_all(&packet).await?;

            Ok(len)
        }

        /// Runs both directions of the bridge over one bidirectional PPP stream.
        ///
        /// This is convenient for transports such as `a76xx::PppIo` which implement
        /// both [`embedded_io_async::Read`] and [`embedded_io_async::Write`] but do
        /// not expose separate halves. The transport's read operation must be
        /// cancellation-safe: it is cancelled whenever ESP-NETIF has an outbound
        /// packet ready to write.
        ///
        /// The method returns successfully if the transport reports end-of-stream
        /// by returning zero bytes.
        pub async fn run<IO>(
            &self,
            mut io: IO,
            rx_buffer: &mut [u8],
        ) -> Result<(), AsyncEspNetifChannelError<IO::Error>>
        where
            IO: Read + Write,
        {
            loop {
                match select(io.read(rx_buffer), self.tx_queue.receive()).await {
                    Either::First(result) => {
                        let len = result.map_err(AsyncEspNetifChannelError::Read)?;
                        if len == 0 {
                            return Ok(());
                        }
                        self.driver
                            .rx(&rx_buffer[..len])
                            .map_err(AsyncEspNetifChannelError::Netif)?;
                    }
                    Either::Second(packet) => io
                        .write_all(&packet)
                        .await
                        .map_err(AsyncEspNetifChannelError::Write)?,
                }
            }
        }

        /// Returns the underlying ESP-NETIF driver.
        pub fn driver(&self) -> &EspNetifDriver<'d, T> {
            &self.driver
        }

        /// Returns the underlying ESP-NETIF driver mutably.
        pub fn driver_mut(&mut self) -> &mut EspNetifDriver<'d, T> {
            &mut self.driver
        }

        /// Unwraps the adapter and returns the underlying ESP-NETIF driver.
        pub fn into_driver(self) -> EspNetifDriver<'d, T> {
            self.driver
        }
    }

    impl<T> EspNetifDriver<'static, T>
    where
        T: BorrowMut<EspNetif>,
    {
        /// Create a new netif driver around the provided `EspNetif` instance.
        ///
        /// The driver transport is represented by:
        /// - The `tx` callback that the driver would call when it wants to ingest a packet
        ///   into the underlying transport
        /// - `EsNetifDriver::rx`, which should be called by the transport when a new packet
        ///   has arrived that has to be ingested in the driver
        ///   
        /// The transport can be anything, but with - say - PPP netif - it would typically be UART,
        /// and the `tx` callback implementation is simply expected to write the PPP packet into UART.
        ///
        /// Arguments:
        /// - `netif` is the `EspNetif` instance that the driver will manage
        /// - `got_ip_event_id` and `lost_ip_event_id` are the event IDs that the driver
        ///   will listen to so that it can connect/disconnect the netif upon receival
        ///   / loss of IP
        /// - `post_attach_cfg` is a netif-specific configuration that will be executed
        ///   after the netif is attached. For example, for a PPP netif, the post attach
        ///   configuration might want to invoke `EspNetif::set_ppp_conf`.
        /// - `tx` is the callback that the driver will call when it wants to ingest a packet
        ///   into the underlying transport
        ///
        /// Example:
        /// ```ignore
        /// let (uart_rx, uart_tx) = uart.into_split();
        ///
        /// let mut driver = EspNetifDriver::new(
        ///     EspNetif::new(NetifStack::Ppp)?,
        ///     |netif| netif.set_ppp_conf(&PppConfiguration {
        ///         phase_events_enabled: false,
        ///         ..Default::default()
        ///     }),
        ///     move |data| uart_tx.write_all(data),
        /// )?;
        ///
        /// loop {
        ///     let mut buffer = [0; 128];
        ///     let len = uart_rx.read(&mut buffer)?;
        ///     driver.rx(&buffer[..len])?;
        /// }
        /// ```
        ///
        pub fn new<P, F>(netif: T, post_attach_cfg: P, tx: F) -> Result<Self, EspError>
        where
            P: FnMut(&mut EspNetif) -> Result<(), EspError> + Send + 'static,
            F: FnMut(&[u8]) -> Result<(), EspError> + Send + 'static,
        {
            Self::new_nonstatic(netif, post_attach_cfg, tx)
        }
    }

    impl<'d, T> EspNetifDriver<'d, T>
    where
        T: BorrowMut<EspNetif>,
    {
        /// Create a new netif driver around the provided `EspNetif` instance.
        ///
        /// The driver transport is represented by:
        /// - The `tx` callback that the driver would call when it wants to ingest a packet
        ///   into the underlying transport
        /// - `EsNetifDriver::rx`, which should be called by the transport when a new packet
        ///   has arrived that has to be ingested in the driver
        ///   
        /// The transport can be anything, but with - say - PPP netif - it would typically be UART,
        /// and the `tx` callback implementation is simply expected to write the PPP packet into UART.
        ///
        /// Arguments:
        /// - `netif` is the `EspNetif` instance that the driver will manage
        /// - `got_ip_event_id` and `lost_ip_event_id` are the event IDs that the driver
        ///   will listen to so that it can connect/disconnect the netif upon receival
        ///   / loss of IP
        /// - `post_attach_cfg` is a netif-specific configuration that will be executed
        ///   after the netif is attached. For example, for a PPP netif, the post attach
        ///   configuration might want to invoke `EspNetif::set_ppp_conf`.
        /// - `tx` is the callback that the driver will call when it wants to ingest a packet
        ///   into the underlying transport
        ///
        /// Example:
        /// ```ignore
        /// let (uart_rx, uart_tx) = uart.into_split();
        ///
        /// let mut driver = EspNetifDriver::new(
        ///     EspNetif::new(NetifStack::Ppp)?,
        ///     |netif| netif.set_ppp_conf(&PppConfiguration {
        ///         phase_events_enabled: false,
        ///         ..Default::default()
        ///     }),
        ///     move |data| uart_tx.write_all(data),
        /// )?;
        ///
        /// loop {
        ///     let mut buffer = [0; 128];
        ///     let len = uart_rx.read(&mut buffer)?;
        ///     driver.rx(&buffer[..len])?;
        /// }
        /// ```
        ///
        /// # Safety
        ///
        /// This method - in contrast to method `new` - allows the user to pass
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
        pub fn new_nonstatic<P, F>(netif: T, post_attach_cfg: P, tx: F) -> Result<Self, EspError>
        where
            P: FnMut(&mut EspNetif) -> Result<(), EspError> + Send + 'd,
            F: FnMut(&[u8]) -> Result<(), EspError> + Send + 'd,
        {
            let mut inner = alloc::boxed::Box::new(EspNetifDriverInner {
                base: esp_netif_driver_base_t {
                    netif: netif.borrow().handle(),
                    post_attach: Some(EspNetifDriverInner::<T>::raw_post_attach),
                },
                netif,
                post_attach_cfg: alloc::boxed::Box::new(post_attach_cfg),
                tx: alloc::boxed::Box::new(tx),
            });

            let inner_ptr = inner.as_mut() as *mut _ as *mut core::ffi::c_void;

            if let Some(got_ip_event_id) = inner.netif.borrow()._got_ip_event_id {
                esp!(unsafe {
                    esp_event_handler_register(
                        IP_EVENT,
                        got_ip_event_id.get() as _,
                        Some(esp_netif_action_connected),
                        inner.netif.borrow().handle() as *mut core::ffi::c_void,
                    )
                })?;
            }

            if let Some(lost_ip_event_id) = inner.netif.borrow()._lost_ip_event_id {
                esp!(unsafe {
                    esp_event_handler_register(
                        IP_EVENT,
                        lost_ip_event_id.get() as _,
                        Some(esp_netif_action_disconnected),
                        inner.netif.borrow().handle() as *mut core::ffi::c_void,
                    )
                })?;
            }

            esp!(unsafe { esp_netif_attach(inner.netif.borrow().handle(), inner_ptr) })?;

            Ok(Self {
                inner,
                started: false,
            })
        }

        /// Ingest a packet into the driver
        ///
        /// The packet can arrive from anywhere, but with say - a PPP netif -
        /// it would be a PPP packet arriving typically from UART, by reading from it.
        pub fn rx(&self, data: &[u8]) -> Result<(), EspError> {
            esp!(unsafe {
                esp_netif_receive(
                    self.inner.netif.borrow().handle(),
                    data.as_ptr() as *mut core::ffi::c_void,
                    data.len() as _,
                    core::ptr::null_mut(),
                )
            })
        }

        /// Start the driver
        pub fn start(&mut self) -> Result<(), EspError> {
            if self.started {
                return Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>());
            }

            unsafe {
                esp_netif_action_start(
                    self.inner.netif.borrow().handle() as *mut core::ffi::c_void,
                    core::ptr::null_mut(),
                    0,
                    core::ptr::null_mut(),
                );
            }

            self.started = true;

            Ok(())
        }

        /// Stop the driver
        pub fn stop(&mut self) -> Result<(), EspError> {
            if !self.started {
                return Err(EspError::from_infallible::<ESP_ERR_INVALID_STATE>());
            }

            unsafe {
                esp_netif_action_stop(
                    self.inner.netif.borrow().handle() as *mut core::ffi::c_void,
                    core::ptr::null_mut(),
                    0,
                    core::ptr::null_mut(),
                );
            }

            self.started = false;

            Ok(())
        }

        /// Check if the driver is started
        pub fn is_started(&self) -> Result<bool, EspError> {
            Ok(self.started)
        }

        /// Get a reference to the underlying `EspNetif` instance
        pub fn netif(&self) -> &EspNetif {
            self.inner.netif.borrow()
        }

        /// Get a mutable reference to the underlying `EspNetif` instance
        pub fn netif_mut(&mut self) -> &mut EspNetif {
            self.inner.netif.borrow_mut()
        }
    }

    impl<T> Drop for EspNetifDriver<'_, T>
    where
        T: BorrowMut<EspNetif>,
    {
        fn drop(&mut self) {
            let _ = self.stop();

            if let Some(got_ip_event_id) = self.inner.netif.borrow()._got_ip_event_id {
                esp!(unsafe {
                    esp_event_handler_unregister(
                        IP_EVENT,
                        got_ip_event_id.get() as _,
                        Some(esp_netif_action_connected),
                    )
                })
                .unwrap();
            }

            if let Some(lost_ip_event_id) = self.inner.netif.borrow()._lost_ip_event_id {
                esp!(unsafe {
                    esp_event_handler_unregister(
                        IP_EVENT,
                        lost_ip_event_id.get() as _,
                        Some(esp_netif_action_disconnected),
                    )
                })
                .unwrap();
            }
        }
    }

    #[repr(C)]
    struct EspNetifDriverInner<'d, T>
    where
        T: BorrowMut<EspNetif>,
    {
        base: esp_netif_driver_base_t,
        netif: T,
        #[allow(clippy::type_complexity)]
        tx: alloc::boxed::Box<dyn FnMut(&[u8]) -> Result<(), EspError> + Send + 'd>,
        #[allow(clippy::type_complexity)]
        post_attach_cfg:
            alloc::boxed::Box<dyn FnMut(&mut EspNetif) -> Result<(), EspError> + Send + 'd>,
    }

    impl<T> EspNetifDriverInner<'_, T>
    where
        T: BorrowMut<EspNetif>,
    {
        fn post_attach(&mut self, netif_handle: *mut esp_netif_obj) -> Result<(), EspError> {
            let driver_ifconfig = esp_netif_driver_ifconfig_t {
                transmit: Some(Self::raw_tx),
                handle: self as *mut _ as *mut core::ffi::c_void,
                ..Default::default()
            };

            debug!("Post attach ifconfig: {driver_ifconfig:?}");

            // d->base.netif = esp_netif; TODO: This is weird; the netif in base is already set on constructor?

            esp!(unsafe { esp_netif_set_driver_config(netif_handle, &driver_ifconfig) })?;

            (self.post_attach_cfg)(self.netif.borrow_mut())?;

            Ok(())
        }

        unsafe extern "C" fn raw_tx(
            h: *mut core::ffi::c_void,
            buffer: *mut core::ffi::c_void,
            len: usize,
        ) -> i32 {
            let this = h as *mut Self;
            let data = core::slice::from_raw_parts(buffer as *mut u8, len);
            // Only the callback field is accessed here. Creating `&mut Self`
            // would incorrectly claim exclusive access to the netif while its
            // RX side can be active on another task.
            let tx = unsafe { &mut *core::ptr::addr_of_mut!((*this).tx) };
            let result = match tx(data) {
                Ok(_) => ESP_OK,
                Err(e) => e.code(),
            };

            // TODO: Might not be necessary, but if I remember correctly, the Netif API
            // wanted that _we_ free the buffer; in any case needs to be compared with the C ESP Modem code
            // free(buffer);

            result
        }

        unsafe extern "C" fn raw_post_attach(
            netif: *mut esp_netif_obj,
            args: *mut core::ffi::c_void,
        ) -> i32 {
            let this = { (args as *mut Self).as_mut() }.unwrap();
            match this.post_attach(netif) {
                Ok(_) => ESP_OK,
                Err(e) => e.code(),
            }
        }
    }
}

#[cfg(esp_idf_lwip_ppp_support)]
mod ppp {
    use core::ffi::{self, CStr};

    use enumset::{EnumSet, EnumSetType};

    use crate::eventloop::{EspEventDeserializer, EspEventSource};
    use crate::handle::RawHandle;
    use crate::sys::*;

    /// Represents a PPP event on the system event loop
    #[derive(Copy, Clone, Debug)]
    pub enum PppEvent {
        /// No error
        NoError,
        /// Invalid parameter
        ParameterError,
        /// Unable to open PPP session
        OpenError,
        /// Invalid I/O device for PPP
        DeviceError,
        /// Unable to allocate resources
        AllocError,
        /// User interrupt
        UserError,
        /// Connection lost
        DisconnectError,
        /// Failed authentication challenge
        AuthFailError,
        /// Failed to meet protocol
        ProtocolError,
        /// Connection timeout
        PeerDeadError,
        /// Idle Timeout
        IdleTimeoutError,
        /// Max connect time reached
        MaxConnectTimeoutError,
        /// Loopback detected
        LoopbackError,
        PhaseDead,
        PhaseMaster,
        PhaseHoldoff,
        PhaseInitialize,
        PhaseSerialConnection,
        PhaseDormant,
        PhaseEstablish,
        PhaseAuthenticate,
        PhaseCallback,
        PhaseNetwork,
        PhaseRunning,
        PhaseTerminate,
        PhaseDisconnect,
        PhaseFailed,

        /// An event ID not recognised by this version of the library was received.
        ///
        /// This variant is produced instead of panicking when an unknown event ID
        /// arrives, allowing applications to remain forward-compatible with
        /// ESP-IDF versions that introduce new PPP events.
        Other(i32),
    }

    unsafe impl EspEventSource for PppEvent {
        fn source() -> Option<&'static core::ffi::CStr> {
            Some(unsafe { ffi::CStr::from_ptr(NETIF_PPP_STATUS) })
        }
    }

    impl EspEventDeserializer for PppEvent {
        type Data<'a> = PppEvent;

        #[allow(non_upper_case_globals, non_snake_case)]
        fn deserialize<'a>(data: &crate::eventloop::EspEvent<'a>) -> Self::Data<'a> {
            let event_id = data.event_id as u32;

            match event_id {
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORNONE => PppEvent::NoError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORPARAM => PppEvent::ParameterError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERROROPEN => PppEvent::OpenError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORDEVICE => PppEvent::DeviceError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORALLOC => PppEvent::AllocError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORUSER => PppEvent::UserError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORCONNECT => PppEvent::DisconnectError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORAUTHFAIL => PppEvent::AuthFailError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORPROTOCOL => PppEvent::ProtocolError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORPEERDEAD => PppEvent::PeerDeadError,
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORIDLETIMEOUT => {
                    PppEvent::IdleTimeoutError
                }
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORCONNECTTIME => {
                    PppEvent::MaxConnectTimeoutError
                }
                esp_netif_ppp_status_event_t_NETIF_PPP_ERRORLOOPBACK => PppEvent::LoopbackError,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_DEAD => PppEvent::PhaseDead,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_MASTER => PppEvent::PhaseMaster,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_HOLDOFF => PppEvent::PhaseHoldoff,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_INITIALIZE => {
                    PppEvent::PhaseInitialize
                }
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_SERIALCONN => {
                    PppEvent::PhaseSerialConnection
                }
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_DORMANT => PppEvent::PhaseDormant,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_ESTABLISH => PppEvent::PhaseEstablish,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_AUTHENTICATE => {
                    PppEvent::PhaseAuthenticate
                }
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_CALLBACK => PppEvent::PhaseCallback,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_NETWORK => PppEvent::PhaseNetwork,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_RUNNING => PppEvent::PhaseRunning,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_TERMINATE => PppEvent::PhaseTerminate,
                esp_netif_ppp_status_event_t_NETIF_PPP_PHASE_DISCONNECT => {
                    PppEvent::PhaseDisconnect
                }
                esp_netif_ppp_status_event_t_NETIF_PPP_CONNECT_FAILED => PppEvent::PhaseFailed,
                _ => {
                    ::log::warn!("PppEvent: unknown event ID {event_id}, ignoring");
                    PppEvent::Other(event_id as i32)
                }
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq, Hash)]
    pub struct PppConfiguration {
        /// Enables events coming from PPP PHASE change
        pub phase_events_enabled: bool,
        /// Enables events from main PPP state machine producing errors
        pub error_events_enabled: bool,
        /// Allows to temporarily disable LCP keepalive (runtime, if enabled compile time)
        /// When LCP echo is enabled in menuconfig, this option can be used to override the setting,
        /// if we have to relax LCP keepalive criteria during runtime operation, for example before OTA update.
        /// The current session must be closed, settings will be applied upon connecting.
        #[cfg(esp_idf_lwip_enable_lcp_echo)]
        pub lcp_echo_disabled: bool,
        /// Set our preferred address, typically used when we're the PPP server
        #[cfg(esp_idf_lwip_ppp_server_support)]
        our_ip4_addr: core::net::Ipv4Addr,
        /// Set our preferred address, typically used when we're the PPP server        
        #[cfg(esp_idf_lwip_ppp_server_support)]
        their_ip4_addr: core::net::Ipv4Addr,
    }

    impl PppConfiguration {
        pub const fn new() -> Self {
            Self {
                phase_events_enabled: true,
                error_events_enabled: true,
                #[cfg(esp_idf_lwip_enable_lcp_echo)]
                lcp_echo_disabled: false,
                #[cfg(esp_idf_lwip_ppp_server_support)]
                our_ip4_addr: core::net::Ipv4Addr::UNSPECIFIED,
                #[cfg(esp_idf_lwip_ppp_server_support)]
                their_ip4_addr: core::net::Ipv4Addr::UNSPECIFIED,
            }
        }
    }

    impl Default for PppConfiguration {
        fn default() -> Self {
            Self::new()
        }
    }

    impl From<esp_netif_ppp_config_t> for PppConfiguration {
        fn from(cfg: esp_netif_ppp_config_t) -> Self {
            Self {
                phase_events_enabled: cfg.ppp_phase_event_enabled,
                error_events_enabled: cfg.ppp_error_event_enabled,
                #[cfg(esp_idf_lwip_enable_lcp_echo)]
                lcp_echo_disabled: cfg.ppp_lcp_echo_disabled,
                #[cfg(esp_idf_lwip_ppp_server_support)]
                our_ip4_addr: Newtype::<core::net::Ipv4Addr>::from(cfg.our_ip4_addr),
                #[cfg(esp_idf_lwip_ppp_server_support)]
                their_ip4_addr: Newtype::<core::net::Ipv4Addr>::from(cfg.their_ip4_addr),
            }
        }
    }

    impl From<&PppConfiguration> for esp_netif_ppp_config_t {
        fn from(cfg: &PppConfiguration) -> Self {
            Self {
                ppp_phase_event_enabled: cfg.phase_events_enabled,
                ppp_error_event_enabled: cfg.error_events_enabled,
                #[cfg(esp_idf_lwip_enable_lcp_echo)]
                ppp_lcp_echo_disabled: cfg.lcp_echo_disabled,
                #[cfg(esp_idf_lwip_ppp_server_support)]
                our_ip4_addr: Newtype::<esp_ip4_addr_t>::from(cfg.our_ip4_addr),
                #[cfg(esp_idf_lwip_ppp_server_support)]
                their_ip4_addr: Newtype::<esp_ip4_addr_t>::from(cfg.their_ip4_addr),
            }
        }
    }

    #[derive(Debug, EnumSetType)]
    #[enumset(repr = "u32")]
    pub enum PppAuthentication {
        Pap,
        Chap,
        MsChap,
        MsChapV2,
        Eap,
    }

    /// PPP-specific configuration of a Netif
    impl super::EspNetif {
        /// Get the current PPP configuration
        #[cfg(not(esp_idf_version_major = "4"))]
        pub fn get_ppp_conf(&self) -> Result<PppConfiguration, EspError> {
            let mut ppp_config = Default::default();

            esp!(unsafe { esp_netif_ppp_get_params(self.handle(), &mut ppp_config) })?;

            Ok(ppp_config.into())
        }

        /// Set the PPP configuration
        pub fn set_ppp_conf(&mut self, conf: &PppConfiguration) -> Result<(), EspError> {
            let ppp_config: esp_netif_ppp_config_t = conf.into();

            esp!(unsafe { esp_netif_ppp_set_params(self.handle(), &ppp_config) })
        }

        /// Set the PPP authentication
        /// Arguments:
        /// - `auth` is the set of all authentication methods to allow; if empty, no authentication will be used
        /// - `username` is the username to use for authentication; only relevant when the `auth` enumset is non-empty
        /// - `password` is the password to use for authentication; only relevant when the `auth` enumset is non-empty
        pub fn set_ppp_auth(
            &mut self,
            auth: EnumSet<PppAuthentication>,
            username: &CStr,
            password: &CStr,
        ) -> Result<(), EspError> {
            esp!(unsafe {
                esp_netif_ppp_set_auth(
                    self.handle(),
                    auth.as_repr(),
                    username.as_ptr() as _,
                    password.as_ptr() as _,
                )
            })
        }
    }
}

pub mod asynch {
    use crate::sys::EspError;

    pub trait NetifStatus {
        async fn is_up(&self) -> Result<bool, EspError>;
    }

    impl<T> NetifStatus for &T
    where
        T: NetifStatus,
    {
        async fn is_up(&self) -> Result<bool, EspError> {
            (**self).is_up().await
        }
    }

    impl<T> NetifStatus for &mut T
    where
        T: NetifStatus,
    {
        async fn is_up(&self) -> Result<bool, EspError> {
            (**self).is_up().await
        }
    }

    impl NetifStatus for super::EspNetif {
        async fn is_up(&self) -> Result<bool, EspError> {
            super::EspNetif::is_up(self)
        }
    }

    #[cfg(all(feature = "alloc", esp_idf_comp_esp_timer_enabled))]
    impl<T> NetifStatus for super::AsyncNetif<T>
    where
        T: super::NetifStatus,
    {
        async fn is_up(&self) -> Result<bool, EspError> {
            super::AsyncNetif::is_up(self)
        }
    }
}
