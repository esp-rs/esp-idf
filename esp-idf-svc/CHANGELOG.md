# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Netif: add an asynchronous, modem-agnostic bridge between split `embedded-io-async` byte streams and `EspNetifDriver`.

### Fixed
- Netif: align the default PPP client with ESP-IDF's PPP configuration, correctly deserialize `IP_EVENT_PPP_LOST_IP`, and keep custom driver lifecycle state in sync across start/stop calls.

## [0.53.0] - 2026-09-25

### Breaking
- MQTT: `MqttProtocolVersion::V5` variant and [`Mqtt5ConnectionPropertyConfig`] struct exposing MQTT 5.0 CONNECT properties (session/will/message expiry intervals, receive/packet/topic-alias maxima, request-response/problem info, payload format indicator). Gated on `CONFIG_MQTT_PROTOCOL_5=y`. To negotiate MQTT 5 and set properties:
  ```rust
  MqttClientConfiguration {
      protocol_version: Some(MqttProtocolVersion::V5),
      mqtt5_connection_property: Some(Mqtt5ConnectionPropertyConfig {
          session_expiry_interval: Some(60),
          ..Default::default()
      }),
      ..Default::default()
  }
  ```
- HTTP: Add `keep_alive: Option<KeepAlive>` and `so_linger: Option<Duration>` to server `Configuration`
- TLS: Add `tls_handshake_timeout_ms: u32` to server `ServerConfig` on ESP-IDF >= 5.5.0 (`0` keeps the ESP-TLS default of 10 seconds; only honored by the blocking `EspTls::negotiate_server`). Struct literals on those IDF versions must set the new field (or use `ServerConfig::default()` / `..Default::default()`).
- New events need to be handled in the WiFi event loop:
  - `WifiEvent::StaNeighborRep` / `StaNeighborRepRef` (v5.3.0+)
  - `WifiEvent::ApWrongPassword` / `ApWrongPasswordRef` (v5.3.3+, v5.4.1+, v5.5.0+)
  - `WifiEvent::StaBeaconOffsetUnstable` / `StaBeaconOffsetUnstableRef` (v5.5.0+)
  - `WifiEvent::DppUriReady` / `DppUriReadyRef` (v5.5.0+)
  - `WifiEvent::DppCfgRecvd` / `DppCfgRecvdRef` (v5.5.0+)
  - `WifiEvent::DppFailed` / `DppFailedRef` (v5.5.0+)
- WiFi: `DppCfgRecvdRef` (v5.5.0+) no longer implements `Copy`/`Clone`. Recent ESP-IDF added a flexible-array-member field (`configs[]`) to `wifi_event_dpp_config_received_t`, which bindgen surfaces as `__IncompleteArrayField` and which cannot be copied by value. The payload is only ever observed as `&DppCfgRecvdRef` inside the `WifiEvent::DppCfgRecvd` callback, so borrowing it is unaffected; only code that copied or cloned it out of the callback needs to change.
- Unknown event IDs on the ESP-IDF event bus now produce an `Other(i32)` variant and a `log::warn!` instead of panicking, for `WifiEvent`, `IpEvent`, `EthEvent`, `PppEvent`, and `ThreadEvent`, improving forward compatibility with future ESP-IDF releases
- Note that in ESP-IDF V6.0, some drivers have been moved to external components (`mqtt`, ethernet PHY/SPI drivers). If the code fails to build, you may need to enable extra components in your `Cargo.toml`, e.g.:
```toml
[[package.metadata.esp-idf-sys.extra_components]]
remote_component = { name = "espressif/lan87xx", version = "1.*" }
```

### Fixed
- OpenThread: SRP slots increased from 3 to 6; extra logging when SRP registration fails
- BT/BLE: Advertising was not working with ESP-IDF 5.5+ and esp32, esp32c3 and esp32s3
- BT/BLE: Extended advertising exvets were mis-mapped as `BleGapEvent::Other`
- WiFi: receiving any of the six new events listed above on ESP-IDF v5.3+ / v5.5+ no longer causes a panic (fixes #618)
- WebSocket: `EspWebSocketClient::drop()` no longer panics when `esp_websocket_client_close` or `esp_websocket_client_destroy` returns an error (e.g. after a network disconnection); errors are now logged instead of unwrapped
- WebSocket: `EspWebSocketClient::new`, `new_with_conn` and `new_nonstatic` no longer leave a partially initialized client behind when `esp_websocket_register_events` or `esp_websocket_client_start` fails (e.g. when the transport cannot be created, or the client task cannot be spawned). Previously the client struct was built before those calls, so the `?` on a failure dropped a client that was never started, and its `Drop` impl invoked `esp_websocket_client_close` on it - which always reports `ESP_FAIL` for a non-running client. The raw handle is now destroyed directly on the failing path and no client is constructed at all
- MQTT: MQTT 5.0 CONNECT properties can now be set on `EspMqttClient` without deadlocking on `MQTT_API_LOCK`; they are applied inside the library between `esp_mqtt_client_init` and `esp_mqtt_client_start` via the new [`MqttClientConfiguration::mqtt5_connection_property`] field.
- BT: Fixed panic when an A2DP sink disconnects from the ESP while streaming audio.
- Thread: `scan`, `energy_scan` and the IPv6 receive callbacks no longer pass the wrong context pointer to OpenThread (the closure box instead of the `ThreadDriverInner`), fixing a type-confusion crash when the callbacks fire.
- Ethernet: `mod eth` is enabled again on ESP-IDF 6.0+ when SPI Ethernet PHYs are provided as managed components (`espressif/w5500`, `espressif/dm9051`, `espressif/ksz8851snl`), not only via the removed in-tree `CONFIG_ETH_SPI_ETHERNET_*` Kconfig options
- TLS: Server-side APIs (`ServerConfig`, `negotiate_server`, …) are available again on ESP-IDF >= 5.3 with mbedTLS. `CONFIG_ESP_TLS_SERVER` was removed in v5.3, so the old `esp_idf_esp_tls_server` cfg was never set and those APIs were compiled out; gating now uses `esp_idf_esp_tls_server` (IDF ≤ 5.2) or `esp_idf_version_at_least_5_3_0` + `esp_idf_esp_tls_using_mbedtls`.

### Added
- Compatibility with ESP-IDF V6.0, and some pre-release 6.0.x.
- Added support for the Generic Ethernet PHY driver: particularly useful on ESP-IDF 6.0+ as it is built-in.
- Added type-safe wrappers for the NimBLE low-resource-use BLE stack: GAP, GATT Server, GATT Client, L2CAP. See `examples/ble_*.rs`
- TLS: Async server handshake (requires ESP-IDF 5.5.0): `EspAsyncTls::negotiate_server`. Also check the new `tls_server_async` example.
- Added support for Bluetooth A2DP External Codec API in esp-idf v6.1 (w/ AAC codec negotiation)

## [0.52.1] - 2026-03-10

### Fixed
- Fix several compilation issues with esp32p4 and esp32h2

## [0.52.0] - 2026-03-09

### Breaking
- Added custom server certificate to `http::client::Configuration`
- BT: `BtDriver::set_device_name` deprecated and not available with ESP-IDF 6.0+. Use the new `EspGap::set_device_name` instead
  or the existing `EspBleGap::set_device_name`
- BT: `BleGapEvent::ScanResult` is now a struct instead of a wrapper around esp_ble_gap_cb_param_t_ble_scan_result_evt_param  
- Implement MQTT outbox limit and get_outbox_size()
- Added argument `subprotocol_list` to `ws_handler` to allow subprotocols to be supported by WebSockets
- Thread enhancements (#592). Specifically:
 - Thread SRP (Thread-specific mDNS) is now supported and has a new API so that the user can register/unregister SRP services
 - Option to start/stop the Thread stack (methods `Thread::start` / `Thread::stop` in place of the previous `Thread::run`)
 - Several callbacks where actually unsound, as they were not `Send + 'static`. Now fixed
 - Simplifications:
  - `Thread::init` and `Thread::deinit` are now gone
  - No option to swap the Thread Netif with a custom one, as it complicates the implementation, and I don't see the use-case (unlike with Wifi)
 - Compatibility with ESP-IDF 5.1.6+, 5.2.4+, 5.3.2+ and 5.4+ in that the new DatasetChanged event is properly handled and does not cause the event code to panic
- MSRV raised to 1.82
- NVS: Removed `NvsDataType::Any` and replaced `From<nvs_type_t>` with `NvsDataType:from_nvs_type`
- (#529) `Peripheral` and `PeripheralRef` removed and replaced with a simple pattern similar to the `esp-hal` one.
  - Check https://github.com/esp-rs/esp-idf-hal/pull/529 for details on that change
- HTTP: added `task_caps` option into server `Configuration`
- Update `heapless` dependency

### Fixed
- `WifiDriver::get_ap_info` not takes `&self` instead of `&mut self`. Convenience method `EspWifi::get_ap_info` that delegates
  to `WifiDriver::get_ap_info`
- BT: Fix BLE not working on the s3 and with ESP-IDF 5.3+
- BT: Fix `bt_gatt_server` example not allowing reconnect after client connect (#553) and handle sububscribe/unsubscribe to indications
- BT: Fix `EspBleGap::set_security_conf` not setting auth_req_mode, and returning ESP_ERR_INVALID_ARG when setting key sizes
- Fix wrong conversion from `ScanType` to `u32` in Wi-Fi configuration
- Fix wrong BT configuration version on the c6 (issue #556)
- Fix inconsistent mutability in NVS (#567)
- Fix #570 (c_char vs i8 mismatch on newer rustc toolchains)
- ESP-IDF partitions support is no longer behind the `experimental` feature
- Filesystems support is no longer behind the `experimental` feature
- Bluetooth support is no longer behind the `experimental` feature
- Thread support is no longer behind the `experimental` feature
- MQTT: Fix a crash when the LWT payload is empty (#597)
- WS: Fix `EspWebSocketClientConfig` `reconnect_timeout_ms` and `network_timeout_ms` fields not being passed to the underlying C struct (#635)

### Added
- Compatibility with ESP-IDF V5.4.x and V5.5.x
- Logging configuration enhanced with a simpler setup where Rust logs can be configured to have a verbosity which is
  disconnected from the verbosity of the ESP-IDF native C logging (#593)
- OTA: New method - `EspFirmwareInfoLoad::fetch_native` - returning the full native ESP-IDF image descriptor structures
- Added `use_serde` feature, which enables the `use_serde` feature of `embedded-svc` crate, allowing to deserialize configuration structs.
- OTA: Allow specifying image size to speed up erase
- Bluetooth: New methods `EspBleGap::set_scan_params`, `EspBleGap::start_scanning`, `EspBleGap::stop_scanning`,
  `EspBleGap::resolve_adv_data_by_type`, `EspBleGap::disconnect` and `gatt::set_local_mtu`
- Bluetooth: New BLE Gatt Client `EspGattc`
- Bluetooth Classic: Added Serial Port Profile, `spp`
- New example, `bt_spp_acceptor` to demonstrate usage of bt classic spp profile
- New example, `bt_ble_gap_scanner` to demonstrate usage of added ble scanning methods
- New example, `bt_gatt_client` to demonstrate usage of added ble gatt client
- New example, `mdns_advertise` to demonstrate mDNS service advertisement
- NVS: Implemented `RawHandle` for `EspNvs<NvsDefault>`
- NVS: Added `EspNvs::erase_all` to remove all data stored in an nvs namespace
- NVS: Added `EspNvs::keys` to iterate over all stored keys
- esp32p4: This chip has no radio, but can use wifi via esp_wifi_remote. Support for esp_wifi_remote has been added.
- HTTP: added `keep_alive_*` option into configuration

## [0.51.0] - 2025-01-15

### Breaking
- New `embassy-time-driver` implementation compatible with `embassy-time-driver` V0.2 (#548)

### Added
- New example, `wifi_dhcp_with_hostname` to demonstrate setting a custom hostname when establishing a DHCP connection

## [0.50.1] - 2025-01-06
### Fixed
- Fix ambiguous name error (a compilation issue when the NimBLE component is enabled in esp-idf-sys)

## [0.50.0] - 2025-01-02

### Deprecated
- `EspFirmwareInfoLoader` use `EspFirmwareInfoLoad` instead

### Breaking
- Wifi event details (#455)
- Add poll_read/write and implement futures_io::AsyncRead/Write for EspAsyncTls (#488)
- Support for LittleFS (#498)
- Change default eth key (#502)
- ESP IDF Partitions API (#511)
- Expose src_addr and dst_addr in espnow recv cb (#525)
- Add `core` field to `http::server::Configuration` to control which CPU core runs the HTTP server task

### Added
- Compatibility with ESP-IDF v5.3.X
- feat(eth): Implement alternative polling mode (#452)
- SD Card driver; SD Card host drivers (SPI and SDMMC) (#454)
- Make EspAsyncMqttClient::wrap public. (#462)
- Netif-driver support
- Netif PPP (#473)
- Added http_local_network_server example (#471)
- Added esp_wifi_sta_get_rssi function in EspWifi (#478)
- Expose esp_mqtt_client_set_uri. Fix issue #481. (#482)
- Option to explicitly initialize the netif stack (NetifStack::initialize)
- Support for Thread (#484)
- Enable usage of `esp_idf_log_timestamp_rtos` for ms since boot and `esp_idf_log_timestamp_source_system` for system time in rust logging (#494)
- Document OTA API (#500)
- Add option to specify initial caps for the MQTT async adaptor vectors
- Allow using esp timer with skip_unhandled_events (#526)
- OTA - Implements a new type `EspFirmwareInfoLoad` that has a reduced memory consumption (#531)
- Added set_promiscuous function in EthDriver (#246)
- Added set_promiscuous, is_promiscuous functions in WifiDriver (#246)
- Blocking and buffered StdIo (#541) - i.e. easy reading/input from `std::io::stdin`
- Added source_ipv4, source_ipv6 function in EspHttpRawConnection (#538)

### Fixed
- The alloc::Vec version stomps the scan state from Done to Idle. (#459)
- Logging - Fix set_target_level (#458)
- Make async mqtt client implement Send. (#461)
- Implement Sync for EspMqttEvent. (#463)
- Avoid potential memory leak when dropping mqtt clients (#464)
- Filter asynchronous events (#466)
- Remove unnecessary buffer draining in HTTP client example (#470)
- Fixed: EspLogger is not extensible
- Fix incorrect key variable being logged for the struct storage for raw NVS access example (#479)
- `eth_esp32_emac_default_config` - 5.3 compatibility
- Fix "esp32c2 use example http_sw_server report err ESP_ERR_HTTPD_TASK"
- Added IP_EVENT_ETH_LOST_IP to deserialize list (#491)
- examples/http_ws_server.rs: fix string decoding (#510)
- Bugfix: crash on MQTT async client restart
- Fix missing newline if CONFIG_LOG_COLORS=n is set (#521)
- gatekeep mdns ipv6 behind feature flag (#523)
- Fix emac_rx stack overflow when log verbosity is increased (#535)

## [0.49.1] - 2024-07-09
### Fixed
* Bluetooth: The experimental Bluedroid support did not compile on esp32c2, esp32h2 and esp32c6 (#447)

## [0.49.0] - 2024-06-23

### Deprecated
**ESP-IDF v4.4** Please start upgrading to ESP-IDF v5.
### Breaking
* **removed** ESP-IDF v4.3 support, including mostly conditional compilations. (#431)
* wifi: now can use embedded-svc PmfConfiguration, ScanMethod, and ScanSortMethod in ClientConfiguration. (#381)
* wifi: The WifiEvent's ApStaConnected and ApStaDisconnected were changed to include the idf's wifi_event. (#396)
* eth: callbacks now use newly added EthFrames instead of &[u8]. (#406)
* wifi: callbacks now use newly added WifiFrames instead of &[u8]. (#406)
* http_server: Configuration now allows for setting the ctrl_port. (#427)
* http_server: UB fix: `handler`, `fn_handler` and `handler_chain` all now only accept `'static` callbacks,
  which is the only safe option in the presence of `core::mem::forget` on the HTTP server. All of those have the
  previous behavior preserved in the form of `unsafe` `*_nonstatic` variants which were added. (#437)
* tls: negotiate now returns the new CompletedHandshake struct instead of (). (#428)
* wifi: Remove AUTOUP as the default flag on ClientConfiguration:Fixed. (#426)
* tls: Allow TLS negotiation on a multi-threaded executor. (#432)
* MSRV: 1.77 (due to `core::net` which is re-exported by `embedded-svc` and is stable since Rust 1.77)
### Added
* tls: Support for TLS server. (#368)
* ws: expose crt_bundle_attach to EspWebSocketClientConfig. (#391)
* ping: can now be used with disabled IPv6. (#418)
* wifi: EspWifi's wrap_all method now supports only wrapping sta if softAp is disabled. (#376)
* sd: **New SD mmc & spi drivers**. Check out the sd_mmc and sd_spi examples. (#422)
* fs: new wrapper implementation around fat. (#422)
* tls: Make EspTls and EspAsyncTls Send when possible. (#429)
* ble/gatt: **New BLE GATT server support using Bluedroid.** Check out the bt_gatt_server example. (#421)
### Fixed
* nvs: encrypted partition could not find partition by name. (#387)
* ota: handle partition errors gracefully. (#393)
* http_client: flush responses to avoid repeated request failures. (#404)
* eth: missing error return inside the rx_callback function. (#414)
* wifi: AccessPointConfiguration now correctly limits max_connections. (#426)
* wifi: Fix WPS regression around null termination of ssid/password. (#379)
* Compatibility with ESP-IDF v5.3 (pre-release): various fixes such that esp-idf-svc can be used against the latest esp-idf versions. (#434)

## [0.48.1] - 2024-02-21
* Disable the `esp_idf_svc::io::vfs` module if the ESP IDF VFS component is not enabled either
* Bugfix / async MQTT: The internal `Unblocker` utility was missing `drop` and therefore did not delete its task properly, resulting in a crash when the async MQTT client is dropped
* #357 - `AsyncWifi` was not `Send` anymore (regression). `Send` restored.
* #356 - Change payload of `EspEventPostData` from `&[u32]` to `&[u8]`
* #357 - Restore `Send` for `AsyncWifi`
* #369 - (Crash) Restore the drop impl EspMqttClient
* #370 - (Deadlock) Fix a deadlock when dropping an MQTT client in the presence of a blocking EspMqttConnection
* Fix clippy duplicate imports warnings with latest 1.78 nightly

## [0.48.0] - 2024-01-26
* New examples: 
  * MQTT client (blocking and async)
  * TLS (async; blocking already exists)
  * Event loop (blocking and async)
  * Timers (blocking and async)
  * SPI Ethernet (async; a blocking example for RMII Ethernet already exists)
  * TCP client and server (blocking and async)
  * SNTP service
  * Websocket client (blocking)
* Breaking changes in module `eventloop`: 
  * Async send and receive functionality now implemented directly on the `esp-idf-svc` event loop types, as the `embedded_svc::utils::asyncify` module is now gone
  * Types `EspTypedEventLoop` and `EspPostbox` are now retired. Use `EspEventLoop` directly, as it has the same functionality
  * `EspEventFetchData` renamed to `EspEvent`; both `EspEvent` and `EspEventPostData` now lifetimed instead of using raw pointers
  * Trait `EspTypedEventSource` is renamed to `EspEventSource` and marked as unsafe (i.e., implementors should do `unsafe impl EspTypedEventSource for ...`); check the documentation of the trait for justification
  * Types `EspTypedEventDeserializer` and `EspTypedEventSerializer` renamed to just `EspEventSerializer` and `EspEventDeserializer`; more importantly, their payload is now modeled using a lifetimed GAT member called `Data<'a>`; this allows deserializers to implement zerocopy deserialization by referencing the event payload which is owned by the event loop; all `esp-idf-svc` deserializers (notably - `WifiEvent` and `IpEvent`) are now implemented with zerocopy, thus reducing the pressure on the system event loop task stack size
  * The `EspEvent` type is now also a dummy (no op) `EspEventDeserializer`; the `EspEventPostData` type is now also a dummy (no op) `EspEventSerializer`
  * Because of the above changes, methods `subscribe*` and `post*` are now slightly less convenient to use in that they need the (de)serializer specified using turbofish syntax, i.e. `event_loop.subscribe::<WifiEvent, _>(...)`; this is so because these methods no longer require (and cannot require - due to the lifetimed GAT from above) `where P: EspEvent(De)serializer<P>`, i.e. the event type *itself* to implement the (de)sderializer, even if all event types provided by `esp-idf-svc` do that
  * The `post*` and `spin` methods now take a timeout of type `TickType_t` as everywhere rather than the complex `Option<Duration>`
* Breaking changes in module `http::server`: 
  * Due to the breaking change in `embedded_svc::http::server`, whereas `HandlerError` and `HandlerResult` were removed, these types are no longer used in the `embedded_svc::http::server` module either. Check the Changelog of `embedded_svc` for more details
* Breaking change in module `timer`: all async timer functionality now implemented directly on the `esp-idf-svc` timer types, as the `embedded_svc::utils::asyncify` module is now gone
* Breaking changes in module `mqtt::client`: 
  * All async send/receive functionality now implemented directly on the `esp-idf-svc` MQTT types, as the `embedded_svc::utils::asyncify` module is now gone
  * Changes induced by breaking changes in `embedded_svc::mqtt::client` API contract:
    * All event conversion logic now retired, significantly simplifying the type signatures of `EspMqttClient` and `EspMqttConnection`, as well as the number of offered constructors
    * For MQTT events, user always gets an instance of `EspMqttEvent` which implements the `embedded_svc::mqtt::client::Event` trait - valid for both callback-based event processing as well as for connection-based blocking and asynchronous event processing
* Breaking change: `AsyncEspTls` renamed to `EspAsyncTls`
* MSRV 1.75; remove the nightly feature flag from all async trait implementations
* Update public dependency `heapless` to 0.8
* Remove dependency on `embassy-time` and replace it with a dependency on `embassy-time-driver`; get rid of the custom embassy time queue as it was anyway re-implementing something like a generic timer queue, which is available in the `embassy-time` crate (with its feature `generic-queue` enabled)
* #316 - breaking change addressing a typo - `http::server::Configuration::max_resp_handlers` renamed to `http::server::Configuration::max_resp_headers`
* #319 - Set default TTL in `EspPing` to 64
* #322 - Fix MQTT PSK code (did not compile)
* #323 - ETH example with a statically configured IP
* #324 - New methods in `EspWifi` to swap STA and AP netifs separately from each other
* #326 - logging from multiple threads in Rust/ESP-IDF no longer results in intermixed logs
* #331 - Add support for WPS

## [0.47.3] - 2023-11-12
* BREAKING CHANGE IN A PATCH RELEASE DUE TO DISCOVERED UB: All constructors and methods in the Classic BT services, as well as in
services `EspNow`, driver `EthDriver`, `EventLoop`, `Wait`, `EspHttpServer`, `EspMqttClient`, `EspSntp`, `EspTimerService`,
driver `WifiDriver` and `EspWebSocketClient` no longer accept non-static callbacks, as these lead to UB / crash when the service/driver/subscription 
is forgotten with e.g. `core::mem::forget`. Since local borrows are a very useful feature however, these are still allowed via the newly-introduced 
and `unsafe` methods/constructors that follow the naming pattern of the safe methods/constructors, but with a `_nonstatic` suffix attached.

## [0.47.2] - 2023-11-02
* Remove dependency on `AtomicU64` which is no longer supported by the upstream `*-espidf` targets
* HTTP client: support for chunked encoding of POST requests

## [0.47.1] - 2023-10-18
* Compatibility with `embedded-svc` 0.26.1

## [0.47.0] - 2023-10-17
* MSRV raised to 1.71
* New `experimental` module - `bt` - providing Bluetooth support based on the ESP-IDF Bluedroid implementation
  * Only classic BT supported for now (on the ESP32) with the following profiles: A2DP sink, AVRC controller, HFP client, GAP
  * BLE support in the works, but not buildable yet
* TLS over TCP/IP support in the `tls` module via `EspTls` and (for async mode) `AsyncEspTls`
* PSK support for `mqtt`
* Dependencies `esp-idf-sys` and `esp-idf-hal` are now re-exported as `esp_idf_svc::sys` and `esp_idf_svc::hal`
* Upgraded to `embedded-svc` 0.26
* OTA: New method: `EspOta::finish` that allows to postpone/avoid setting the updated partition as a boot one
* Breaking change: OTA: `EspOtaUpdate` now parametric over the lifetime of a mutable reference to `EspOta` and returned by value
* Breaking change: OTA: `EspOtaUpdate::abort` and `EspOtaUpdate::complete` now take `self` instead of `&mut self`
* Breaking change: HTTP server: Scoped handlers; handlers now need to live only as long as the `EspHttpServer` instance. Therefore, `EspHttpServer` is now lifetimed: `EspHttpServer<'a>`
* Breaking change: HTTP server: `EspHttpRequest` renamed to `EspHttpRawConnection`
* Breaking change: WS client: Scoped handler; the event handler now needs to live only as long as the `EspWebSocketClient` instance. Therefore, `EspWebSocketClient` is now lifetimed: `EspWebSocketClient<'a>` 
* Breaking change: EspTimerService: scoped handler: the timer callback now only needs to live as long as the returned `EspTimer` instance. Therefore, `EspTimer` is now lifetimed: `EspTimer<'a>`
* Breaking change: EspEventLoop: scoped handler: the subscription callback now only needs to live as long as the returned `EspSubscription` instance. Therefore, `EspSubscription` is now lifetimed: `EspSubscription<'a, ...>`
* Breaking change: MQTT client: Scoped handler; the event handler now needs to live only as long as the `EspMqttClient` instance. Therefore, `EspMqttClient` is now lifetimed: `EspMqttClient<'a, ...>` 
* Breaking change: EspNow client: Scoped handlers; the event handlers now need to live only as long as the `EspNow` instance. Therefore, `EspNow` is now lifetimed: `EspNow<'a>` 
* Breaking change: Sntp client: Scoped handler; the event handler now needs to live only as long as the `EspSntp` instance. Therefore, `EspSntp` is now lifetimed: `EspSntp<'a>` 
* Breaking change (bugfix): Ping client: `EspPing::ping_details` now takes a `FnMut` callback, however the callback needs to be `Send`
* Breaking change: Removed the deprecated module `httpd` and the dependency on `anyhow`
* Breaking change: module `notify` removed as it was rarely - if ever - used, and there is a simpler `hal::task::notification` module now
* Deprecated: Using ESP-IDF 4.3 is now deprecated and all special cfg flags will be removed in the next release

## [0.46.2] - 2023-07-30

* EspMdns::query crashes when no services are found #279
* OTA: get_running_slot was wrongly returning the boot slot

## [0.46.1] - 2023-07-30

* Workaround issue 11921 in ESP IDF (new member of struct `wifi_scan_config_t`)
* Make all conversions to CString fallible rather than panic-ing
* Bugfixes in HTTPD WS support: Allow calls with a zero-length buffer
* Added docstrings for wifi module (#262)

## [0.46.0] - 2023-05-13

* MSRV 1.66
* Support for ESP IDF 5.0, 5.1 and 5.2 (master)
* Remove the `experimental` status from all formerly experimental features
* Remove the `nightly` feature flag guard from all `asyncify` modules as Rust GATs are stable now
* Async and blocking APIs for `Wifi`, `Eth` and `EspNetif` that abstract away the ESP IDF system event loop (for easier initial configuration) - API breakage in `netif`
* `Eth` SPI driver rebased on top of `esp-idf-hal`'s `SpiDeviceDriver`; it can now either borrow or own the SPI device driver (API breakage)
* `Eth` driver now supports SPI bus sharing with other SPI devices (API breakage)
* `NVS` - additional APIs that support the serde format of the ESP IDF NVS C code
* `SNTP` - new, callback API
* `log` - support for setting target level per module
* `OTA` - small API extensions
* `netif` - compilation errors when PPP & SLIP support is enabled are addressed
* HTTP client & server - various bugfixes
* `EspError::from_infallible`

## [0.45.0] - 2022-12-13

HTTP server:
* Compatibility with `embedded-svc` V0.24
* New function - `fn_handler` that addresses HRTB lifetime issues when converting a Fn closure to a `Handler` instance
* Remove `EspHttpFnTraversableChain`; it is not necessary, now that the `fn_handler` function from above does exist

## [0.44.0] - 2022-12-09

Rebase on top of `esp-idf-sys` 0.32:
* Retire any usages of `esp-idf-sys::c_types` in favor of `core::ffi`
* Remove the `cstr_core` dependency as `Cstr` and `CString` are now part of Rust core
* Remove casts from `usize` to `u32` and back now that `esp-idf-sys` is compiled with `--size_t-is-usize` enabled

## [0.43.3, 0.43.4, 0.43.5] - 2022-12-08

Patch releases:
* Eth driver:
  * SPI drivers now work with ESP IDF 5+
  * DMA channel is now configurable
* Clippy fixes

## [0.43.1, 0.43.2] - 2022-11-21

Patch releases to fix compilation errors under no_std.

## [0.43.0] - 2022-11-01

Release 0.43 is a backwards-incompatible release where almost all services were touched in one way or another.

### Major Changes

The main themes of the 0.43 release are:
* Public API
* Separate the notions of using a "nightly" compiler (which is a precondition for all async support) from "experimental" features, which might or might not be async-related
* Expose access to the wrapped ESP IDF services / drivers
* Wifi & Eth: separate layer 2 (ethernet) from layer 3 (IP)
* Http client & server: implement the new traits from `embedded-svc`
* Merge the `nvs_storage` module into `nvs`
* Support for the `embassy-time` crate by providing alarm implementation
* Support for the `embassy-time` crate by providing timer queue implementation

### Major changes elaboration

### Public API

In addition to implementing the `embedded-svc` traits where possible, all services now have public API. While the public API loosely follows the APIs from `embedded-svc`, it deviates where appropriate so that the native underlying ESP IDF service is better utilized.

These public APIs mean that the user is no longer required to depend on the `embedded-svc` crate so as to consume the `esp-idf-svc` services.
Consuming the services via the `embedded-svc` traits is now only necessary when the user is targetting cross-platform portability of their application code.

### Expose access to the wrapped ESP IDF services / drivers

All services now implement the `Handle` trait which does provide a reference to the native underlying ESP IDF service or driver. This is useful when the Rust wrapper for the service does not expose a particular functionality, as in that case, users can still call the functionality by using the raw `esp-idf-svc` bindings for the service.

### Wifi & Eth: separate layer 2 (Ethernet) from layer 3 (IP)

The Wifi and Ethernet drivers are now split into two separate structures:
* `WifiDriver` / `EthDriver` - these are layer 2 (Ethernet) drivers which follow all the conventions of the other drivers in the `esp-idf-hal` crate, including the need to own or mutably borrow the actual petihperal (the Wifi modem peripheral or the RMII / SPI peripheral from `esp-idf-hal`). They are however part of `esp-idf-svc` as this is better aligned with their native ESP IDF counterparts, which actually do use/rely on certain ESP IDF services, like the event loop and NVS storage, which - being services - are also exposed as part of `esp-idf-svc`, and not as part of `esp-idf-hal`. These drivers implement the `Wifi` and `Eth` traits respectively, which were redesigned to not have any IP-related semantics. Users are allowed to use the Layer 2 drivers directly by providing their own custom RX callbacks, thus completely bypassing the ESP IDF LwIP IP & Netif layer (i.e. with `smoltcp` or other 3rd party IP stack)
* `EspWifi` / `EspEth` - these are layer 3 (IP) + layer 2 (Ethernet) services, which - on construction - are expected to own a `WifiDriver` / `EthDriver` - either by constructing it themselves, or by the user passing the driver. These services "glue" the ESP IDF IP & Netif layer (`EspNetif`) with the ethernet layer provided by the drivers. These services *also* implement the `Wifi` / `Eth` traits, as they are wrapping Layer 2 functionality anyway. The Layer 3 functionality (configuring the network interface as well as fetching IP-related information from the network interfaces) however uses custom methods on the services' themselves and is not (yet) abstracted using `embedded-svc` traits.

Additionally, the `Wifi` and `Eth` trait implementations now provide finer grained control over the behavior of their drivers / services in that users should explicitly call `start`/`stop` to start/stop the driver, as well as `connect`/`disconnect` (for the Wifi driver in STA mode) to connect to an access point. While this makes the driver configuration a bit more involved, these changes provide the necessary flexibility for some corner use cases:
* When the Wifi driver is used together with the `EspNow` ESP NOW service, there is no need to actually `connect` the driver at all, which is now possible
* More complex connect/disconnect schemes can now be implemented by users, for roaming or for reacting in case the Wifi connection is lost

### Http client & server: implement the new traits from `embedded-svc`

Subject says it all, so to say.

### Merge the `nvs_storage` module into `nvs`

* The previous distinction of two separate modules was awkward and is thus removed
* The other notable change here is that the ESP IDF implementation actually only implements the `RawStorage` trait, which provides facilities for reading / writing blobs. It is up to the user to layer a `Storage` implementation on top of the `RawStorage` implementation, but the benefit of that is that user is in control of how their structures are serialized/deserialized into binary. To ease the layering, users may take advantage of the `StorageImpl` structure from `embedded-svc` and only provide a `Serde` trait implementation which abstracts away the concrete Rust SerDe implementation (i.e. `serde-json`, `postcard`, etc.)

### Support for the `embassy-time` crate by providing alarm implementation

`esp-idf-svc` provides an implementation of `embassy-time`'s alarm interface (the `Driver` trait), which is implemented in terms of the ESP IDF Timer service (also exposed in the `timer` module of `esp-idf-svc`). 

To use this feature, users need to enable the `embassy-time-driver` Cargo feature.

### Support for the `embassy-time` crate by providing timer queue implementation

`esp-idf-svc` does provide a custom `embassy-time` Timer Queue implementation (also implemented in terms of the ESP IDF Timer service), even though in the meantime `embassy-time` features a generic timer queue which works everywhere and can also be used. This custom timer queue does not rely on the alarms interface provided by the `embassy-time` crate (see the previous section).

The one major difference between `embassy-time`'s generic timer queue, and the one provided by `esp-idf-svc` is that the latter has a slightly lower latency in that it does support the `CONFIG_ESP_TIMER_SUPPORTS_ISR_DISPATCH_METHOD=y` ESP IDF configuration parameter. When this parameter is enabled, the `esp-idf-svc` timer queue does not use the ESP IDF Timer Service dispatch task/thread and notifies the executor about due timers **directly from the ISR routine**. When this parameter is not enabled, the `esp-idf-svc` timer queue has no benefits compared to the generic timer queue in `embassy-time`.

NOTE: 
* The `esp-idf-svc` timer queue should **only** be used with async executors that are ISR safe, in that they can be awoken from an ISR. `edge-executor` is such an executor.
* `embassy-executor` is currently NOT ISR safe, because it relies - for its synchronization - on the `critical-section` crate, yet the critical section implementation that the `critical-section` crate uses on top of `esp-idf-hal` is based on a FreeRTOS mutex, and NOT on a disable-all-interupts ISR-safe implementation (that is, unless the user has configured a more elaborate setup with their own critical section implementation). On the other hand, `embassy-executor` has its own highly optimized timer queue which should probably be used anyway and is enabled by default
* All other executors can just use the generic timer queue implementation which is built-in in the `embassy-time` crate

To use this feature, users need to enable the `embassy-time-isr-queue` Cargo feature.
