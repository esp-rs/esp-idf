//! HTTP client
//!
//! This provides a set of APIs for making HTTP(S) requests.
//!
//! You can find a usage example at
//! [`examples/http_request.rs`](https://github.com/esp-rs/esp-idf-svc/blob/master/examples/http_request.rs).

use core::cell::UnsafeCell;
use core::fmt::Write as _;

extern crate alloc;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;

use ::log::*;

use embedded_svc::io::{ErrorType, Read, Write};

use crate::sys::*;

use uncased::{Uncased, UncasedStr};

use crate::handle::RawHandle;
use crate::io::EspIOError;
use crate::private::common::Newtype;
use crate::private::cstr::*;
use crate::tls::{HwPrivateKey, RawHwPrivateKey, X509};

pub use embedded_svc::http::client::{Connection, Request, Response};

pub use super::*;

impl From<Method> for Newtype<(esp_http_client_method_t, ())> {
    fn from(method: Method) -> Self {
        Self((
            match method {
                Method::Get => esp_http_client_method_t_HTTP_METHOD_GET,
                Method::Post => esp_http_client_method_t_HTTP_METHOD_POST,
                Method::Delete => esp_http_client_method_t_HTTP_METHOD_DELETE,
                Method::Head => esp_http_client_method_t_HTTP_METHOD_HEAD,
                Method::Put => esp_http_client_method_t_HTTP_METHOD_PUT,
                Method::Options => esp_http_client_method_t_HTTP_METHOD_OPTIONS,
                Method::Copy => esp_http_client_method_t_HTTP_METHOD_COPY,
                Method::Lock => esp_http_client_method_t_HTTP_METHOD_LOCK,
                Method::MkCol => esp_http_client_method_t_HTTP_METHOD_MKCOL,
                Method::Move => esp_http_client_method_t_HTTP_METHOD_MOVE,
                Method::Propfind => esp_http_client_method_t_HTTP_METHOD_PROPFIND,
                Method::Proppatch => esp_http_client_method_t_HTTP_METHOD_PROPPATCH,
                Method::Unlock => esp_http_client_method_t_HTTP_METHOD_UNLOCK,
                Method::Notify => esp_http_client_method_t_HTTP_METHOD_NOTIFY,
                Method::Subscribe => esp_http_client_method_t_HTTP_METHOD_SUBSCRIBE,
                Method::Unsubscribe => esp_http_client_method_t_HTTP_METHOD_UNSUBSCRIBE,
                Method::Patch => esp_http_client_method_t_HTTP_METHOD_PATCH,
                method => panic!("Method {method:?} is not supported"),
            },
            (),
        ))
    }
}

#[derive(Default, Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum FollowRedirectsPolicy {
    FollowNone,
    #[default]
    FollowGetHead,
    FollowAll,
}

#[derive(Clone, Debug, Default)]
pub struct Configuration {
    pub buffer_size: Option<usize>,
    pub buffer_size_tx: Option<usize>,
    pub timeout: Option<core::time::Duration>,
    pub follow_redirects_policy: FollowRedirectsPolicy,
    pub client_certificate: Option<X509<'static>>,
    pub server_certificate: Option<X509<'static>>,
    pub private_key: Option<X509<'static>>,
    /// A private key kept in hardware, used instead of `private_key`
    pub hw_private_key: Option<HwPrivateKey>,
    pub use_global_ca_store: bool,
    pub crt_bundle_attach: Option<unsafe extern "C" fn(conf: *mut core::ffi::c_void) -> esp_err_t>,
    pub raw_request_body: bool,
    pub keep_alive_enable: bool,
    pub keep_alive_idle: Option<usize>,
    pub keep_alive_interval: Option<usize>,
    pub keep_alive_count: Option<usize>,
    /// Keep the TLS session ticket of a connection and resume the session on the next
    /// connection to the same host, instead of a full handshake. Needs
    /// `CONFIG_ESP_TLS_CLIENT_SESSION_TICKETS=y`; ignored without it.
    pub save_client_session: bool,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum State {
    New,
    Request,
    Response,
}

#[allow(clippy::type_complexity)]
pub struct EspHttpConnection {
    raw_client: esp_http_client_handle_t,
    follow_redirects_policy: FollowRedirectsPolicy,
    raw_request_body: bool,
    event_handler: Box<Option<Box<dyn Fn(&esp_http_client_event_t) -> esp_err_t>>>,
    state: State,
    request_content_len: i64,
    follow_redirects: bool,
    headers: BTreeMap<Uncased<'static>, String>,
    content_len_header: UnsafeCell<Option<Option<String>>>,
    _hw_private_key: RawHwPrivateKey,
}

impl EspHttpConnection {
    pub fn new(configuration: &Configuration) -> Result<Self, EspError> {
        let event_handler = Box::new(None);

        #[allow(clippy::manual_c_str_literals)]
        let mut native_config = esp_http_client_config_t {
            // The ESP-IDF HTTP client is really picky on being initialized with a valid URL
            // So we set something here, which will be changed later anyway, in the request() method
            url: b"http://127.0.0.1\0".as_ptr() as *const _,
            event_handler: Some(Self::on_events),
            user_data: &*event_handler as *const _ as *mut core::ffi::c_void,

            use_global_ca_store: configuration.use_global_ca_store,
            crt_bundle_attach: configuration.crt_bundle_attach,
            keep_alive_enable: configuration.keep_alive_enable,
            #[cfg(esp_idf_esp_tls_client_session_tickets)]
            save_client_session: configuration.save_client_session,

            ..Default::default()
        };

        if let Some(buffer_size) = configuration.buffer_size {
            native_config.buffer_size = buffer_size as _;
        };

        if let Some(buffer_size_tx) = configuration.buffer_size_tx {
            native_config.buffer_size_tx = buffer_size_tx as _;
        }

        if let Some(timeout) = configuration.timeout {
            native_config.timeout_ms = timeout.as_millis() as _;
        }

        if let Some(cert) = configuration.server_certificate {
            #[cfg(esp_idf_version_at_least_5_5_0)]
            {
                native_config.__bindgen_anon_1.cert_pem = cert.as_esp_idf_raw_ptr() as _;
            }
            #[cfg(not(esp_idf_version_at_least_5_5_0))]
            {
                native_config.cert_pem = cert.as_esp_idf_raw_ptr() as _;
            }
            native_config.cert_len = cert.as_esp_idf_raw_len();
        }

        if configuration.private_key.is_some() && configuration.hw_private_key.is_some() {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_ARG>());
        }

        let has_private_key =
            configuration.private_key.is_some() || configuration.hw_private_key.is_some();

        if let (Some(cert), true) = (configuration.client_certificate, has_private_key) {
            #[cfg(esp_idf_version_at_least_5_5_0)]
            {
                native_config.__bindgen_anon_2.client_cert_pem = cert.as_esp_idf_raw_ptr() as _;
            }

            #[cfg(not(esp_idf_version_at_least_5_5_0))]
            {
                native_config.client_cert_pem = cert.as_esp_idf_raw_ptr() as _;
            }

            native_config.client_cert_len = cert.as_esp_idf_raw_len();

            if let Some(private_key) = configuration.private_key {
                native_config.client_key_pem = private_key.as_esp_idf_raw_ptr() as _;
                native_config.client_key_len = private_key.as_esp_idf_raw_len();
            }
        }

        let hw_private_key = RawHwPrivateKey::new(configuration.hw_private_key.as_ref());

        #[cfg(all(esp_idf_esp_tls_use_ds_peripheral, esp_idf_version_at_least_5_1_0))]
        {
            native_config.ds_data = hw_private_key.ds_data();
        }

        #[cfg(esp_idf_mbedtls_hardware_ecdsa_sign)]
        if let Some(efuse_block) = hw_private_key.ecdsa_efuse_block() {
            native_config.use_ecdsa_peripheral = true;
            native_config.ecdsa_key_efuse_blk = efuse_block;
        }

        if configuration.keep_alive_enable {
            if let Some(keep_alive_idle) = configuration.keep_alive_idle {
                native_config.keep_alive_idle = keep_alive_idle as _;
            }
            if let Some(keep_alive_interval) = configuration.keep_alive_interval {
                native_config.keep_alive_interval = keep_alive_interval as _;
            }
            if let Some(keep_alive_count) = configuration.keep_alive_count {
                native_config.keep_alive_count = keep_alive_count as _;
            }
        }

        let raw_client = unsafe { esp_http_client_init(&native_config) };
        if raw_client.is_null() {
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(Self {
                raw_client,
                follow_redirects_policy: configuration.follow_redirects_policy,
                raw_request_body: configuration.raw_request_body,
                event_handler,
                state: State::New,
                request_content_len: -1,
                follow_redirects: false,
                headers: BTreeMap::new(),
                content_len_header: UnsafeCell::new(None),
                _hw_private_key: hw_private_key,
            })
        }
    }

    pub fn status(&self) -> u16 {
        self.assert_response();
        unsafe { esp_http_client_get_status_code(self.raw_client) as _ }
    }

    pub fn status_message(&self) -> Option<&str> {
        self.assert_response();
        None
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.assert_response();

        if name.eq_ignore_ascii_case("Content-Length") {
            if let Some(content_len_opt) =
                unsafe { self.content_len_header.get().as_mut().unwrap() }.as_ref()
            {
                content_len_opt.as_ref().map(|s| s.as_str())
            } else {
                let content_len = unsafe { esp_http_client_get_content_length(self.raw_client) };
                *unsafe { self.content_len_header.get().as_mut().unwrap() } = if content_len >= 0 {
                    Some(Some(content_len.to_string()))
                } else {
                    None
                };

                unsafe { self.content_len_header.get().as_mut().unwrap() }
                    .as_ref()
                    .and_then(|s| s.as_ref().map(|s| s.as_ref()))
            }
        } else {
            self.headers.get(UncasedStr::new(name)).map(|s| s.as_str())
        }
    }

    pub fn initiate_request<'a>(
        &'a mut self,
        method: Method,
        uri: &'a str,
        headers: &'a [(&'a str, &'a str)],
    ) -> Result<(), EspError> {
        // If response data from the previous request remains, subsequent requests may fail
        if self.is_response_initiated() {
            self.flush_response()?;
        }

        self.assert_initial();

        let c_uri = to_cstring_arg(uri)?;

        esp!(unsafe { esp_http_client_set_url(self.raw_client, c_uri.as_ptr() as _) })?;
        esp!(unsafe {
            esp_http_client_set_method(
                self.raw_client,
                Newtype::<(esp_http_client_method_t, ())>::from(method).0 .0,
            )
        })?;

        let mut content_len = None;

        for (name, value) in headers {
            if name.eq_ignore_ascii_case("Content-Length") {
                if let Ok(len) = value.parse::<i64>() {
                    content_len = Some(len);
                }
            } else {
                let c_name = to_cstring_arg(name)?;

                // TODO: Replace with a proper conversion from UTF8 to ISO-8859-1
                let c_value = to_cstring_arg(value)?;

                esp!(unsafe {
                    esp_http_client_set_header(
                        self.raw_client,
                        c_name.as_ptr() as _,
                        c_value.as_ptr() as _,
                    )
                })?;
            }
        }

        self.follow_redirects = match self.follow_redirects_policy {
            FollowRedirectsPolicy::FollowAll => true,
            FollowRedirectsPolicy::FollowGetHead => method == Method::Get || method == Method::Head,
            _ => false,
        };

        // No Content-Length for POST requests means chunked encoding
        // This is indicated to the ESP IDF client by setting the
        // content length param of `esp_http_client_open` to -1
        self.request_content_len =
            content_len.unwrap_or(if method == Method::Post { -1 } else { 0 });

        esp!(unsafe { esp_http_client_open(self.raw_client, self.request_content_len as i32) })?;

        self.state = State::Request;

        Ok(())
    }

    pub fn is_request_initiated(&self) -> bool {
        self.state == State::Request
    }

    pub fn initiate_response(&mut self) -> Result<(), EspError> {
        self.assert_request();

        self.flush()?;
        self.fetch_headers()?;

        self.state = State::Response;

        Ok(())
    }

    pub fn is_response_initiated(&self) -> bool {
        self.state == State::Response
    }

    pub fn split(&mut self) -> (&EspHttpConnection, &mut Self) {
        self.assert_response();

        let headers_ptr: *const EspHttpConnection = self as *const _;

        let headers = unsafe { headers_ptr.as_ref().unwrap() };

        (headers, self)
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspError> {
        self.assert_response();

        let result = Self::check(unsafe {
            esp_http_client_read(self.raw_client, buf.as_mut_ptr() as _, buf.len() as _)
        });

        // workaround since esp_http_client_read does not yet return EAGAIN error in ESP-IDF v4.
        // in ESP-IDF v5 esp_http_client_read will return EAGAIN and this should not be needed.
        #[cfg(esp_idf_version_major = "4")]
        {
            match result {
                Ok(0) if unsafe { !esp_http_client_is_complete_data_received(self.raw_client) } => {
                    // no error but read 0 bytes and body is not yet complete, probably caused by EAGAIN
                    Err(EspError::from_infallible::<ESP_ERR_HTTP_EAGAIN>())
                }
                other => other,
            }
        }
        #[cfg(not(esp_idf_version_major = "4"))]
        {
            result
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, EspError> {
        if buf.is_empty() {
            Ok(0)
        } else if !self.raw_request_body && self.request_content_len == -1 {
            // Assume chunked encoding and send the buffer as the next chunk
            let mut len_buf = heapless::String::<8>::new();
            write!(&mut len_buf, "{:x}", buf.len()).unwrap();

            self.raw_write_all(len_buf.as_bytes())?;
            self.raw_write_all(b"\r\n")?;

            self.raw_write_all(buf)?;
            self.raw_write_all(b"\r\n")?;

            Ok(buf.len())
        } else {
            self.raw_write(buf)
        }
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<(), EspError> {
        let mut offset = 0;

        while offset < data.len() {
            offset += self.write(&data[offset..])?;
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), EspError> {
        if !self.raw_request_body && self.request_content_len == -1 {
            // Finish the chunked-encoded stream
            self.raw_write_all(b"0\r\n\r\n")
        } else {
            Ok(())
        }
    }

    /// Closes the connection to the server, keeping the client and what it holds: its
    /// buffers, its configuration and, with `Configuration::save_client_session`, the TLS
    /// session ticket, so the next request resumes the TLS session instead of a full
    /// handshake. A response that was not read to the end is discarded. The connection is
    /// in its initial state afterwards and ready for `initiate_request`.
    pub fn close(&mut self) -> Result<(), EspError> {
        esp!(unsafe { esp_http_client_close(self.raw_client) })?;
        self.state = State::New;

        Ok(())
    }

    fn flush_response(&mut self) -> Result<(), EspError> {
        let mut len = 0_i32;
        esp!(unsafe { esp_http_client_flush_response(self.raw_client, &mut len) })?;

        Ok(())
    }

    fn raw_write(&mut self, buf: &[u8]) -> Result<usize, EspError> {
        self.assert_request();

        Self::check(unsafe {
            esp_http_client_write(self.raw_client, buf.as_ptr() as _, buf.len() as _)
        })
    }

    fn raw_write_all(&mut self, data: &[u8]) -> Result<(), EspError> {
        let mut offset = 0;

        while offset < data.len() {
            offset += self.raw_write(&data[offset..])?;
        }

        Ok(())
    }

    fn check(result: i32) -> Result<usize, EspError> {
        match EspError::from(result) {
            Some(err) if result < 0 => Err(err),
            _ => Ok(result as _),
        }
    }

    extern "C" fn on_events(event: *mut esp_http_client_event_t) -> esp_err_t {
        match unsafe { event.as_mut() } {
            Some(event) => {
                let handler = event.user_data
                    as *const Option<Box<dyn Fn(&esp_http_client_event_t) -> esp_err_t>>;
                if let Some(handler) = unsafe { handler.as_ref() } {
                    if let Some(handler) = handler.as_ref() {
                        return handler(event);
                    }
                }

                ESP_OK as _
            }
            None => ESP_FAIL as _,
        }
    }

    fn fetch_headers(&mut self) -> Result<(), EspError> {
        self.headers.clear();
        *self.content_len_header.get_mut() = None;

        loop {
            // TODO: Implement a mechanism where the client can declare in which header it is interested
            let headers_ptr = &mut self.headers as *mut BTreeMap<Uncased, String>;

            let handler = move |event: &esp_http_client_event_t| {
                if event.event_id == esp_http_client_event_id_t_HTTP_EVENT_ON_HEADER {
                    unsafe {
                        // TODO: Replace with a proper conversion from ISO-8859-1 to UTF8

                        headers_ptr.as_mut().unwrap().insert(
                            Uncased::from(from_cstr_ptr(event.header_key).to_string()),
                            from_cstr_ptr(event.header_value).to_string(),
                        );
                    }
                }

                ESP_OK as esp_err_t
            };

            self.register_handler(handler);

            let result = unsafe { esp_http_client_fetch_headers(self.raw_client) };

            self.deregister_handler();

            Self::check(result as _)?;

            trace!("Fetched headers: {:?}", self.headers);

            if self.follow_redirects {
                let status = unsafe { esp_http_client_get_status_code(self.raw_client) as u16 };

                if status::REDIRECT.contains(&status) && status != 304 {
                    info!("Got response {status}, about to follow redirect");

                    let mut len = 0_i32;
                    esp!(unsafe { esp_http_client_flush_response(self.raw_client, &mut len) })?;
                    esp!(unsafe {
                        esp_http_client_set_method(
                            self.raw_client,
                            esp_http_client_method_t_HTTP_METHOD_GET,
                        )
                    })?;
                    esp!(unsafe { esp_http_client_set_redirection(self.raw_client) })?;
                    esp!(unsafe {
                        esp_http_client_open(self.raw_client, self.request_content_len as i32)
                    })?;

                    self.headers.clear();

                    continue;
                }
            }

            break;
        }

        Ok(())
    }

    fn register_handler(
        &mut self,
        handler: impl Fn(&esp_http_client_event_t) -> esp_err_t + 'static,
    ) {
        *self.event_handler = Some(Box::new(handler));
    }

    fn deregister_handler(&mut self) {
        *self.event_handler = None;
    }

    fn assert_initial(&self) {
        if self.state != State::New && self.state != State::Response {
            panic!("connection is not in initial phase");
        }
    }

    fn assert_request(&self) {
        if self.state != State::Request {
            panic!("connection is not in request phase");
        }
    }

    fn assert_response(&self) {
        if self.state != State::Response {
            panic!("connection is not in response phase");
        }
    }
}

impl Drop for EspHttpConnection {
    fn drop(&mut self) {
        esp!(unsafe { esp_http_client_cleanup(self.raw_client) })
            .expect("Unable to stop the client cleanly");
    }
}

impl RawHandle for EspHttpConnection {
    type Handle = esp_http_client_handle_t;

    fn handle(&self) -> Self::Handle {
        self.raw_client
    }
}

impl embedded_svc::http::Status for EspHttpConnection {
    fn status(&self) -> u16 {
        EspHttpConnection::status(self)
    }

    fn status_message(&self) -> Option<&str> {
        EspHttpConnection::status_message(self)
    }
}

impl embedded_svc::http::Headers for EspHttpConnection {
    fn header(&self, name: &str) -> Option<&str> {
        EspHttpConnection::header(self, name)
    }
}

impl ErrorType for EspHttpConnection {
    type Error = EspIOError;
}

impl Read for EspHttpConnection {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let size = EspHttpConnection::read(self, buf)?;

        Ok(size)
    }
}

impl Write for EspHttpConnection {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let size = EspHttpConnection::write(self, buf)?;

        Ok(size)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.assert_request();

        Ok(())
    }
}

impl embedded_svc::http::client::Connection for EspHttpConnection {
    type Headers = Self;

    type Read = Self;

    type RawConnectionError = EspIOError;

    type RawConnection = Self;

    fn initiate_request<'a>(
        &'a mut self,
        method: Method,
        uri: &'a str,
        headers: &'a [(&'a str, &'a str)],
    ) -> Result<(), Self::Error> {
        EspHttpConnection::initiate_request(self, method, uri, headers).map_err(EspIOError)
    }

    fn is_request_initiated(&self) -> bool {
        EspHttpConnection::is_request_initiated(self)
    }

    fn initiate_response(&mut self) -> Result<(), Self::Error> {
        EspHttpConnection::initiate_response(self).map_err(EspIOError)
    }

    fn is_response_initiated(&self) -> bool {
        EspHttpConnection::is_response_initiated(self)
    }

    fn split(&mut self) -> (&Self::Headers, &mut Self::Read) {
        EspHttpConnection::split(self)
    }

    fn raw_connection(&mut self) -> Result<&mut Self::RawConnection, Self::Error> {
        Err(EspError::from_infallible::<ESP_FAIL>().into())
    }
}
