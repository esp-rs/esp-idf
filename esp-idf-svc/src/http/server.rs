//! HTTP server
//!
//! Provides an HTTP(S) server in `EspHttpServer`, plus all related structs.
//!
//! Typical usage of `EspHttpServer` involves creating a function (or closure)
//! for every URI+method that the server is meant to handle. A minimal server that
//! only handles HTTP GET requests to `index.html` looks like this:
//!
//! ```
//! use esp_idf_svc::http::server::{Configuration, EspHttpServer};
//!
//! let mut server = EspHttpServer::new(&Configuration::default())?;
//!
//! server.fn_handler("/index.html", Method::Get, |request| {
//!     request
//!         .into_ok_response()?
//!         .write_all(b"<html><body>Hello world!</body></html>")
//! })?;
//! ```
//!
//! Note that the server is automatically started when instantiated, and stopped
//! when dropped. If you want to keep the server running indefinitely then
//! make sure it's not dropped - you may add an infinite loop after the server
//! is created, use `core::mem::forget`, or keep around a reference to it somehow.
//!
//! You can find an example of handling GET/POST requests at [`examples/http_server.rs`](https://github.com/esp-rs/esp-idf-svc/blob/master/examples/http_server.rs).
//!
//! You can find an example of HTTP+Websockets at [`examples/http_ws_server.rs`](https://github.com/esp-rs/esp-idf-svc/blob/master/examples/http_ws_server.rs).
//!
//! By default, the ESP-IDF library allocates 512 bytes for reading and parsing
//! HTTP headers, but desktop web browsers might send headers longer than that.
//! If this becomes a problem, add `CONFIG_HTTPD_MAX_REQ_HDR_LEN=1024` to your
//! `sdkconfig.defaults` file.

use core::cell::UnsafeCell;
use core::fmt::Debug;
use core::marker::PhantomData;
#[cfg(esp_idf_lwip_ipv4)]
use core::net::Ipv4Addr;
#[cfg(esp_idf_lwip_ipv6)]
use core::net::Ipv6Addr;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::*;
use core::{ffi, ptr};

extern crate alloc;
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;

use ::log::{info, warn};

use embedded_svc::http::headers::content_type;
use embedded_svc::io::{ErrorType, Read, Write};

use esp_idf_hal::cpu::Core;

use crate::sys::*;

use uncased::{Uncased, UncasedStr};

use crate::handle::RawHandle;
use crate::io::EspIOError;
use crate::private::common::Newtype;
use crate::private::cstr::to_cstring_arg;
use crate::private::cstr::{CStr, CString};
use crate::private::mutex::Mutex;
#[cfg(esp_idf_esp_https_server_enable)]
use crate::tls::X509;

pub use embedded_svc::http::server::{
    CompositeHandler, Connection, FnHandler, Handler, Middleware, Request, Response,
};
pub use embedded_svc::utils::http::server::registration::*;

pub use super::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct KeepAlive {
    pub idle_secs: u32,
    pub interval_secs: u32,
    pub probe_count: u32,
}

impl KeepAlive {
    pub const fn new() -> Self {
        Self {
            idle_secs: 5,
            interval_secs: 5,
            probe_count: 3,
        }
    }
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Configuration {
    pub http_port: u16,
    pub ctrl_port: u16,
    pub https_port: u16,
    pub core: Option<Core>,
    pub max_sessions: usize,
    pub session_timeout: Duration,
    pub task_caps: u32,
    pub stack_size: usize,
    pub max_open_sockets: usize,
    pub max_uri_handlers: usize,
    pub max_resp_headers: usize,
    pub lru_purge_enable: bool,
    pub uri_match_wildcard: bool,
    pub keep_alive: Option<KeepAlive>,
    pub so_linger: Option<Duration>,
    #[cfg(esp_idf_esp_https_server_enable)]
    pub server_certificate: Option<X509<'static>>,
    #[cfg(esp_idf_esp_https_server_enable)]
    pub private_key: Option<X509<'static>>,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            http_port: 80,
            ctrl_port: 32768,
            https_port: 443,
            core: None,
            max_sessions: 16,
            session_timeout: Duration::from_secs(20 * 60),
            task_caps: (MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT),
            #[cfg(not(esp_idf_esp_https_server_enable))]
            stack_size: 6144,
            #[cfg(esp_idf_esp_https_server_enable)]
            stack_size: 10240,
            max_open_sockets: 4,
            max_uri_handlers: 32,
            max_resp_headers: 8,
            lru_purge_enable: true,
            uri_match_wildcard: false,
            keep_alive: None,
            so_linger: None,
            #[cfg(esp_idf_esp_https_server_enable)]
            server_certificate: None,
            #[cfg(esp_idf_esp_https_server_enable)]
            private_key: None,
        }
    }
}

impl From<&Configuration> for Newtype<httpd_config_t> {
    #[allow(clippy::needless_update)]
    fn from(conf: &Configuration) -> Self {
        Self(httpd_config_t {
            task_priority: 5,
            // Since 5.3.0
            #[cfg(any(
                all(not(esp_idf_version_major = "4"), not(esp_idf_version_major = "5")),
                all(
                    esp_idf_version_major = "5",
                    not(any(
                        esp_idf_version_minor = "0",
                        esp_idf_version_minor = "1",
                        esp_idf_version_minor = "2"
                    ))
                ),
            ))]
            task_caps: conf.task_caps,
            stack_size: conf.stack_size,
            core_id: conf.core.map(|core| core.into()).unwrap_or(i32::MAX),
            server_port: conf.http_port,
            ctrl_port: conf.ctrl_port,
            max_open_sockets: conf.max_open_sockets as _,
            max_uri_handlers: conf.max_uri_handlers as _,
            max_resp_headers: conf.max_resp_headers as _,
            backlog_conn: 5,
            lru_purge_enable: conf.lru_purge_enable,
            recv_wait_timeout: 5,
            send_wait_timeout: 5,
            global_user_ctx: ptr::null_mut(),
            global_user_ctx_free_fn: None,
            global_transport_ctx: ptr::null_mut(),
            global_transport_ctx_free_fn: None,
            open_fn: None,
            close_fn: None,
            uri_match_fn: conf.uri_match_wildcard.then_some(httpd_uri_match_wildcard),
            keep_alive_enable: conf.keep_alive.is_some(),
            keep_alive_idle: conf.keep_alive.map(|ka| ka.idle_secs as i32).unwrap_or(0),
            keep_alive_interval: conf
                .keep_alive
                .map(|ka| ka.interval_secs as i32)
                .unwrap_or(0),
            keep_alive_count: conf.keep_alive.map(|ka| ka.probe_count as i32).unwrap_or(0),
            enable_so_linger: conf.so_linger.is_some(),
            linger_timeout: conf.so_linger.map(|d| d.as_secs() as i32).unwrap_or(0),
            ..Default::default()
        })
    }
}

#[allow(non_upper_case_globals)]
impl From<Newtype<ffi::c_uint>> for Method {
    fn from(method: Newtype<ffi::c_uint>) -> Self {
        match method.0 {
            http_method_HTTP_GET => Method::Get,
            http_method_HTTP_POST => Method::Post,
            http_method_HTTP_DELETE => Method::Delete,
            http_method_HTTP_HEAD => Method::Head,
            http_method_HTTP_PUT => Method::Put,
            http_method_HTTP_CONNECT => Method::Connect,
            http_method_HTTP_OPTIONS => Method::Options,
            http_method_HTTP_TRACE => Method::Trace,
            http_method_HTTP_COPY => Method::Copy,
            http_method_HTTP_LOCK => Method::Lock,
            http_method_HTTP_MKCOL => Method::MkCol,
            http_method_HTTP_MOVE => Method::Move,
            http_method_HTTP_PROPFIND => Method::Propfind,
            http_method_HTTP_PROPPATCH => Method::Proppatch,
            http_method_HTTP_SEARCH => Method::Search,
            http_method_HTTP_UNLOCK => Method::Unlock,
            http_method_HTTP_BIND => Method::Bind,
            http_method_HTTP_REBIND => Method::Rebind,
            http_method_HTTP_UNBIND => Method::Unbind,
            http_method_HTTP_ACL => Method::Acl,
            http_method_HTTP_REPORT => Method::Report,
            http_method_HTTP_MKACTIVITY => Method::MkActivity,
            http_method_HTTP_CHECKOUT => Method::Checkout,
            http_method_HTTP_MERGE => Method::Merge,
            http_method_HTTP_MSEARCH => Method::MSearch,
            http_method_HTTP_NOTIFY => Method::Notify,
            http_method_HTTP_SUBSCRIBE => Method::Subscribe,
            http_method_HTTP_UNSUBSCRIBE => Method::Unsubscribe,
            http_method_HTTP_PATCH => Method::Patch,
            http_method_HTTP_PURGE => Method::Purge,
            http_method_HTTP_MKCALENDAR => Method::MkCalendar,
            http_method_HTTP_LINK => Method::Link,
            http_method_HTTP_UNLINK => Method::Unlink,
            _ => unreachable!(),
        }
    }
}

#[cfg(esp_idf_esp_https_server_enable)]
impl From<&Configuration> for Newtype<httpd_ssl_config_t> {
    fn from(conf: &Configuration) -> Self {
        let http_config: Newtype<httpd_config_t> = conf.into();
        // start in insecure mode if no certificates are set
        let transport_mode = match (conf.server_certificate, conf.private_key) {
            (Some(_), Some(_)) => httpd_ssl_transport_mode_t_HTTPD_SSL_TRANSPORT_SECURE,
            _ => {
                warn!("Starting server in insecure mode because no certificates were set in the http config.");
                httpd_ssl_transport_mode_t_HTTPD_SSL_TRANSPORT_INSECURE
            }
        };
        // Default values taken from: https://github.com/espressif/esp-idf/blob/master/components/esp_https_server/include/esp_https_server.h#L114

        #[allow(clippy::needless_update)]
        Self(httpd_ssl_config_t {
            httpd: http_config.0,
            session_tickets: false,
            #[cfg(not(esp_idf_version_major = "4"))]
            use_secure_element: false,
            port_secure: conf.https_port,
            port_insecure: conf.http_port,
            transport_mode,
            cacert_pem: ptr::null(),
            cacert_len: 0,
            prvtkey_pem: ptr::null(),
            prvtkey_len: 0,
            #[cfg(esp_idf_version_major = "4")]
            client_verify_cert_pem: ptr::null(),
            #[cfg(esp_idf_version_major = "4")]
            client_verify_cert_len: 0,
            #[cfg(not(esp_idf_version_major = "4"))]
            servercert: ptr::null(),
            #[cfg(not(esp_idf_version_major = "4"))]
            servercert_len: 0,
            user_cb: None,
            ..Default::default()
        })
    }
}

impl From<Method> for Newtype<ffi::c_uint> {
    fn from(method: Method) -> Self {
        Self(match method {
            Method::Get => http_method_HTTP_GET,
            Method::Post => http_method_HTTP_POST,
            Method::Delete => http_method_HTTP_DELETE,
            Method::Head => http_method_HTTP_HEAD,
            Method::Put => http_method_HTTP_PUT,
            Method::Connect => http_method_HTTP_CONNECT,
            Method::Options => http_method_HTTP_OPTIONS,
            Method::Trace => http_method_HTTP_TRACE,
            Method::Copy => http_method_HTTP_COPY,
            Method::Lock => http_method_HTTP_LOCK,
            Method::MkCol => http_method_HTTP_MKCOL,
            Method::Move => http_method_HTTP_MOVE,
            Method::Propfind => http_method_HTTP_PROPFIND,
            Method::Proppatch => http_method_HTTP_PROPPATCH,
            Method::Search => http_method_HTTP_SEARCH,
            Method::Unlock => http_method_HTTP_UNLOCK,
            Method::Bind => http_method_HTTP_BIND,
            Method::Rebind => http_method_HTTP_REBIND,
            Method::Unbind => http_method_HTTP_UNBIND,
            Method::Acl => http_method_HTTP_ACL,
            Method::Report => http_method_HTTP_REPORT,
            Method::MkActivity => http_method_HTTP_MKACTIVITY,
            Method::Checkout => http_method_HTTP_CHECKOUT,
            Method::Merge => http_method_HTTP_MERGE,
            Method::MSearch => http_method_HTTP_MSEARCH,
            Method::Notify => http_method_HTTP_NOTIFY,
            Method::Subscribe => http_method_HTTP_SUBSCRIBE,
            Method::Unsubscribe => http_method_HTTP_UNSUBSCRIBE,
            Method::Patch => http_method_HTTP_PATCH,
            Method::Purge => http_method_HTTP_PURGE,
            Method::MkCalendar => http_method_HTTP_MKCALENDAR,
            Method::Link => http_method_HTTP_LINK,
            Method::Unlink => http_method_HTTP_UNLINK,
        })
    }
}

static OPEN_SESSIONS: Mutex<BTreeMap<(u32, ffi::c_int), Arc<AtomicBool>>> =
    Mutex::new(BTreeMap::new());
static CLOSE_HANDLERS: Mutex<BTreeMap<u32, Vec<CloseHandler<'static>>>> =
    Mutex::new(BTreeMap::new());

type NativeHandler<'a> = Box<dyn Fn(*mut httpd_req_t) -> ffi::c_int + 'a>;
type CloseHandler<'a> = Box<dyn Fn(ffi::c_int) + Send + 'a>;

pub struct EspHttpServer<'a> {
    sd: httpd_handle_t,
    registrations: Vec<(CString, crate::sys::httpd_uri_t)>,
    _reg: PhantomData<&'a ()>,
}

impl EspHttpServer<'static> {
    pub fn new(conf: &Configuration) -> Result<Self, EspIOError> {
        Self::internal_new(conf)
    }
}

/// HTTP server
impl<'a> EspHttpServer<'a> {
    /// # Safety
    ///
    /// This method - in contrast to method `new` - allows the user to set
    /// non-static callbacks/closures as handlers into the returned `EspHttpServer` service. This enables users to borrow
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
    pub unsafe fn new_nonstatic(conf: &Configuration) -> Result<Self, EspIOError> {
        Self::internal_new(conf)
    }

    fn internal_new(conf: &Configuration) -> Result<Self, EspIOError> {
        let mut handle: httpd_handle_t = ptr::null_mut();
        let handle_ref = &mut handle;

        #[cfg(not(esp_idf_esp_https_server_enable))]
        {
            let mut config: Newtype<httpd_config_t> = conf.into();
            config.0.close_fn = Some(Self::close_fn);
            esp!(unsafe { httpd_start(handle_ref, &config.0 as *const _) })?;
        }

        #[cfg(esp_idf_esp_https_server_enable)]
        {
            let mut config: Newtype<httpd_ssl_config_t> = conf.into();
            config.0.httpd.close_fn = Some(Self::close_fn);

            if let (Some(cert), Some(private_key)) = (conf.server_certificate, conf.private_key) {
                // NOTE: Contrary to other components in ESP IDF (HTTP & MQTT client),
                // HTTP server does allocate internal buffers for the certificates
                // Moreover - due to internal implementation details - it needs the
                // full length of the certificate, even for the PEM case

                #[cfg(esp_idf_version_major = "4")]
                {
                    config.0.cacert_pem = cert.as_esp_idf_raw_ptr() as _;
                    config.0.cacert_len = cert.as_esp_idf_raw_len();
                }

                #[cfg(not(esp_idf_version_major = "4"))]
                {
                    config.0.servercert = cert.as_esp_idf_raw_ptr() as _;
                    config.0.servercert_len = cert.as_esp_idf_raw_len();
                }

                config.0.prvtkey_pem = private_key.as_esp_idf_raw_ptr() as _;
                config.0.prvtkey_len = private_key.as_esp_idf_raw_len();

                esp!(unsafe { httpd_ssl_start(handle_ref, &mut config.0) })?;
            } else {
                esp!(unsafe { httpd_ssl_start(handle_ref, &mut config.0) })?;
            }
        }

        info!("Started Httpd server with config {conf:?}");

        let server = Self {
            sd: handle,
            registrations: Vec::new(),
            _reg: PhantomData,
        };

        CLOSE_HANDLERS.lock().insert(server.sd as _, Vec::new());

        Ok(server)
    }

    /// Unregisters a URI.
    fn unregister(&mut self, uri: CString, conf: httpd_uri_t) -> Result<(), EspIOError> {
        unsafe {
            esp!(httpd_unregister_uri_handler(
                self.sd,
                uri.as_ptr() as _,
                conf.method
            ))?;

            let _drop = Box::from_raw(conf.user_ctx as *mut NativeHandler<'static>);
        };

        info!(
            "Unregistered Httpd server handler {:?} for URI \"{}\"",
            conf.method,
            uri.to_str().unwrap()
        );

        Ok(())
    }

    /// Stops the server.
    fn stop(&mut self) -> Result<(), EspIOError> {
        if !self.sd.is_null() {
            while let Some((uri, registration)) = self.registrations.pop() {
                self.unregister(uri, registration)?;
            }

            // Maybe its better to always call httpd_stop because httpd_ssl_stop directly wraps httpd_stop anyways
            // https://github.com/espressif/esp-idf/blob/e6fda46a02c41777f1d116a023fbec6a1efaffb9/components/esp_https_server/src/https_server.c#L268
            #[cfg(not(esp_idf_esp_https_server_enable))]
            esp!(unsafe { crate::sys::httpd_stop(self.sd) })?;

            // httpd_ssl_stop doesn't return EspErr for some reason. It returns void.
            #[cfg(all(esp_idf_esp_https_server_enable, esp_idf_version_major = "4"))]
            unsafe {
                crate::sys::httpd_ssl_stop(self.sd)
            };

            // esp-idf version 5 does return EspErr
            #[cfg(all(esp_idf_esp_https_server_enable, not(esp_idf_version_major = "4")))]
            esp!(unsafe { crate::sys::httpd_ssl_stop(self.sd) })?;

            CLOSE_HANDLERS.lock().remove(&(self.sd as u32));

            self.sd = ptr::null_mut();
        }

        info!("Httpd server stopped");

        Ok(())
    }

    pub fn handler_chain<C>(&mut self, chain: C) -> Result<&mut Self, EspError>
    where
        C: EspHttpTraversableChain<'a>,
    {
        chain.accept(self)?;

        Ok(self)
    }

    /// # Safety
    ///
    /// This method - in contrast to method `handler_chain` - allows the user to pass
    /// a chain of non-static callbacks/closures. This enables users to borrow
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
    pub unsafe fn handler_chain_nonstatic<C>(&mut self, chain: C) -> Result<&mut Self, EspError>
    where
        C: EspHttpTraversableChainNonstatic<'a>,
    {
        chain.accept(self)?;

        Ok(self)
    }

    /// Registers a `Handler` for a URI and a method (GET, POST, etc).
    pub fn handler<H>(
        &mut self,
        uri: &str,
        method: Method,
        handler: H,
    ) -> Result<&mut Self, EspError>
    where
        H: for<'r> Handler<EspHttpConnection<'r>> + Send + 'static,
    {
        unsafe { self.handler_nonstatic(uri, method, handler) }
    }

    /// Registers a `Handler` for a URI and a method (GET, POST, etc).
    ///
    /// # Safety
    ///
    /// This method - in contrast to method `handler` - allows the user to pass
    /// a non-static callback/closure. This enables users to borrow
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
    pub unsafe fn handler_nonstatic<H>(
        &mut self,
        uri: &str,
        method: Method,
        handler: H,
    ) -> Result<&mut Self, EspError>
    where
        H: for<'r> Handler<EspHttpConnection<'r>> + Send + 'a,
    {
        let c_str = to_cstring_arg(uri)?;

        #[allow(clippy::needless_update)]
        let conf = httpd_uri_t {
            uri: c_str.as_ptr() as _,
            method: Newtype::<ffi::c_uint>::from(method).0,
            user_ctx: Box::into_raw(Box::new(self.to_native_handler(handler))) as *mut _,
            handler: Some(EspHttpServer::handle_req),
            ..Default::default()
        };

        esp!(unsafe { crate::sys::httpd_register_uri_handler(self.sd, &conf) })?;

        info!(
            "Registered Httpd server handler {:?} for URI \"{}\"",
            method,
            c_str.to_str().unwrap()
        );

        self.registrations.push((c_str, conf));

        Ok(self)
    }

    /// Registers a function as the handler for the given URI and HTTP method (GET, POST, etc).
    ///
    /// The function will be called every time an HTTP client requests that URI
    /// (via the appropriate HTTP method), receiving a different `Request` each
    /// call. The `Request` contains a reference to the underlying `EspHttpConnection`.
    pub fn fn_handler<E, F>(
        &mut self,
        uri: &str,
        method: Method,
        f: F,
    ) -> Result<&mut Self, EspError>
    where
        F: for<'r> Fn(Request<&mut EspHttpConnection<'r>>) -> Result<(), E> + Send + 'static,
        E: Debug,
    {
        unsafe { self.fn_handler_nonstatic(uri, method, f) }
    }

    /// Registers a function as the handler for the given URI and HTTP method (GET, POST, etc).
    ///
    /// The function will be called every time an HTTP client requests that URI
    /// (via the appropriate HTTP method), receiving a different `Request` each
    /// call. The `Request` contains a reference to the underlying `EspHttpConnection`.
    ///
    /// # Safety
    ///
    /// This method - in contrast to method `fn_handler` - allows the user to pass
    /// a non-static callback/closure. This enables users to borrow
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
    pub unsafe fn fn_handler_nonstatic<E, F>(
        &mut self,
        uri: &str,
        method: Method,
        f: F,
    ) -> Result<&mut Self, EspError>
    where
        F: for<'r> Fn(Request<&mut EspHttpConnection<'r>>) -> Result<(), E> + Send + 'a,
        E: Debug,
    {
        self.handler_nonstatic(uri, method, FnHandler::new(f))
    }

    fn to_native_handler<H>(&self, handler: H) -> NativeHandler<'a>
    where
        H: Handler<EspHttpConnection<'a>> + Send + 'a,
    {
        Box::new(move |raw_req| {
            let mut connection = EspHttpConnection::new(unsafe { raw_req.as_mut().unwrap() });

            let result = connection.invoke(&handler);

            match result {
                Ok(()) => {
                    if let Err(e) = connection.complete() {
                        connection.handle_error(e);
                    }
                }
                Err(e) => {
                    connection.handle_error(e);
                    if let Err(e) = connection.complete() {
                        connection.handle_error(e);
                    }
                }
            }

            ESP_OK as _
        })
    }

    extern "C" fn handle_req(raw_req: *mut httpd_req_t) -> ffi::c_int {
        let handler_ptr = (unsafe { *raw_req }).user_ctx as *mut NativeHandler<'static>;

        let handler = unsafe { handler_ptr.as_ref() }.unwrap();

        (handler)(raw_req)
    }

    extern "C" fn close_fn(sd: httpd_handle_t, sockfd: ffi::c_int) {
        {
            let mut sessions = OPEN_SESSIONS.lock();

            if let Some(closed) = sessions.remove(&(sd as u32, sockfd)) {
                closed.store(true, Ordering::SeqCst);
            }
        }

        let all_close_handlers = CLOSE_HANDLERS.lock();

        let close_handlers = all_close_handlers.get(&(sd as u32)).unwrap();

        for close_handler in close_handlers {
            (close_handler)(sockfd);
        }
        esp_nofail!(unsafe { close(sockfd) });
    }
}

impl Drop for EspHttpServer<'_> {
    fn drop(&mut self) {
        self.stop().expect("Unable to stop the server cleanly");
    }
}

impl RawHandle for EspHttpServer<'_> {
    type Handle = httpd_handle_t;

    fn handle(&self) -> Self::Handle {
        self.sd
    }
}

/// Wraps the given function into an `FnHandler`.
///
/// Do not confuse with `EspHttpServer::fn_handler`.
pub fn fn_handler<F, E>(f: F) -> FnHandler<F>
where
    F: for<'a> Fn(Request<&mut EspHttpConnection<'a>>) -> Result<(), E> + Send,
    E: Debug,
{
    FnHandler::new(f)
}

pub trait EspHttpTraversableChain<'a> {
    fn accept(self, server: &mut EspHttpServer<'a>) -> Result<(), EspError>;
}

/// # Safety
///
/// Implementing this trait means that the chain can contain non-`'static` handlers
/// and that the chain can be used with method `EspHttpServer::handler_chain_nonstatic`.
///
/// Consult the documentation of `EspHttpServer::handler_chain_nonstatic` for more
/// information on how to use non-static handler chains.
pub unsafe trait EspHttpTraversableChainNonstatic<'a>: EspHttpTraversableChain<'a> {}

impl<'a> EspHttpTraversableChain<'a> for ChainRoot {
    fn accept(self, _server: &mut EspHttpServer<'a>) -> Result<(), EspError> {
        Ok(())
    }
}

impl<'a, H, N> EspHttpTraversableChain<'a> for ChainHandler<H, N>
where
    H: for<'r> Handler<EspHttpConnection<'r>> + Send + 'static,
    N: EspHttpTraversableChain<'a>,
{
    fn accept(self, server: &mut EspHttpServer<'a>) -> Result<(), EspError> {
        self.next.accept(server)?;

        server.handler(self.path, self.method, self.handler)?;

        Ok(())
    }
}

/// A newtype wrapper for `ChainHandler` that allows
/// non-`'static`` handlers  in the chain to be registered
/// and passed to the server.
pub struct NonstaticChain<H, N>(ChainHandler<H, N>);

impl<H, N> NonstaticChain<H, N> {
    /// Wraps the given chain with a `NonstaticChain` newtype.
    pub fn new(handler: ChainHandler<H, N>) -> Self {
        Self(handler)
    }
}

unsafe impl EspHttpTraversableChainNonstatic<'_> for ChainRoot {}

impl<'a, H, N> EspHttpTraversableChain<'a> for NonstaticChain<H, N>
where
    H: for<'r> Handler<EspHttpConnection<'r>> + Send + 'a,
    N: EspHttpTraversableChain<'a>,
{
    fn accept(self, server: &mut EspHttpServer<'a>) -> Result<(), EspError> {
        self.0.next.accept(server)?;

        unsafe {
            server.handler_nonstatic(self.0.path, self.0.method, self.0.handler)?;
        }

        Ok(())
    }
}

unsafe impl<'a, H, N> EspHttpTraversableChainNonstatic<'a> for NonstaticChain<H, N>
where
    H: for<'r> Handler<EspHttpConnection<'r>> + Send + 'a,
    N: EspHttpTraversableChain<'a>,
{
}

pub struct EspHttpRawConnection<'a>(&'a mut httpd_req_t);

impl EspHttpRawConnection<'_> {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspError> {
        if !buf.is_empty() {
            let fd = unsafe { httpd_req_to_sockfd(self.0) };
            let len = unsafe { crate::sys::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };

            Ok(len as _)
        } else {
            Ok(0)
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, EspError> {
        if !buf.is_empty() {
            let fd = unsafe { httpd_req_to_sockfd(self.0) };
            let len = unsafe { crate::sys::write(fd, buf.as_ptr() as *const _, buf.len()) };

            Ok(len as _)
        } else {
            Ok(0)
        }
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<(), EspError> {
        let mut offset = 0;

        while offset < data.len() {
            offset += self.write(&data[offset..])?;
        }

        Ok(())
    }

    /// Retrieves the source IPv4 of the request.
    ///
    /// The IPv4 is retrieved using the underlying session socket.
    #[cfg(esp_idf_lwip_ipv4)]
    pub fn source_ipv4(&self) -> Result<Ipv4Addr, EspError> {
        unsafe {
            let sockfd = httpd_req_to_sockfd(self.handle());

            if sockfd == -1 {
                return Err(EspError::from_infallible::<ESP_FAIL>());
            }

            let mut addr = sockaddr_in {
                sin_len: core::mem::size_of::<sockaddr_in>() as _,
                sin_family: AF_INET as _,
                ..Default::default()
            };

            esp!(lwip_getpeername(
                sockfd,
                &mut addr as *mut _ as *mut _,
                &mut core::mem::size_of::<sockaddr_in>() as *mut _ as *mut _,
            ))?;

            Ok(Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)))
        }
    }

    /// Retrieves the source IPv6 of the request.
    ///
    /// The IPv6 is retrieved using the underlying session socket.
    #[cfg(esp_idf_lwip_ipv6)]
    pub fn source_ipv6(&self) -> Result<Ipv6Addr, EspError> {
        unsafe {
            let sockfd = httpd_req_to_sockfd(self.handle());

            if sockfd == -1 {
                return Err(EspError::from_infallible::<ESP_FAIL>());
            }

            let mut addr = sockaddr_in6 {
                sin6_len: core::mem::size_of::<sockaddr_in6>() as _,
                sin6_family: AF_INET6 as _,
                ..Default::default()
            };

            esp!(lwip_getpeername(
                sockfd,
                &mut addr as *mut _ as *mut _,
                &mut core::mem::size_of::<sockaddr_in6>() as *mut _ as *mut _,
            ))?;

            Ok(Ipv6Addr::from(addr.sin6_addr.un.u8_addr))
        }
    }
}

impl RawHandle for EspHttpRawConnection<'_> {
    type Handle = *mut httpd_req_t;

    fn handle(&self) -> Self::Handle {
        self.0 as *const _ as *mut _
    }
}

impl ErrorType for EspHttpRawConnection<'_> {
    type Error = EspIOError;
}

impl Read for EspHttpRawConnection<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        EspHttpRawConnection::read(self, buf).map_err(EspIOError)
    }
}

impl Write for EspHttpRawConnection<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        EspHttpRawConnection::write(self, buf).map_err(EspIOError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

type EspHttpHeaders = BTreeMap<Uncased<'static>, String>;

pub struct EspHttpConnection<'a> {
    request: EspHttpRawConnection<'a>,
    headers: Option<UnsafeCell<EspHttpHeaders>>,
    response_headers: Option<Vec<CString>>,
}

/// Represents the two-way connection between an HTTP request and its response.
impl<'a> EspHttpConnection<'a> {
    fn new(raw_req: &'a mut httpd_req_t) -> Self {
        Self {
            request: EspHttpRawConnection(raw_req),
            headers: Some(UnsafeCell::new(EspHttpHeaders::new())),
            response_headers: None,
        }
    }

    // Returns the URI for the current request in this connection.
    pub fn uri(&self) -> &str {
        self.assert_request();

        let c_uri = unsafe { CStr::from_ptr(self.request.0.uri.as_ptr()) };

        c_uri.to_str().unwrap()
    }

    // Returns the HTTP method for the current request in this connection.
    pub fn method(&self) -> Method {
        self.assert_request();

        Method::from(Newtype(self.request.0.method as u32))
    }

    // Searches for the header of the given name in the HTTP request's headers.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.assert_request();

        let headers = self.headers.as_ref().unwrap();

        if let Some(value) = unsafe { headers.get().as_ref().unwrap() }.get(UncasedStr::new(name)) {
            Some(value.as_ref())
        } else {
            let raw_req = self.request.0 as *const httpd_req_t as *mut httpd_req_t;

            if let Ok(c_name) = to_cstring_arg(name) {
                match unsafe { httpd_req_get_hdr_value_len(raw_req, c_name.as_ptr() as _) } {
                    0 => None,
                    len => {
                        // TODO: Would've been much more effective, if ESP-IDF was capable of returning a
                        // pointer to the header value that is in the scratch buffer
                        //
                        // Check if we can implement it ourselves vy traversing the scratch buffer manually

                        let mut buf: Vec<u8> = Vec::with_capacity(len + 1);

                        esp_nofail!(unsafe {
                            httpd_req_get_hdr_value_str(
                                raw_req,
                                c_name.as_ptr(),
                                buf.as_mut_ptr().cast(),
                                len + 1,
                            )
                        });

                        unsafe {
                            buf.set_len(len + 1);
                        }

                        // TODO: Replace with a proper conversion from ISO-8859-1 to UTF8
                        let value = String::from_utf8_lossy(&buf[..len]).into_owned();
                        unsafe { headers.get().as_mut().unwrap() }
                            .insert(Uncased::from(name.to_owned()), value);

                        unsafe { headers.get().as_ref().unwrap() }
                            .get(UncasedStr::new(name))
                            .map(|s| s.as_ref())
                    }
                }
            } else {
                None
            }
        }
    }

    pub fn split(&mut self) -> (&EspHttpConnection<'a>, &mut Self) {
        self.assert_request();

        let headers_ptr: *const EspHttpConnection<'a> = self as *const _;

        let headers = unsafe { headers_ptr.as_ref().unwrap() };

        (headers, self)
    }

    /// Sends the HTTP status (e.g. "200 OK") and the response headers to the
    /// HTTP client.
    pub fn initiate_response(
        &mut self,
        status: u16,
        message: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<(), EspError> {
        self.assert_request();

        let mut c_headers = Vec::new();

        let status = if let Some(message) = message {
            format!("{status} {message}")
        } else {
            status.to_string()
        };

        let c_status = to_cstring_arg(status.as_str())?;
        esp!(unsafe { httpd_resp_set_status(self.request.0, c_status.as_ptr() as _) })?;

        c_headers.push(c_status);

        for (key, value) in headers {
            if key.eq_ignore_ascii_case("Content-Type") {
                let c_type = to_cstring_arg(value)?;

                esp!(unsafe { httpd_resp_set_type(self.request.0, c_type.as_c_str().as_ptr()) })?;

                c_headers.push(c_type);
            } else if key.eq_ignore_ascii_case("Content-Length") {
                let c_len = to_cstring_arg(value)?;

                //esp!(unsafe { httpd_resp_set_len(self.raw_req, c_len.as_c_str().as_ptr()) })?;

                c_headers.push(c_len);
            } else {
                let name = to_cstring_arg(key)?;
                let value = to_cstring_arg(value)?;

                esp!(unsafe {
                    httpd_resp_set_hdr(
                        self.request.0,
                        name.as_c_str().as_ptr() as _,
                        value.as_c_str().as_ptr() as _,
                    )
                })?;

                c_headers.push(name);
                c_headers.push(value);
            }
        }

        self.response_headers = Some(c_headers);
        self.headers = None;

        Ok(())
    }

    /// Returns `true` if the response headers have been sent to the HTTP client.
    pub fn is_response_initiated(&self) -> bool {
        self.headers.is_none()
    }

    /// Reads bytes from the body of the HTTP request.
    ///
    /// This is typically used whenever the HTTP server has to parse the body
    /// of an HTTP POST request.
    ///
    /// ```ignore
    /// server.fn_handler("/foo", Method::Post, move |mut request| {
    ///     let (_headers, connection) = request.split();
    ///     let mut buffer: [u8; 1024] = [0; 1024];
    ///     let bytes_read = connection.read(&mut buffer)?;
    ///
    ///     let my_data = MyDataStruct::from_bytes(&buffer[0..bytes_read]);
    ///     // etc
    ///
    ///     Ok(())
    /// })?;
    /// ```
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, EspError> {
        self.assert_request();

        unsafe {
            let len = httpd_req_recv(self.request.0, buf.as_mut_ptr() as *mut _, buf.len());

            if len < 0 {
                esp!(len)?;
            }

            Ok(len as usize)
        }
    }

    /// Sends bytes back to the HTTP client; returns the number of bytes sent.
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, EspError> {
        self.assert_response();

        if !buf.is_empty() {
            esp!(unsafe {
                httpd_resp_send_chunk(self.request.0, buf.as_ptr().cast(), buf.len() as isize)
            })?;

            self.response_headers = None;
        }

        Ok(buf.len())
    }

    // Sends bytes back to the HTTP client (as per `EspHttpConnection::write`),
    // does *not* return the number of bytes sent.
    pub fn write_all(&mut self, buf: &[u8]) -> Result<(), EspError> {
        self.write(buf)?;

        Ok(())
    }

    pub fn raw_connection(&mut self) -> Result<&mut EspHttpRawConnection<'a>, EspError> {
        Ok(&mut self.request)
    }

    fn invoke<H>(&mut self, handler: &H) -> Result<(), H::Error>
    where
        H: Handler<Self>,
    {
        // TODO info!("About to handle query string {:?}", self.query_string());

        handler.handle(self)?;

        Ok(())
    }

    fn complete(&mut self) -> Result<(), EspError> {
        let buf = &[];

        if self.response_headers.is_some() {
            esp!(unsafe { httpd_resp_send(self.request.0, buf.as_ptr() as *const _, 0) })?;
        } else {
            esp!(unsafe { httpd_resp_send_chunk(self.request.0, buf.as_ptr() as *const _, 0) })?;
        }

        self.response_headers = None;

        Ok(())
    }

    fn handle_error<E>(&mut self, error: E)
    where
        E: Debug,
    {
        if self.headers.is_some() {
            info!("About to handle internal error [{error:?}], response not sent yet");

            if let Err(error2) = self.render_error(&error) {
                warn!(
                    "Internal error[{error2}] while rendering another internal error:\n{error:?}"
                );
            }
        } else {
            warn!("Unhandled internal error [{error:?}], response is already sent");
        }
    }

    fn render_error<E>(&mut self, error: E) -> Result<(), EspError>
    where
        E: Debug,
    {
        self.initiate_response(500, Some("Internal Error"), &[content_type("text/html")])?;

        self.write_all(
            format!(
                r#"
                    <!DOCTYPE html5>
                    <html>
                        <body style="font-family: Verdana, Sans;">
                            <h1>INTERNAL ERROR</h1>
                            <hr>
                            <pre>{error:?}</pre>
                        <body>
                    </html>
                "#
            )
            .as_bytes(),
        )?;

        Ok(())
    }

    fn assert_request(&self) {
        if self.headers.is_none() {
            panic!("connection is not in request phase");
        }
    }

    fn assert_response(&self) {
        if self.headers.is_some() {
            panic!("connection is not in response phase");
        }
    }
}

impl RawHandle for EspHttpConnection<'_> {
    type Handle = *mut httpd_req_t;

    fn handle(&self) -> Self::Handle {
        self.request.handle()
    }
}

impl embedded_svc::http::Query for EspHttpConnection<'_> {
    fn uri(&self) -> &str {
        EspHttpConnection::uri(self)
    }

    fn method(&self) -> Method {
        EspHttpConnection::method(self)
    }
}

impl embedded_svc::http::Headers for EspHttpConnection<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        EspHttpConnection::header(self, name)
    }
}

impl ErrorType for EspHttpConnection<'_> {
    type Error = EspIOError;
}

impl Read for EspHttpConnection<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        EspHttpConnection::read(self, buf).map_err(EspIOError)
    }
}

impl Write for EspHttpConnection<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        EspHttpConnection::write(self, buf).map_err(EspIOError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.assert_response();

        Ok(())
    }
}

impl<'b> Connection for EspHttpConnection<'b> {
    type Headers = Self;

    type Read = Self;

    type RawConnectionError = EspIOError;

    type RawConnection = EspHttpRawConnection<'b>;

    fn split(&mut self) -> (&Self::Headers, &mut Self::Read) {
        EspHttpConnection::split(self)
    }

    fn initiate_response<'a>(
        &'a mut self,
        status: u16,
        message: Option<&'a str>,
        headers: &'a [(&'a str, &'a str)],
    ) -> Result<(), Self::Error> {
        EspHttpConnection::initiate_response(self, status, message, headers).map_err(EspIOError)
    }

    fn is_response_initiated(&self) -> bool {
        EspHttpConnection::is_response_initiated(self)
    }

    fn raw_connection(&mut self) -> Result<&mut Self::RawConnection, Self::Error> {
        EspHttpConnection::raw_connection(self).map_err(EspIOError)
    }
}

#[cfg(esp_idf_httpd_ws_support)]
pub mod ws {
    use core::ffi;
    use core::fmt::Debug;
    use core::sync::atomic::{AtomicBool, Ordering};

    extern crate alloc;
    use alloc::boxed::Box;
    use alloc::collections::BTreeSet;
    use alloc::sync::Arc;

    use ::log::*;

    use embedded_svc::http::Method;
    use embedded_svc::ws::*;

    use crate::sys::*;

    use crate::private::common::Newtype;
    use crate::private::cstr::to_cstring_arg;
    use crate::private::mutex::{Condvar, Mutex};

    use super::EspHttpServer;
    use super::CLOSE_HANDLERS;
    use super::OPEN_SESSIONS;
    use super::{CloseHandler, NativeHandler};

    /// A Websocket connection between this server and a client.
    pub enum EspHttpWsConnection {
        New(httpd_handle_t, *mut httpd_req_t),
        Receiving(httpd_handle_t, *mut httpd_req_t, Option<httpd_ws_frame_t>),
        Closed(ffi::c_int),
    }

    impl EspHttpWsConnection {
        // Returns the internal file descriptor for the socket.
        pub fn session(&self) -> i32 {
            match self {
                Self::New(_, raw_req) | Self::Receiving(_, raw_req, _) => unsafe {
                    httpd_req_to_sockfd(*raw_req)
                },
                Self::Closed(fd) => *fd,
            }
        }

        /// Returns `true` when the connection still hasn't received any data
        pub fn is_new(&self) -> bool {
            matches!(self, Self::New(_, _))
        }

        /// Returns `true` when the connection already has been closed.
        pub fn is_closed(&self) -> bool {
            matches!(self, Self::Closed(_))
        }

        pub fn create_detached_sender(&self) -> Result<EspHttpWsDetachedSender, EspError> {
            match self {
                Self::New(sd, raw_req) | Self::Receiving(sd, raw_req, _) => {
                    let fd = unsafe { httpd_req_to_sockfd(*raw_req) };

                    let mut sessions = OPEN_SESSIONS.lock();

                    let closed = sessions
                        .entry((*sd as u32, fd))
                        .or_insert_with(|| Arc::new(AtomicBool::new(false)));

                    Ok(EspHttpWsDetachedSender::new(*sd, fd, closed.clone()))
                }
                Self::Closed(_) => Err(EspError::from_infallible::<ESP_FAIL>()),
            }
        }

        /// Sends a frame to the client.
        pub fn send(&mut self, frame_type: FrameType, frame_data: &[u8]) -> Result<(), EspError> {
            match self {
                Self::New(_, raw_req) | Self::Receiving(_, raw_req, _) => {
                    let raw_frame = Self::create_raw_frame(frame_type, frame_data);

                    esp!(unsafe {
                        httpd_ws_send_frame(*raw_req, &raw_frame as *const _ as *mut _)
                    })?;

                    Ok(())
                }
                _ => Err(EspError::from_infallible::<ESP_FAIL>()),
            }
        }

        /// Receives a frame from the client.
        pub fn recv(&mut self, frame_data_buf: &mut [u8]) -> Result<(FrameType, usize), EspError> {
            match self {
                Self::New(_, _) => Err(EspError::from_infallible::<ESP_FAIL>()),
                Self::Receiving(sd, raw_req, ref mut raw_frame_mut) => {
                    let raw_frame = loop {
                        if let Some(raw_frame) = raw_frame_mut.as_mut() {
                            break raw_frame;
                        }

                        let mut raw_frame: httpd_ws_frame_t = Default::default();

                        esp!(unsafe {
                            httpd_ws_recv_frame(*raw_req, &mut raw_frame as *mut _, 0)
                        })?;

                        if Self::create_frame_type(&raw_frame).is_none() {
                            // RFC 6455 5.2: a reserved opcode fails the connection. ESP-IDF
                            // passes the opcode through unchecked; answer with a Close frame
                            // (1002, protocol error) and close the session.
                            Self::fail_connection(*sd, *raw_req);
                        }

                        // This is necessary because the ESP IDF WS API requires us to
                        // call it exactly once with a frame that has a zero-sized buffer,
                        // and then also exactly once with the same frame instance, except
                        // its buffer set to a non-zero size
                        //
                        // On the other hand, we would like to allow the user the freedom
                        // to call the API as many times as she wants, and only consume the
                        // frame if the provided buffer is big enough
                        *raw_frame_mut = Some(raw_frame);
                    };

                    // A frame with a reserved opcode stays pending, so every later call
                    // fails too, instead of reading its payload as the next frame header
                    let Some((frame_type, len)) = Self::create_frame_type(raw_frame) else {
                        return Err(EspError::from_infallible::<ESP_ERR_INVALID_RESPONSE>());
                    };

                    if frame_data_buf.len() >= len {
                        raw_frame.payload = frame_data_buf.as_mut_ptr() as *mut _;
                        esp!(unsafe { httpd_ws_recv_frame(*raw_req, raw_frame as *mut _, len) })?;

                        *raw_frame_mut = None;
                    }

                    Ok((frame_type, len))
                }
                Self::Closed(_) => Ok((FrameType::SocketClose, 0)),
            }
        }

        #[allow(clippy::needless_update)]
        fn create_raw_frame(frame_type: FrameType, frame_data: &[u8]) -> httpd_ws_frame_t {
            httpd_ws_frame_t {
                type_: match frame_type {
                    FrameType::Text(_) => httpd_ws_type_t_HTTPD_WS_TYPE_TEXT,
                    FrameType::Binary(_) => httpd_ws_type_t_HTTPD_WS_TYPE_BINARY,
                    FrameType::Ping => httpd_ws_type_t_HTTPD_WS_TYPE_PING,
                    FrameType::Pong => httpd_ws_type_t_HTTPD_WS_TYPE_PONG,
                    FrameType::Close => httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE,
                    FrameType::Continue(_) => httpd_ws_type_t_HTTPD_WS_TYPE_CONTINUE,
                    FrameType::SocketClose => panic!("Cannot send SocketClose as a frame"),
                },
                final_: frame_type.is_final(),
                fragmented: frame_type.is_fragmented(),
                payload: frame_data.as_ptr() as *const _ as *mut _,
                len: frame_data.len(),
                ..Default::default()
            }
        }

        /// Returns `None` for a reserved opcode, which the peer controls
        #[allow(non_upper_case_globals)]
        fn create_frame_type(raw_frame: &httpd_ws_frame_t) -> Option<(FrameType, usize)> {
            Some(match raw_frame.type_ {
                httpd_ws_type_t_HTTPD_WS_TYPE_TEXT => {
                    (FrameType::Text(raw_frame.fragmented), raw_frame.len + 1)
                }
                httpd_ws_type_t_HTTPD_WS_TYPE_BINARY => {
                    (FrameType::Binary(raw_frame.fragmented), raw_frame.len)
                }
                httpd_ws_type_t_HTTPD_WS_TYPE_CONTINUE => {
                    (FrameType::Continue(raw_frame.final_), raw_frame.len)
                }
                httpd_ws_type_t_HTTPD_WS_TYPE_PING => (FrameType::Ping, 0),
                httpd_ws_type_t_HTTPD_WS_TYPE_PONG => (FrameType::Pong, 0),
                httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE => (FrameType::Close, 0),
                _ => return None,
            })
        }

        /// Sends a Close frame with status 1002 (protocol error) and closes the session.
        /// Both steps are best effort: the peer may already be gone.
        fn fail_connection(sd: httpd_handle_t, raw_req: *mut httpd_req_t) {
            let status = 1002_u16.to_be_bytes();
            let close = Self::create_raw_frame(FrameType::Close, &status);

            unsafe {
                httpd_ws_send_frame(raw_req, &close as *const _ as *mut _);
                httpd_sess_trigger_close(sd, httpd_req_to_sockfd(raw_req));
            }
        }
    }

    impl ErrorType for EspHttpWsConnection {
        type Error = EspError;
    }

    impl Sender for EspHttpWsConnection {
        fn send(&mut self, frame_type: FrameType, frame_data: &[u8]) -> Result<(), Self::Error> {
            EspHttpWsConnection::send(self, frame_type, frame_data)
        }
    }

    impl Receiver for EspHttpWsConnection {
        fn recv(&mut self, frame_data_buf: &mut [u8]) -> Result<(FrameType, usize), Self::Error> {
            EspHttpWsConnection::recv(self, frame_data_buf)
        }
    }

    struct EspWsDetachedSendRequest {
        sd: httpd_handle_t,
        fd: ffi::c_int,

        closed: Arc<AtomicBool>,

        raw_frame: *const httpd_ws_frame_t,

        error_code: Mutex<Option<u32>>,
        condvar: Condvar,
    }

    pub struct EspHttpWsDetachedSender {
        sd: httpd_handle_t,
        fd: ffi::c_int,
        closed: Arc<AtomicBool>,
    }

    impl EspHttpWsDetachedSender {
        fn new(sd: httpd_handle_t, fd: ffi::c_int, closed: Arc<AtomicBool>) -> Self {
            Self { sd, fd, closed }
        }

        pub fn session(&self) -> i32 {
            self.fd
        }

        pub fn is_new(&self) -> bool {
            false
        }

        pub fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }

        pub fn send(&mut self, frame_type: FrameType, frame_data: &[u8]) -> Result<(), EspError> {
            if !self.closed.load(Ordering::SeqCst) {
                let raw_frame = EspHttpWsConnection::create_raw_frame(frame_type, frame_data);

                let send_request = EspWsDetachedSendRequest {
                    sd: self.sd,
                    fd: self.fd,

                    closed: self.closed.clone(),

                    raw_frame: &raw_frame as *const _,

                    error_code: Mutex::new(None),
                    condvar: Condvar::new(),
                };

                esp!(unsafe {
                    httpd_queue_work(
                        self.sd,
                        Some(Self::enqueue),
                        &send_request as *const _ as *mut _,
                    )
                })?;

                let mut guard = send_request.error_code.lock();

                while guard.is_none() {
                    guard = send_request.condvar.wait(guard);
                }

                esp!((*guard).unwrap())?;
            } else {
                return Err(EspError::from_infallible::<ESP_FAIL>());
            }

            Ok(())
        }

        extern "C" fn enqueue(arg: *mut ffi::c_void) {
            let request = unsafe { (arg as *const EspWsDetachedSendRequest).as_ref().unwrap() };

            let ret = if !request.closed.load(Ordering::SeqCst) {
                unsafe {
                    httpd_ws_send_frame_async(
                        request.sd,
                        request.fd,
                        request.raw_frame as *const _ as *mut _,
                    )
                }
            } else {
                ESP_FAIL
            };

            let mut guard = request.error_code.lock();

            *guard = Some(ret as _);

            request.condvar.notify_all();
        }
    }

    unsafe impl Send for EspHttpWsDetachedSender {}

    impl Clone for EspHttpWsDetachedSender {
        fn clone(&self) -> Self {
            Self {
                sd: self.sd,
                fd: self.fd,
                closed: self.closed.clone(),
            }
        }
    }

    impl ErrorType for EspHttpWsDetachedSender {
        type Error = EspError;
    }

    impl Sender for EspHttpWsDetachedSender {
        fn send(&mut self, frame_type: FrameType, frame_data: &[u8]) -> Result<(), Self::Error> {
            EspHttpWsDetachedSender::send(self, frame_type, frame_data)
        }
    }

    impl<'a> EspHttpServer<'a> {
        /// Registers a function as the handler for a Websockets URI.
        ///
        /// The function will be called every time a Websockets connection is
        /// made to that URI, receiving a different `EspHttpWsConnection` each
        /// call.
        ///
        /// # Arguments
        ///
        /// * `uri` - The URI to connect to
        /// * `subprotocol_list` - An optional string slice containing a comma-separated
        ///   list of subprotocols to be supported by this handler
        /// * handler: the handler
        ///
        /// Note that Websockets functionality is gated behind an SDK flag.
        /// See [`crate::ws`](esp-idf-svc::ws)
        pub fn ws_handler<H, E>(
            &mut self,
            uri: &str,
            subprotocol_list: Option<&str>,
            handler: H,
        ) -> Result<&mut Self, EspError>
        where
            H: for<'r> Fn(&'r mut EspHttpWsConnection) -> Result<(), E> + Send + Sync + 'a,
            E: Debug,
        {
            let uri_c_str = to_cstring_arg(uri)?;

            let (req_handler, close_handler) = self.to_native_ws_handler(self.sd, handler);

            let mut conf = httpd_uri_t {
                uri: uri_c_str.as_ptr() as _,
                method: Newtype::<ffi::c_uint>::from(Method::Get).0,
                user_ctx: Box::into_raw(Box::new(req_handler)) as *mut _,
                handler: Some(EspHttpServer::handle_req),
                is_websocket: true,
                // Since ESP-IDF 5.5.5 / 6.0.1, the handler is no longer called for the handshake
                #[cfg(esp_idf_httpd_ws_post_handshake_cb_support)]
                ws_post_handshake_cb: Some(EspHttpServer::handle_req),
                // TODO: Expose as a parameter in future: handle_ws_control_frames: true,
                ..Default::default()
            };

            #[cfg(all(
                any(
                    esp_idf_version_patch_at_least_5_5_5,
                    esp_idf_version_patch_at_least_6_0_1,
                    esp_idf_version_at_least_6_1_0
                ),
                not(esp_idf_httpd_ws_post_handshake_cb_support)
            ))]
            warn!(
                "WS handshakes for URI \"{}\" will not be reported to the handler; \
                 enable `CONFIG_HTTPD_WS_POST_HANDSHAKE_CB_SUPPORT` for that",
                uri_c_str.to_str().unwrap()
            );

            let subproto_c_str; // SAFETY: same scope as httpd_register_uri_handler required!
            if let Some(subprotocol_list) = subprotocol_list {
                subproto_c_str = to_cstring_arg(subprotocol_list)?;
                conf.supported_subprotocol = subproto_c_str.as_ptr();
            }

            esp!(unsafe { crate::sys::httpd_register_uri_handler(self.sd, &conf) })?;

            {
                let mut all_close_handlers = CLOSE_HANDLERS.lock();

                let close_handlers = all_close_handlers.get_mut(&(self.sd as u32)).unwrap();

                let close_handler: CloseHandler<'static> =
                    unsafe { core::mem::transmute(close_handler) };

                close_handlers.push(close_handler);
            }

            info!(
                "Registered Httpd server WS handler for URI \"{}\"",
                uri_c_str.to_str().unwrap()
            );

            self.registrations.push((uri_c_str, conf));

            Ok(self)
        }

        fn handle_ws_request<H, E>(
            connection: &mut EspHttpWsConnection,
            handler: &H,
        ) -> Result<(), E>
        where
            H: for<'b> Fn(&'b mut EspHttpWsConnection) -> Result<(), E> + Send + 'a,
            E: Debug,
        {
            handler(connection)?;

            Ok(())
        }

        fn handle_ws_error<E>(error: E) -> ffi::c_int
        where
            E: Debug,
        {
            warn!("Unhandled internal error [{error:?}]:\n{error:?}");

            ESP_OK as _
        }

        fn to_native_ws_handler<H, E>(
            &self,
            server_handle: httpd_handle_t,
            handler: H,
        ) -> (NativeHandler<'a>, CloseHandler<'a>)
        where
            H: for<'r> Fn(&'r mut EspHttpWsConnection) -> Result<(), E> + Send + Sync + 'a,
            E: Debug,
        {
            let boxed_handler = Arc::new(move |mut connection: EspHttpWsConnection| {
                let result = Self::handle_ws_request(&mut connection, &handler);

                match result {
                    Ok(()) => ESP_OK as _,
                    Err(e) => Self::handle_ws_error(e),
                }
            });

            // The sockets of the sessions of this handler: the server reports every closed
            // socket - including plain HTTP ones and the ones of other WS handlers - while only
            // the closing of its own sessions is to be reported to the handler
            let sessions = Arc::new(Mutex::new(BTreeSet::new()));

            let req_handler = {
                let boxed_handler = boxed_handler.clone();
                let sessions = sessions.clone();

                Box::new(move |raw_req: *mut httpd_req_t| {
                    let req = unsafe { raw_req.as_ref() }.unwrap();

                    // A session is recorded on its handshake, or - when handshakes are not
                    // reported to the handler - when it receives its first frame
                    sessions
                        .lock()
                        .insert(unsafe { httpd_req_to_sockfd(raw_req) });

                    (boxed_handler)(if req.method == http_method_HTTP_GET as i32 {
                        EspHttpWsConnection::New(server_handle, raw_req)
                    } else {
                        EspHttpWsConnection::Receiving(server_handle, raw_req, None)
                    })
                })
            };

            let close_handler = Box::new(move |fd| {
                if sessions.lock().remove(&fd) {
                    (boxed_handler)(EspHttpWsConnection::Closed(fd));
                }
            });

            (req_handler, close_handler)
        }
    }

    // TODO: Consider if it makes sense at all to put a complex async layer on top of the ESP-IDF WS API,
    // which is very far from being async
    // TODO: Port all of the code below to `zerocopy`, thus simplifying it and providing blocking
    // sender/receiver/acceptor implementations as well

    // enum ReceiverData {
    //     None,
    //     Metadata((FrameType, usize)),
    //     Data(*mut u8),
    //     DataCopied,
    //     Closed,
    // }

    // unsafe impl Send for ReceiverData {}

    // struct SharedReceiverState {
    //     waker: Option<Waker>,
    //     data: ReceiverData,
    // }

    // struct ConnectionState {
    //     session: ffi::c_int,
    //     receiver_state: Arc<Mutex<SharedReceiverState>>,
    // }

    // pub struct SharedAcceptorState {
    //     waker: Option<Waker>,
    //     data: Option<Option<(Arc<Mutex<SharedReceiverState>>, EspHttpWsDetachedSender)>>,
    // }

    // pub struct EspHttpWsAsyncSender<U> {
    //     unblocker: U,
    //     sender: EspHttpWsDetachedSender,
    // }

    // impl<U> EspHttpWsAsyncSender<U>
    // where
    //     U: Unblocker,
    // {
    //     pub async fn send(
    //         &mut self,
    //         frame_type: FrameType,
    //         frame_data: &[u8],
    //     ) -> Result<(), EspError> {
    //         #[cfg(not(feature = "std"))]
    //         use alloc::borrow::ToOwned;

    //         debug!(
    //             "Sending data (frame_type={:?}, frame_len={}) to WS connection {:?}",
    //             frame_type,
    //             frame_data.len(),
    //             self.sender.session()
    //         );

    //         let mut sender = self.sender.clone();
    //         let frame_data: alloc::vec::Vec<u8> = frame_data.to_owned();

    //         self.unblocker
    //             .unblock(move || sender.send(frame_type, &frame_data))
    //             .await
    //     }
    // }

    // impl<U> ErrorType for EspHttpWsAsyncSender<U> {
    //     type Error = EspError;
    // }

    // impl<U> asynch::Sender for EspHttpWsAsyncSender<U>
    // where
    //     U: Unblocker,
    // {
    //     async fn send(
    //         &mut self,
    //         frame_type: FrameType,
    //         frame_data: &[u8],
    //     ) -> Result<(), Self::Error> {
    //         EspHttpWsAsyncSender::send(self, frame_type, frame_data).await
    //     }
    // }

    // pub struct EspHttpWsAsyncReceiver {
    //     shared: Arc<Mutex<SharedReceiverState>>,
    //     condvar: Arc<Condvar>,
    // }

    // impl EspHttpWsAsyncReceiver {
    //     pub async fn recv(
    //         &mut self,
    //         frame_data_buf: &mut [u8],
    //     ) -> Result<(FrameType, usize), EspError> {
    //         AsyncReceiverFuture {
    //             receiver: self,
    //             frame_data_buf,
    //         }
    //         .await
    //     }
    // }

    // struct AsyncReceiverFuture<'a> {
    //     receiver: &'a mut EspHttpWsAsyncReceiver,
    //     frame_data_buf: &'a mut [u8],
    // }

    // impl<'a> Future for AsyncReceiverFuture<'a> {
    //     type Output = Result<(FrameType, usize), EspError>;

    //     fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    //         let frame_data_buf_ptr = self.frame_data_buf.as_mut_ptr();
    //         let mut shared = self.receiver.shared.lock();

    //         if let ReceiverData::Metadata((frame_type, size)) = shared.data {
    //             if self.frame_data_buf.len() >= size {
    //                 shared.data = ReceiverData::Data(frame_data_buf_ptr);

    //                 self.receiver.condvar.notify_all();

    //                 while !matches!(shared.data, ReceiverData::DataCopied) {
    //                     shared = self.receiver.condvar.wait(shared);
    //                 }

    //                 shared.data = ReceiverData::None;
    //                 self.receiver.condvar.notify_all();
    //             }

    //             Poll::Ready(Ok((frame_type, size)))
    //         } else if let ReceiverData::Closed = shared.data {
    //             Poll::Ready(Ok((FrameType::Close, 0)))
    //         } else {
    //             shared.waker = Some(cx.waker().clone());
    //             Poll::Pending
    //         }
    //     }
    // }

    // impl ErrorType for EspHttpWsAsyncReceiver {
    //     type Error = EspError;
    // }

    // impl asynch::Receiver for EspHttpWsAsyncReceiver {
    //     async fn recv(
    //         &mut self,
    //         frame_data_buf: &mut [u8],
    //     ) -> Result<(FrameType, usize), Self::Error> {
    //         EspHttpWsAsyncReceiver::recv(self, frame_data_buf).await
    //     }
    // }

    // pub struct EspHttpWsAsyncAcceptor<U> {
    //     unblocker: U,
    //     accept: Arc<Mutex<SharedAcceptorState>>,
    //     condvar: Arc<Condvar>,
    // }

    // impl<U> EspHttpWsAsyncAcceptor<U> {
    //     pub fn accept(&self) -> &EspHttpWsAsyncAcceptor<U> {
    //         self
    //     }
    // }

    // impl<'a, U> Future for &'a EspHttpWsAsyncAcceptor<U>
    // where
    //     U: Clone,
    // {
    //     type Output = Result<(EspHttpWsAsyncSender<U>, EspHttpWsAsyncReceiver), EspError>;

    //     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    //         let mut accept = self.accept.lock();

    //         match accept.data.take() {
    //             Some(Some((shared, sender))) => {
    //                 let sender = EspHttpWsAsyncSender {
    //                     unblocker: self.unblocker.clone(),
    //                     sender,
    //                 };

    //                 let receiver = EspHttpWsAsyncReceiver {
    //                     shared,
    //                     condvar: self.condvar.clone(),
    //                 };

    //                 self.condvar.notify_all();

    //                 Poll::Ready(Ok((sender, receiver)))
    //             }
    //             Some(None) => {
    //                 accept.data = Some(None);
    //                 Poll::Pending
    //             }
    //             None => {
    //                 accept.waker = Some(cx.waker().clone());
    //                 Poll::Pending
    //             }
    //         }
    //     }
    // }

    // impl<U> ErrorType for EspHttpWsAsyncAcceptor<U> {
    //     type Error = EspError;
    // }

    // impl<U> asynch::server::Acceptor for EspHttpWsAsyncAcceptor<U>
    // where
    //     U: Unblocker + Clone + Send,
    // {
    //     type Sender<'a> = EspHttpWsAsyncSender<U> where U: 'a;
    //     type Receiver<'a> = EspHttpWsAsyncReceiver where U: 'a;

    //     async fn accept(&self) -> Result<(Self::Sender<'_>, Self::Receiver<'_>), Self::Error> {
    //         self.await
    //     }
    // }

    // #[allow(clippy::type_complexity)]
    // pub struct EspHttpWsProcessor<const N: usize> {
    //     connections: alloc::vec::Vec<ConnectionState>,
    //     frame_data_buf: [u8; N],
    //     accept: Arc<Mutex<SharedAcceptorState>>,
    //     condvar: Arc<Condvar>,
    // }

    // impl<const N: usize> EspHttpWsProcessor<N> {
    //     pub fn new<U>(unblocker: U) -> (Self, EspHttpWsAsyncAcceptor<U>) {
    //         let this = Self {
    //             connections: alloc::vec::Vec::new(),
    //             frame_data_buf: [0_u8; N],
    //             accept: Arc::new(Mutex::new(SharedAcceptorState {
    //                 waker: None,
    //                 data: None,
    //             })),
    //             condvar: Arc::new(Condvar::new()),
    //         };

    //         let acceptor = EspHttpWsAsyncAcceptor {
    //             unblocker,
    //             accept: this.accept.clone(),
    //             condvar: this.condvar.clone(),
    //         };

    //         (this, acceptor)
    //     }

    //     pub fn process(&mut self, connection: &mut EspHttpWsConnection) -> Result<(), EspError> {
    //         if connection.is_new() {
    //             let session = connection.session();

    //             info!("New WS connection {:?}", session);

    //             self.process_accept(session, connection)?;
    //         } else if connection.is_closed() {
    //             let session = connection.session();

    //             if let Some(index) = self
    //                 .connections
    //                 .iter()
    //                 .enumerate()
    //                 .find_map(|(index, conn)| (conn.session == session).then_some(index))
    //             {
    //                 let conn = self.connections.swap_remove(index);

    //                 Self::process_receive_close(&conn.receiver_state);
    //                 info!("Closed WS connection {:?}", session);
    //             }
    //         } else {
    //             let session = connection.session();
    //             let (frame_type, len) = connection.recv(&mut self.frame_data_buf)?;

    //             debug!(
    //                 "Incoming data (frame_type={:?}, frame_len={}) from WS connection {:?}",
    //                 frame_type, len, session
    //             );

    //             if let Some(connection) = self
    //                 .connections
    //                 .iter()
    //                 .find(|connection| connection.session == session)
    //             {
    //                 self.process_receive(&connection.receiver_state, frame_type, len)
    //             }
    //         }

    //         Ok(())
    //     }

    //     fn process_accept(
    //         &mut self,
    //         session: ffi::c_int,
    //         sender: &EspHttpWsConnection,
    //     ) -> Result<(), EspError> {
    //         let receiver_state = Arc::new(Mutex::new(SharedReceiverState {
    //             waker: None,
    //             data: ReceiverData::None,
    //         }));

    //         let state = ConnectionState {
    //             session,
    //             receiver_state: receiver_state.clone(),
    //         };

    //         self.connections.push(state);

    //         let sender = sender.create_detached_sender()?;

    //         let mut accept = self.accept.lock();

    //         accept.data = Some(Some((receiver_state, sender)));

    //         if let Some(waker) = accept.waker.take() {
    //             waker.wake();
    //         }

    //         while accept.data.is_some() {
    //             accept = self.condvar.wait(accept);
    //         }

    //         Ok(())
    //     }

    //     fn process_receive(
    //         &self,
    //         state: &Mutex<SharedReceiverState>,
    //         frame_type: FrameType,
    //         len: usize,
    //     ) {
    //         let mut shared = state.lock();

    //         shared.data = ReceiverData::Metadata((frame_type, len));

    //         if let Some(waker) = shared.waker.take() {
    //             waker.wake();
    //         }

    //         loop {
    //             if let ReceiverData::Data(buf) = &shared.data {
    //                 unsafe { slice::from_raw_parts_mut(*buf, len) }
    //                     .copy_from_slice(&self.frame_data_buf[..len]);
    //                 shared.data = ReceiverData::DataCopied;
    //                 self.condvar.notify_all();

    //                 break;
    //             }

    //             shared = self.condvar.wait(shared);
    //         }

    //         while !matches!(shared.data, ReceiverData::None) {
    //             shared = self.condvar.wait(shared);
    //         }
    //     }

    //     fn process_accept_close(&mut self) {
    //         let mut accept = self.accept.lock();

    //         accept.data = Some(None);

    //         if let Some(waker) = accept.waker.take() {
    //             waker.wake();
    //         }
    //     }

    //     fn process_receive_close(state: &Mutex<SharedReceiverState>) {
    //         let mut shared = state.lock();

    //         shared.data = ReceiverData::Closed;

    //         if let Some(waker) = shared.waker.take() {
    //             waker.wake();
    //         }
    //     }
    // }

    // impl<const N: usize> Drop for EspHttpWsProcessor<N> {
    //     fn drop(&mut self) {
    //         self.process_accept_close();
    //     }
    // }
}
