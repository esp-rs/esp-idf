# Shared sdkconfig defaults for the whole workspace: the crates, their tests and all the examples.
# Its variants used by CI and by specific examples live next to it (`sdkconfig.defaults.nimble`, `sdkconfig.defaults.a2dp_external_codec`,
# and `sdkconfig.defaults.newlib` / `sdkconfig.defaults.picolibc`, which CI uses on top of this file).

# The examples require a larger than the default stack size for the main thread, the posix threads,
# the sysloop event task and the esp-timer task (necessary when the `embassy-time-driver` feature is enabled).
CONFIG_ESP_MAIN_TASK_STACK_SIZE=20000
CONFIG_ESP_SYSTEM_EVENT_TASK_STACK_SIZE=8192
CONFIG_ESP_TIMER_TASK_STACK_SIZE=4096
CONFIG_PTHREAD_TASK_STACK_SIZE_DEFAULT=8192

# Go figure...
CONFIG_FREERTOS_IDLE_TASK_STACKSIZE=4096

# Async SPI only enabled when this config is disabled (it is enabled by default)
CONFIG_SPI_MASTER_ISR_IN_IRAM=n

# Suppress deprecation warnings for legacy drivers still included in bindings
CONFIG_I2C_SUPPRESS_DEPRECATE_WARN=y
CONFIG_TWAI_SUPPRESS_DEPRECATE_WARN=y
CONFIG_TOUCH_SUPPRESS_DEPRECATE_WARN=y

CONFIG_MBEDTLS_CERTIFICATE_BUNDLE=y
CONFIG_MBEDTLS_CERTIFICATE_BUNDLE_DEFAULT_FULL=y

# Enable WS support
CONFIG_HTTPD_WS_SUPPORT=y

# SPI Ethernet demo
CONFIG_ETH_SPI_ETHERNET_DM9051=y
CONFIG_ETH_SPI_ETHERNET_W5500=y
CONFIG_ETH_SPI_ETHERNET_KSZ8851SNL=y

# BLE with NimBLE.
#
# NimBLE and Bluedroid are a mutually-exclusive Kconfig `choice` (components/bt/Kconfig), so a
# single build can only exercise one host. This file is the NimBLE counterpart of the workspace
# `sdkconfig.defaults` (which selects Bluedroid); CI builds against both so the NimBLE-only code
# paths (`esp_idf_svc::ble` and the `ble_*` examples) get compile-checked too. NimBLE is BLE-only,
# so there is no BT Classic / SPP here.
CONFIG_BT_ENABLED=y
CONFIG_BT_NIMBLE_ENABLED=y

# Enable L2CAP connection-oriented channels (CoC), so the `esp_idf_svc::ble` L2CAP code paths get
# compile-checked in CI.
CONFIG_BT_NIMBLE_L2CAP_COC_MAX_NUM=3

# Support for TLS with a pre-shared key.
#CONFIG_ESP_TLS_PSK_VERIFICATION=y

# Compile-check the MQTT 5.0 code paths in CI.
CONFIG_MQTT_PROTOCOL_5=y

CONFIG_LWIP_PPP_SUPPORT=y
#CONFIG_LWIP_SLIP_SUPPORT=y

# Generic Thread functionality
CONFIG_OPENTHREAD_ENABLED=y

# Thread Border Router
#CONFIG_OPENTHREAD_BORDER_ROUTER=y

# These are also necessary for the Joiner feature
#CONFIG_MBEDTLS_CMAC_C=y
#CONFIG_MBEDTLS_SSL_PROTO_DTLS=y
#CONFIG_MBEDTLS_KEY_EXCHANGE_ECJPAKE=y
#CONFIG_MBEDTLS_ECJPAKE_C=y

# Border Router again, LWIP
#CONFIG_LWIP_IPV6_NUM_ADDRESSES=12
#CONFIG_LWIP_NETIF_STATUS_CALLBACK=y
#CONFIG_LWIP_IPV6_FORWARD=y
#CONFIG_LWIP_MULTICAST_PING=y
#CONFIG_LWIP_NETIF_STATUS_CALLBACK=y
#CONFIG_LWIP_HOOK_IP6_ROUTE_DEFAULT=y
#CONFIG_LWIP_HOOK_ND6_GET_GW_DEFAULT=y
#CONFIG_LWIP_HOOK_IP6_INPUT_CUSTOM=y
#CONFIG_LWIP_HOOK_IP6_SELECT_SRC_ADDR_CUSTOM=y
#CONFIG_LWIP_IPV6_AUTOCONFIG=y
#CONFIG_LWIP_TCPIP_TASK_STACK_SIZE=4096

# Border Router again, mDNS
#CONFIG_MDNS_MULTIPLE_INSTANCE=y

# Border Router again, Wifi/IEEE802.15.4 software co-exist
#CONFIG_ESP_COEX_SW_COEXIST_ENABLE=y

# Littlefs on SD cards example
#CONFIG_LITTLEFS_SDMMC_SUPPORT=y
