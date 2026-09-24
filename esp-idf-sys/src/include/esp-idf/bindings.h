#include "esp_system.h"

#if ((ESP_IDF_VERSION_MAJOR < 4) || ((ESP_IDF_VERSION_MAJOR == 4) && (ESP_IDF_VERSION_MINOR < 3)) || ((ESP_IDF_VERSION_MAJOR == 4) && (ESP_IDF_VERSION_MINOR == 3) && (ESP_IDF_VERSION_PATCH < 2)))
#error Only ESP-IDF versions >= V4.3.2 are currently supported; if you are using the PIO build (the default one), wipe out your `.embuild` folder and try again with a clean rebuild
#endif

// Standard C library headers whose types and constants are mirrored by the `libc` crate
// and cross-checked in `src/checks/libc.rs`; included explicitly so that the bindings
// contain them regardless of which ESP-IDF components are built
#include <errno.h>
#include <fcntl.h>

#include "esp_rom_crc.h"
#include "esp_log.h"
#include "esp_debug_helpers.h"

#include "esp_sleep.h"
#include "esp_task.h"
#include "esp_task_wdt.h"
#if ESP_IDF_VERSION_MAJOR < 6
#include "esp_interface.h"
#endif
#include "esp_ipc.h"
#include "esp_mac.h"
#include "esp_freertos_hooks.h"

#include "freertos/FreeRTOS.h"
#include "freertos/atomic.h"
#include "freertos/event_groups.h"
#include "freertos/list.h"
#include "freertos/message_buffer.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/stream_buffer.h"
#include "freertos/task.h"
#include "freertos/task_snapshot.h"
#include "freertos/timers.h"

#include "esp_heap_task_info.h"

#if CONFIG_IDF_TARGET_ESP32
#if ESP_IDF_VERSION_MAJOR == 4
#include "esp32/himem.h"
#else
#ifdef ESP_IDF_COMP_ESP_PSRAM_ENABLED
#include "esp32/himem.h"
#endif // ESP_IDF_COMP_ESP_PSRAM_ENABLED
#endif // ESP_IDF_VERSION_MAJOR == 4
#endif // CONFIG_IDF_TARGET_ESP32

#if ESP_IDF_VERSION_MAJOR == 4
#include "esp_spiram.h"
#else
#ifdef ESP_IDF_COMP_ESP_PSRAM_ENABLED
#include "esp_psram.h"
#endif // ESP_IDF_COMP_ESP_PSRAM_ENABLED
#endif // ESP_IDF_VERSION_MAJOR == 4

#if ESP_IDF_VERSION_MAJOR == 4
#include "esp_int_wdt.h"
#else
#include "esp_private/esp_int_wdt.h"
#endif

#ifdef ESP_IDF_COMP_CONSOLE_ENABLED
#include "esp_console.h"
#include "linenoise/linenoise.h"
#include "argtable3/argtable3.h"
#endif

#ifdef ESP_IDF_COMP_EFUSE_ENABLED
#include "esp_efuse.h"
#include "esp_efuse_table.h"
#endif

#ifdef ESP_IDF_COMP_ESP_PM_ENABLED
#include "esp_pm.h"
#endif

#ifdef ESP_IDF_COMP_ESP_TIMER_ENABLED
#include "esp_timer.h"
#endif

#if ESP_IDF_VERSION_MAJOR > 4
#ifdef ESP_IDF_COMP_SPI_FLASH_ENABLED
#include "esp_flash.h"
#if ESP_IDF_VERSION_MAJOR > 5
#include "spi_flash_mmap.h"
#else
#include "esp_spi_flash.h"
#endif
#endif
#ifdef ESP_IDF_COMP_ESP_PARTITION_ENABLED
#include "esp_partition.h"
#endif
#else
#ifdef ESP_IDF_COMP_SPI_FLASH_ENABLED
#include "esp_spi_flash.h"
#include "esp_partition.h"
#endif
#endif

#if defined(ESP_IDF_COMP_ESP_ADC_CAL_ENABLED) || defined(ESP_IDF_COMP_ESP_ADC_ENABLED)
#if ESP_IDF_VERSION_MAJOR < 6
#include "esp_adc_cal.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 4
#include "esp_adc/adc_cali.h"
#include "esp_adc/adc_cali_scheme.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_EVENT_ENABLED
#include "esp_event.h"
#endif

#ifdef ESP_IDF_COMP_ESP_NETIF_ENABLED
#include "esp_netif.h"
#if ESP_IDF_VERSION_MAJOR > 4
#include "esp_netif_net_stack.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 4 && defined(CONFIG_ESP_NETIF_TCPIP_LWIP) && defined(CONFIG_ESP_NETIF_BRIDGE_EN)
#include "esp_netif_br_glue.h"
#endif
#ifdef CONFIG_LWIP_PPP_SUPPORT
#include "esp_netif_ppp.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_WIFI_ENABLED
#include "esp_wifi.h"
#ifdef ESP_IDF_COMP_ESP_NETIF_ENABLED
#include "esp_wifi_netif.h"
#endif
#include "esp_now.h"
#include "esp_mesh.h"
#if ESP_IDF_VERSION_MAJOR > 5
#include "esp_eap_client.h"
#else
#include "esp_wpa2.h"
#endif
#if (ESP_IDF_VERSION_MAJOR < 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR < 1)
#include "esp_coexist.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_WIFI_REMOTE_ENABLED
#include "esp_wifi_remote.h"
#endif

#ifdef ESP_IDF_COMP_ESP_HOSTED_ENABLED
#include "esp_hosted.h"
#endif

#if defined(ESP_IDF_COMP_ESP_COEX_ENABLED) && (ESP_IDF_VERSION_MAJOR > 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 1)
#include "esp_coexist.h"
#ifdef CONFIG_SOC_IEEE802154_SUPPORTED
#include "esp_coex_i154.h"
#endif
#endif

#ifdef ESP_IDF_COMP_WPA_SUPPLICANT_ENABLED
#if defined(CONFIG_ESP_WIFI_DPP_SUPPORT) || defined(CONFIG_WPA_DPP_SUPPORT)
#include "esp_dpp.h"
#endif
#if defined(CONFIG_ESP_WIFI_MBO_SUPPORT) || defined(CONFIG_WPA_MBO_SUPPORT)
#include "esp_mbo.h"
#endif
#include "esp_rrm.h"
#include "esp_wnm.h"
#include "esp_wpa.h"
#if ESP_IDF_VERSION_MAJOR > 5
#include "esp_eap_client.h"
#else
#include "esp_wpa2.h"
#endif
#include "esp_wps.h"
#if ESP_IDF_VERSION_MAJOR > 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 1
#include "esp_supplicant_utils.h"
#endif
// #if ESP_IDF_VERSION_MAJOR > 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 2
// #include "esp_eap_client.h"
// #endif
#endif

#ifdef ESP_IDF_COMP_ESP_ETH_ENABLED
#include "esp_eth.h"
#ifdef ESP_IDF_COMP_ESP_NETIF_ENABLED
#include "esp_eth_netif_glue.h"
#endif
#endif

// External Ethernet PHY drivers (managed components)
#ifdef ESP_IDF_COMP_ESPRESSIF__DP83848_ENABLED
#include "esp_eth_phy_dp83848.h"
#endif

#ifdef ESP_IDF_COMP_ESPRESSIF__IP101_ENABLED
#include "esp_eth_phy_ip101.h"
#endif

#ifdef ESP_IDF_COMP_ESPRESSIF__KSZ80XX_ENABLED
#include "esp_eth_phy_ksz80xx.h"
#endif

#ifdef ESP_IDF_COMP_ESPRESSIF__LAN87XX_ENABLED
#include "esp_eth_phy_lan87xx.h"
#endif

#ifdef ESP_IDF_COMP_ESPRESSIF__RTL8201_ENABLED
#include "esp_eth_phy_rtl8201.h"
#endif

// External Ethernet SPI drivers (managed components)
#ifdef ESP_IDF_COMP_ESPRESSIF__DM9051_ENABLED
#include "esp_eth_mac_dm9051.h"
#include "esp_eth_phy_dm9051.h"
#endif

#ifdef ESP_IDF_COMP_ESPRESSIF__W5500_ENABLED
#include "esp_eth_mac_w5500.h"
#include "esp_eth_phy_w5500.h"
#endif

#ifdef ESP_IDF_COMP_ESPRESSIF__KSZ8851SNL_ENABLED
#include "esp_eth_mac_ksz8851snl.h"
#include "esp_eth_phy_ksz8851snl.h"
#endif

// OpenThread support
#ifdef ESP_IDF_COMP_OPENTHREAD_ENABLED
#ifdef CONFIG_OPENTHREAD_ENABLED
#include "esp_openthread.h"
#include "esp_openthread_border_router.h"
#include "esp_openthread_cli.h"
#if (ESP_IDF_VERSION_MAJOR > 4)
#include "esp_openthread_dns64.h"
#endif
#include "esp_openthread_lock.h"
#include "esp_openthread_netif_glue.h"
#include "esp_openthread_types.h"
#include "openthread/dataset.h"
#include "openthread/instance.h"
#include "openthread/ip6.h"
#include "openthread/joiner.h"
#include "openthread/link.h"
#include "openthread/logging.h"
#include "openthread/message.h"
#if (ESP_IDF_VERSION_MAJOR > 4)
#include "openthread/nat64.h"
#endif
#include "openthread/netdata.h"
#include "openthread/thread.h"
#include "openthread/srp_client_buffers.h"
#include "openthread/srp_client.h"
#include "openthread/platform/settings.h"
#endif
#endif

#ifdef ESP_IDF_COMP_VFS_ENABLED
#include "esp_vfs.h"
#if ESP_IDF_VERSION_MAJOR < 6
#include "esp_vfs_cdcacm.h"
#endif
#include "esp_vfs_dev.h"
#include "esp_vfs_semihost.h"
#ifdef SOC_USB_SERIAL_JTAG_SUPPORTED
#if ESP_IDF_VERSION_MAJOR > 5
#include "driver/usb_serial_jtag_vfs.h"
#else
#include "esp_vfs_usb_serial_jtag.h"
#endif
#endif

#ifdef ESP_IDF_COMP_VFS_CONSOLE_ENABLED
#if ((ESP_IDF_VERSION_MAJOR > 4) || ((ESP_IDF_VERSION_MAJOR == 4) && (ESP_IDF_VERSION_MINOR >= 4)))
#include "esp_vfs_console.h"
#endif
#endif

#if ((ESP_IDF_VERSION_MAJOR > 4) || ((ESP_IDF_VERSION_MAJOR == 4) && (ESP_IDF_VERSION_MINOR >= 4)))
#include "esp_vfs_eventfd.h"
#endif

#ifdef ESP_IDF_COMP_SPIFFS_ENABLED
#include "esp_spiffs.h"
#endif

#ifdef ESP_IDF_COMP_FATFS_ENABLED
#ifdef ESP_IDF_COMP_VFS_ENABLED
#include "esp_vfs_fat.h"
#endif
#include "diskio_impl.h"
#include "diskio_rawflash.h"
#include "diskio_sdmmc.h"
#include "diskio_wl.h"
#endif

#if defined(ESP_IDF_COMP_JOLTWALLET__LITTLEFS_ENABLED)
#include "esp_littlefs.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_USB_CDC_ROM_CONSOLE_ENABLED
#include "esp_vfs_cdcacm.h"
#endif

#ifdef ESP_IDF_COMP_LWIP_ENABLED
#include "lwip/dns.h"
#if ESP_IDF_VERSION_MAJOR < 6
#include "lwip/lwip_napt.h"
#endif
#include "lwip/netdb.h"
#include "lwip/sockets.h"
// The POSIX wrapper of the ESP-IDF lwip port. Brings in `net/if.h`, where ESP-IDF defines
// the POSIX socket constants that are not in `lwip/sockets.h` (`MSG_DONTROUTE`, `SOMAXCONN`, `NI_MAXHOST`, ...)
#include <sys/socket.h>
#if ESP_IDF_VERSION_MAJOR > 4
#ifdef ESP_IDF_COMP_ESP_NETIF_ENABLED
#include "lwip/esp_netif_net_stack.h"
#endif
#endif
#include "esp_sntp.h"
#include "ping/ping_sock.h"
#if ESP_IDF_VERSION_MAJOR > 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 1
#ifdef ESP_IDF_COMP_ESP_NETIF_ENABLED
#include "esp_netif_sntp.h"
#endif
#endif
#endif

#ifdef ESP_IDF_COMP_MBEDTLS_ENABLED
#include "mbedtls/ssl.h"
#include "mbedtls/ssl_cache.h"
#include "mbedtls/ssl_cookie.h"
#include "mbedtls/ssl_ticket.h"
#if ESP_IDF_VERSION_MAJOR < 6
// Legacy crypto headers; dropped or made private by Mbed TLS 4 / TF-PSA-Crypto (ESP-IDF 6+)
#include "mbedtls/aes.h"
#include "mbedtls/cipher.h"
#include "mbedtls/entropy.h"
#include "mbedtls/ctr_drbg.h"
#include "mbedtls/cmac.h"
#include "mbedtls/ecdh.h"
#include "mbedtls/dhm.h"
#include "mbedtls/hkdf.h"
#include "mbedtls/hmac_drbg.h"
#include "mbedtls/pkcs5.h"
#include "mbedtls/pkcs12.h"
#endif
#include "mbedtls/ecp.h"
#include "mbedtls/platform.h"
#include "mbedtls/asn1write.h"
#include "mbedtls/oid.h"
#include "mbedtls/pem.h"
#include "mbedtls/nist_kw.h"
#include "mbedtls/version.h"
#if MBEDTLS_VERSION_NUMBER >= 0x03030000
// PKCS#7 and LMS only exist since Mbed TLS 3.3 (ESP-IDF 4.4 ships 2.28, ESP-IDF 5.0 ships 3.2)
#include "mbedtls/pkcs7.h"
#include "mbedtls/lms.h"
#endif
#include "mbedtls/x509_csr.h"
#include "mbedtls/debug.h"

#ifdef CONFIG_MBEDTLS_CERTIFICATE_BUNDLE
#include "esp_crt_bundle.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_TLS_ENABLED

// See https://github.com/espressif/esp-idf/issues/12541
#ifdef CONFIG_ESP_TLS_USING_MBEDTLS
#include "mbedtls/ssl.h"
#elif CONFIG_ESP_TLS_USING_WOLFSSL
#include "wolfssl/wolfcrypt/settings.h"
#include "wolfssl/ssl.h"
#endif

#include "esp_tls.h"
#endif

#ifdef ESP_IDF_COMP_BOOTLOADER_SUPPORT_ENABLED
#include "bootloader_common.h"
#include "bootloader_random.h"
#endif

#ifdef ESP_IDF_COMP_APP_UPDATE_ENABLED
#include "esp_ota_ops.h"
#endif

#ifdef ESP_IDF_COMP_ESP_HTTPS_OTA_ENABLED
#include "esp_https_ota.h"
#endif

#ifdef ESP_IDF_COMP_ESP_HTTP_CLIENT_ENABLED
#include "esp_http_client.h"
#endif

#ifdef ESP_IDF_COMP_TCP_TRANSPORT_ENABLED
#include "esp_transport.h"
#include "esp_transport_ssl.h"
#include "esp_transport_tcp.h"
#ifdef CONFIG_WS_TRANSPORT
#include "esp_transport_ws.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_HTTP_SERVER_ENABLED
#include "esp_http_server.h"
#endif

#ifdef CONFIG_ESP_HTTPS_SERVER_ENABLE
#include "esp_https_server.h"
#endif

#if defined(ESP_IDF_COMP_ESP_WEBSOCKET_CLIENT_ENABLED) || defined(ESP_IDF_COMP_ESPRESSIF__ESP_WEBSOCKET_CLIENT_ENABLED)
#include "esp_websocket_client.h"
#endif

#if defined(ESP_IDF_COMP_MDNS_ENABLED) || defined(ESP_IDF_COMP_ESPRESSIF__MDNS_ENABLED)
#include "mdns.h"
#endif

#if defined(ESP_IDF_COMP_MQTT_ENABLED) || defined(ESP_IDF_COMP_ESPRESSIF__MQTT_ENABLED)
#include "mqtt_client.h"
#ifdef CONFIG_MQTT_PROTOCOL_5
#include "mqtt5_client.h"
#endif
#endif

#ifdef ESP_IDF_COMP_NVS_FLASH_ENABLED
#include "nvs.h"
#include "nvs_flash.h"
#endif

#ifdef ESP_IDF_COMP_WIFI_PROVISIONING_ENABLED
#include "wifi_provisioning/manager.h"
#include "wifi_provisioning/scheme_ble.h"
#include "wifi_provisioning/scheme_softap.h"
#endif

#ifdef ESP_IDF_COMP_SOC_ENABLED
// TODO: Include all XXX_periph.h headers here
#include "soc/gpio_periph.h"
#include "soc/rtc_periph.h"
#endif

// Since ESP-IDF 5.3 all drivers except TWAI are separate components
#define OLD_DRIVER_COMP ((ESP_IDF_VERSION_MAJOR < 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR < 3) && defined(ESP_IDF_COMP_DRIVER_ENABLED))
#define OLD_DRIVER_COMP_TWAI ((ESP_IDF_VERSION_MAJOR < 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR < 5) && defined(ESP_IDF_COMP_DRIVER_ENABLED))

// ADC
#if ESP_IDF_VERSION_MAJOR < 6 && defined(ESP_IDF_COMP_DRIVER_ENABLED)
#include "driver/adc.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 4 && (defined(ESP_IDF_COMP_ESP_ADC_CAL_ENABLED) || defined(ESP_IDF_COMP_ESP_ADC_ENABLED))
#include "esp_adc/adc_oneshot.h"
#include "esp_adc/adc_continuous.h"
#endif

// TWAI
#if OLD_DRIVER_COMP_TWAI || defined(ESP_IDF_COMP_ESP_DRIVER_TWAI_ENABLED)
#include "driver/twai.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 5 && defined(ESP_IDF_COMP_ESP_DRIVER_TWAI_ENABLED)
#include "esp_twai.h"
#include "esp_twai_types.h"
#include "esp_twai_onchip.h"
#endif

// DAC
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_DAC_ENABLED)
#if defined(CONFIG_IDF_TARGET_ESP32) || defined(CONFIG_IDF_TARGET_ESP32S2)
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/dac.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 1
#include "driver/dac_continuous.h"
#include "driver/dac_cosine.h"
#include "driver/dac_oneshot.h"
#endif
#endif
#endif

// GPIO
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_GPIO_ENABLED)
#include "driver/gpio.h"
// In ESP-IDF v6.0, soc/gpio_periph.h no longer includes soc/gpio_reg.h or
// soc/gpio_sig_map.h, so GPIO_OUT_REG / SIG_GPIO_OUT_IDX and friends are no
// longer transitively visible. Include them explicitly.
#if ESP_IDF_VERSION_MAJOR >= 6
#include "soc/gpio_reg.h"
#include "soc/gpio_sig_map.h"
#endif
#endif

// GPTIMER
#if ESP_IDF_VERSION_MAJOR > 4 && (OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_GPTIMER_ENABLED))
#include "driver/gptimer.h"
#endif

// Old timer
#if defined(ESP_IDF_COMP_DRIVER_ENABLED) && ESP_IDF_VERSION_MAJOR < 6
#include "driver/timer.h"
#endif

// I2C
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_I2C_ENABLED)
#include "driver/i2c.h"
#if ESP_IDF_VERSION_MAJOR > 5 || (ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR > 1)
#include "driver/i2c_types.h"
#include "driver/i2c_master.h"
#include "driver/i2c_slave.h"
#endif
#endif

// LEDC
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_LEDC_ENABLED)
#include "driver/ledc.h"
#endif

// I2S
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_I2S_ENABLED)
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/i2s.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 4
#include "driver/i2s_common.h"
#include "driver/i2s_pdm.h"
#include "driver/i2s_std.h"
#include "driver/i2s_tdm.h"
#include "driver/i2s_types.h"
#endif
#endif

// MCPWM
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_MCPWM_ENABLED)
#if ESP_IDF_VERSION_MAJOR > 4
#define extra_flags mcpwm_drv_extra_flags // Rename to avoid conflict with extra_flags in rmt_rx.h
#include "driver/mcpwm_prelude.h"
#undef extra_flags
#else
#include "driver/mcpwm.h"
#endif
#endif

// PCNT
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_PCNT_ENABLED)
#if defined(CONFIG_IDF_TARGET_ESP32) || defined(CONFIG_IDF_TARGET_ESP32S2) || defined(CONFIG_IDF_TARGET_ESP32S3) || defined(CONFIG_IDF_TARGET_ESP32H2) || defined(CONFIG_IDF_TARGET_ESP32C5) || defined(CONFIG_IDF_TARGET_ESP32C6) || defined(CONFIG_IDF_TARGET_ESP32P4) || defined(CONFIG_IDF_TARGET_ESP32S31)
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/pcnt.h"
#endif
#if ESP_IDF_VERSION_MAJOR >= 5
#include "driver/pulse_cnt.h"
#endif
#endif
#endif

// ???
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/periph_ctrl.h"
#endif

// RMT
#if (OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_RMT_ENABLED))
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/rmt.h"
#endif
#if ESP_IDF_VERSION_MAJOR >= 5
#define rmt_channel_t rmt_drv_channel_t // Rename to avoid conflict with rmt_channel_t in rmt.h
#include "driver/rmt_encoder.h"
#include "driver/rmt_tx.h"
#include "driver/rmt_rx.h"
#undef rmt_channel_t
#endif
#endif

// RTC
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/rtc_cntl.h"
#endif
#include "driver/rtc_io.h"

// SDIO Slave
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_SDIO_ENABLED)
#ifdef SOC_SDIO_SLAVE_SUPPORTED
#include "driver/sdio_slave.h"
#endif
#endif

// SDMMC
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_SDMMC_ENABLED)
#include "driver/sdmmc_defs.h"
#include "driver/sdmmc_host.h"
#include "driver/sdmmc_types.h"
#include "driver/sdspi_host.h"
#endif
#if defined(ESP_IDF_COMP_SDMMC_ENABLED)
#include "sdmmc_cmd.h"
#endif

// Sigma-delta / SDM
#if ESP_IDF_VERSION_MAJOR < 6
#include "driver/sigmadelta.h"
#endif
#ifdef ESP_IDF_COMP_ESP_DRIVER_SDM_ENABLED
#include "driver/sdm.h"
#endif

// SPI
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_SPI_ENABLED)
#include "driver/spi_common.h"
#include "driver/spi_master.h"
#include "driver/spi_slave.h"
#endif

// TOUCH
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_TOUCH_SENS_ENABLED)
#if defined(CONFIG_IDF_TARGET_ESP32) || defined(CONFIG_IDF_TARGET_ESP32S2) || defined(CONFIG_IDF_TARGET_ESP32S3)
#include "driver/touch_pad.h"
#endif
#endif

// UART
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_UART_ENABLED)
#include "driver/uart.h"
#include "driver/uart_select.h"
#if defined(ESP_IDF_COMP_VFS_ENABLED) && (ESP_IDF_VERSION_MAJOR > 5 || (ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 3))
#include "driver/uart_vfs.h"
#endif
#endif

// TEMP SENSOR
#if OLD_DRIVER_COMP || defined(ESP_IDF_COMP_ESP_DRIVER_TSENS_ENABLED)
#if ESP_IDF_VERSION_MAJOR > 4 && defined(SOC_TEMP_SENSOR_SUPPORTED)
#include "driver/temperature_sensor.h"
#endif
#endif

// New drivers (ESP-IDF v6.0+)
#if ESP_IDF_VERSION_MAJOR > 5

#ifdef ESP_IDF_COMP_ESP_DRIVER_CAM_ENABLED
#include "esp_cam_ctlr.h"
#include "esp_cam_ctlr_types.h"
#ifdef SOC_MIPI_CSI_SUPPORTED
#include "esp_cam_ctlr_csi.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_DRIVER_ISP_ENABLED
#ifdef SOC_ISP_SUPPORTED
#include "driver/isp.h"
#include "driver/isp_core.h"
#include "driver/isp_types.h"
#include "driver/isp_af.h"
#include "driver/isp_ae.h"
#include "driver/isp_awb.h"
#include "driver/isp_bf.h"
#include "driver/isp_ccm.h"
#include "driver/isp_gamma.h"
#include "driver/isp_hist.h"
#include "driver/isp_sharpen.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_DRIVER_JPEG_ENABLED
#ifdef SOC_JPEG_CODEC_SUPPORTED
#include "driver/jpeg_types.h"
#ifdef SOC_JPEG_DECODE_SUPPORTED
#include "driver/jpeg_decode.h"
#endif
#ifdef SOC_JPEG_ENCODE_SUPPORTED
#include "driver/jpeg_encode.h"
#endif
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_DRIVER_ANA_CMPR_ENABLED
#include "driver/ana_cmpr.h"
#include "driver/ana_cmpr_types.h"
#endif

#ifdef ESP_IDF_COMP_ESP_DRIVER_PARLIO_ENABLED
#include "driver/parlio_types.h"
#include "driver/parlio_tx.h"
#include "driver/parlio_rx.h"
#endif

#endif // ESP_IDF_VERSION_MAJOR > 5

#ifdef ESP_IDF_COMP_ESPCOREDUMP_ENABLED
#ifdef CONFIG_ESP_COREDUMP_ENABLE
#include "esp_core_dump.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ESP_SERIAL_SLAVE_LINK_ENABLED
#include "esp_serial_slave_link/essl.h"
#include "esp_serial_slave_link/essl_sdio.h"
#endif

#ifdef ESP_IDF_COMP_PTHREAD_ENABLED
#include "pthread.h"
#include "esp_pthread.h"
#endif

#ifdef ESP_IDF_COMP_USB_ENABLED
#ifdef CONFIG_USB_OTG_SUPPORTED
#include "usb/usb_host.h"
#endif
#endif

#ifdef ESP_IDF_COMP_ULP_ENABLED
#if (ESP_IDF_VERSION_MAJOR > 4)
// ESP-IDF V5+
#ifdef CONFIG_ULP_COPROC_ENABLED
#if CONFIG_ULP_COPROC_TYPE_FSM
#if CONFIG_IDF_TARGET_ESP32
#include "esp32/ulp.h"
#elif CONFIG_IDF_TARGET_ESP32S2
#include "esp32s2/ulp.h"
#elif CONFIG_IDF_TARGET_ESP32S3
#include "esp32s3/ulp.h"
#endif
#else
#include "ulp_riscv.h"
#endif
#endif
#else
// ESP-IDF V4.X
#ifdef CONFIG_ESP32_ULP_COPROC_ENABLED
#include "esp32/ulp.h"
#endif
#ifdef CONFIG_ESP32S2_ULP_COPROC_ENABLED
#ifdef CONFIG_ESP32S2_ULP_COPROC_RISCV
#include "esp32s2/ulp_riscv.h"
#else
#include "esp32s2/ulp.h"
#endif
#endif
#ifdef CONFIG_ESP32S3_ULP_COPROC_ENABLED
#ifdef CONFIG_ESP32S3_ULP_COPROC_RISCV
#include "esp32s3/ulp_riscv.h"
#else
#include "esp32s2/ulp.h"
#endif
#endif
#endif
#endif

#if !defined(CONFIG_IDF_TARGET_ESP32S2) && !defined(CONFIG_IDF_TARGET_ESP32P4) // No BT in ESP32-S2 & ESP32-P4

// If a custom sdkconfig file has been used to enable Bluetooth support,
// since by default neither of the BT stacks is enabled.
#ifdef CONFIG_BT_ENABLED
#include "esp_bt.h"

// Bluedroid APIs (Classic BT & BLE)
#ifdef CONFIG_BT_BLUEDROID_ENABLED
// Generic
#include "esp_bt_defs.h"
#include "esp_bt_device.h"
#include "esp_bt_main.h"

// Classic BT
#ifdef CONFIG_IDF_TARGET_ESP32 // Only the original ESP32 MCU supports Classic BT
#ifdef CONFIG_BT_CLASSIC_ENABLED
#ifdef CONFIG_BT_A2DP_ENABLE
#include "esp_a2dp_api.h"
#include "esp_avrc_api.h"
#endif
#include "esp_gap_bt_api.h"
#ifdef CONFIG_BT_HFP_ENABLE
#include "esp_hf_ag_api.h"
#include "esp_hf_client_api.h"
#endif
#ifdef CONFIG_BT_HID_ENABLED
#include "esp_hidd_api.h"
#endif
#include "esp_hidh_api.h"
#if ESP_IDF_VERSION_MAJOR > 4
#include "esp_sdp_api.h"
#endif
#ifdef CONFIG_BT_SPP_ENABLED
#include "esp_spp_api.h"
#endif
#endif // CONFIG_BT_CLASSIC_ENABLED
#endif // CONFIG_IDF_TARGET_ESP32

// BLE
#ifdef CONFIG_BT_BLE_ENABLED
#include "esp_gap_ble_api.h"
#include "esp_gatt_defs.h"
#include "esp_gatt_common_api.h"
#ifdef CONFIG_BT_GATTC_ENABLE
#include "esp_gattc_api.h"
#endif
#ifdef CONFIG_BT_GATTS_ENABLE
#include "esp_gatts_api.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 4
#ifdef CONFIG_BT_L2CAP_ENABLED
#include "esp_l2cap_bt_api.h"
#endif
#endif
#endif // CONFIG_BT_BLE_ENABLED
#endif // CONFIG_BT_BLUEDROID_ENABLED

// Nimble APIs (BLE only)
#ifdef CONFIG_BT_NIMBLE_ENABLED
#if defined(CONFIG_IDF_TARGET_ESP32) || defined(CONFIG_IDF_TARGET_ESP32C3) || defined(CONFIG_IDF_TARGET_ESP32S3)
#include "esp_nimble_hci.h"
#endif
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "host/ble_hs.h"
#include "host/util/util.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"
#endif // CONFIG_BT_NIMBLE_ENABLED

// BLE Mesh
#ifdef CONFIG_BLE_MESH
#include "esp_ble_mesh_defs.h"
#include "esp_ble_mesh_ble_api.h"
#include "esp_ble_mesh_common_api.h"
#include "esp_ble_mesh_local_data_operation_api.h"
#include "esp_ble_mesh_low_power_api.h"
#include "esp_ble_mesh_networking_api.h"
#include "esp_ble_mesh_provisioning_api.h"
#include "esp_ble_mesh_proxy_api.h"
#endif // CONFIG_BLE_MESH

#endif // CONFIG_BT_ENABLED

#endif // CONFIG_IDF_TARGET_ESP32S2

// LDO is used by MIPI DSI PHY
#ifdef SOC_GP_LDO_SUPPORTED
#include "esp_ldo_regulator.h"
#endif

// LCD support
#ifdef ESP_IDF_COMP_ESP_LCD_ENABLED
#include "esp_lcd_panel_commands.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_panel_vendor.h"
#include "esp_lcd_types.h"
#include "esp_lcd_panel_interface.h"
#include "esp_lcd_panel_io_interface.h"
#if (ESP_IDF_VERSION_MAJOR == 4 && ESP_IDF_VERSION_MINOR >= 4) || (ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR <= 2) || defined(SOC_LCD_RGB_SUPPORTED)
#include "esp_lcd_panel_rgb.h"
#endif
#if ESP_IDF_VERSION_MAJOR > 5 || ESP_IDF_VERSION_MAJOR == 5 && ESP_IDF_VERSION_MINOR >= 3
#include "esp_lcd_panel_dev.h"
#if ESP_IDF_VERSION_MAJOR < 6
#include "esp_lcd_panel_nt35510.h"
#endif
#include "esp_lcd_panel_ssd1306.h"
#include "esp_lcd_panel_st7789.h"
#ifdef SOC_MIPI_DSI_SUPPORTED
#include "esp_lcd_mipi_dsi.h"
#endif
#endif
#endif // ESP_IDF_COMP_LCD_ENABLED

// Usb serial support
#ifdef SOC_USB_SERIAL_JTAG_SUPPORTED
#include "driver/usb_serial_jtag.h"
#endif

// Official TinyUSB remote component
#ifdef ESP_IDF_COMP_ESPRESSIF__ESP_TINYUSB_ENABLED
#include "tinyusb.h"
#include "tinyusb_net.h"
#include "tinyusb_types.h"
#if CONFIG_TINYUSB_CDC_ENABLED
#include "tusb_cdc_acm.h"
#endif
#include "tusb_config.h"
#include "tusb_console.h"
#include "tusb_tasks.h"
#ifdef ESP_IDF_COMP_VFS_ENABLED
#include "tusb_msc_storage.h"
#include "vfs_tinyusb.h"
#endif
#endif

// Official onewire_bus remote component
#ifdef ESP_IDF_COMP_ESPRESSIF__ONEWIRE_BUS_ENABLED
#include "onewire_bus.h"
#include "onewire_device.h"
#endif
