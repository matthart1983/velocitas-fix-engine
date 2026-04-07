use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fmt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const DRIVER_TIMEOUT_MS: i32 = -1000;
const CLIENT_TIMEOUT_MS: i32 = -1001;
const CONDUCTOR_TIMEOUT_MS: i32 = -1002;
const CLIENT_BUFFER_FULL: i32 = -1003;
const PUBLICATION_BACK_PRESSURED: i32 = -2;
const PUBLICATION_ADMIN_ACTION: i32 = -3;
const PUBLICATION_CLOSED: i32 = -4;
const PUBLICATION_MAX_POSITION_EXCEEDED: i32 = -5;
const PUBLICATION_ERROR: i32 = -6;
const SYNTHETIC_TIMEOUT: i32 = -234324;
const DRIVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const DRIVER_LIVENESS_TIMEOUT_NS: u64 = 5_000_000_000;

static DRIVER_REGISTRY: OnceLock<Mutex<HashMap<String, Weak<EmbeddedMediaDriver>>>> =
    OnceLock::new();

fn driver_registry() -> &'static Mutex<HashMap<String, Weak<EmbeddedMediaDriver>>> {
    DRIVER_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeronErrorKind {
    GenericError,
    ClientErrorDriverTimeout,
    ClientErrorClientTimeout,
    ClientErrorConductorServiceTimeout,
    ClientErrorBufferFull,
    PublicationBackPressured,
    PublicationAdminAction,
    PublicationClosed,
    PublicationMaxPositionExceeded,
    PublicationError,
    TimedOut,
    Unknown(i32),
}

impl AeronErrorKind {
    pub fn from_code(code: i32) -> Self {
        match code {
            -1 => Self::GenericError,
            DRIVER_TIMEOUT_MS => Self::ClientErrorDriverTimeout,
            CLIENT_TIMEOUT_MS => Self::ClientErrorClientTimeout,
            CONDUCTOR_TIMEOUT_MS => Self::ClientErrorConductorServiceTimeout,
            CLIENT_BUFFER_FULL => Self::ClientErrorBufferFull,
            PUBLICATION_BACK_PRESSURED => Self::PublicationBackPressured,
            PUBLICATION_ADMIN_ACTION => Self::PublicationAdminAction,
            PUBLICATION_CLOSED => Self::PublicationClosed,
            PUBLICATION_MAX_POSITION_EXCEEDED => Self::PublicationMaxPositionExceeded,
            PUBLICATION_ERROR => Self::PublicationError,
            SYNTHETIC_TIMEOUT => Self::TimedOut,
            other => Self::Unknown(other),
        }
    }

    pub fn is_back_pressured(self) -> bool {
        matches!(self, Self::PublicationBackPressured)
    }

    pub fn is_admin_action(self) -> bool {
        matches!(self, Self::PublicationAdminAction)
    }

    pub fn is_back_pressured_or_admin_action(self) -> bool {
        self.is_back_pressured() || self.is_admin_action()
    }
}

impl fmt::Display for AeronErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::GenericError => "Generic Error",
            Self::ClientErrorDriverTimeout => "Client Error Driver Timeout",
            Self::ClientErrorClientTimeout => "Client Error Client Timeout",
            Self::ClientErrorConductorServiceTimeout => "Client Error Conductor Service Timeout",
            Self::ClientErrorBufferFull => "Client Error Buffer Full",
            Self::PublicationBackPressured => "Publication Back Pressured",
            Self::PublicationAdminAction => "Publication Admin Action",
            Self::PublicationClosed => "Publication Closed",
            Self::PublicationMaxPositionExceeded => "Publication Max Position Exceeded",
            Self::PublicationError => "Publication Error",
            Self::TimedOut => "Timed Out",
            Self::Unknown(_) => "Unknown Error",
        };
        write!(f, "{label}")
    }
}

#[derive(Debug, Clone)]
pub struct AeronError {
    pub code: i32,
    context: String,
    message: String,
}

impl AeronError {
    pub fn kind(&self) -> AeronErrorKind {
        AeronErrorKind::from_code(self.code)
    }

    pub fn from_code(context: impl Into<String>, code: i32) -> Self {
        let kind = AeronErrorKind::from_code(code);
        Self {
            code,
            context: context.into(),
            message: kind.to_string(),
        }
    }

    fn custom(context: impl Into<String>, code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            context: context.into(),
            message: message.into(),
        }
    }

    fn last(context: impl Into<String>) -> Self {
        let context = context.into();
        let code = unsafe { aeron_errcode() };
        let message = unsafe {
            let msg_ptr = aeron_errmsg();
            if msg_ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(msg_ptr).to_string_lossy().into_owned()
            }
        };

        if message.is_empty() {
            Self::from_code(context, if code == 0 { -1 } else { code })
        } else {
            Self::custom(context, if code == 0 { -1 } else { code }, message)
        }
    }

    pub fn timed_out(context: impl Into<String>) -> Self {
        Self::from_code(context, SYNTHETIC_TIMEOUT)
    }
}

impl fmt::Display for AeronError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.context, self.message, self.code)
    }
}

impl std::error::Error for AeronError {}

fn check_result(result: c_int, context: &str) -> Result<(), AeronError> {
    if result < 0 {
        Err(AeronError::last(context))
    } else {
        Ok(())
    }
}

fn make_c_string(value: &str, context: &str) -> Result<CString, AeronError> {
    CString::new(value)
        .map_err(|_| AeronError::custom(context, -1, "value contains interior NUL byte"))
}

struct ClientContext(*mut aeron_context_t);

impl ClientContext {
    fn new() -> Result<Self, AeronError> {
        let mut context = ptr::null_mut();
        check_result(
            unsafe { aeron_context_init(&mut context) },
            "failed to create Aeron client context",
        )?;
        Ok(Self(context))
    }

    fn set_dir(&self, dir: &CStr) -> Result<(), AeronError> {
        check_result(
            unsafe { aeron_context_set_dir(self.0, dir.as_ptr()) },
            "failed to configure Aeron client directory",
        )
    }
}

impl Drop for ClientContext {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { aeron_context_close(self.0) };
        }
    }
}

struct DriverContext(*mut aeron_driver_context_t);

impl DriverContext {
    fn new() -> Result<Self, AeronError> {
        let mut context = ptr::null_mut();
        check_result(
            unsafe { aeron_driver_context_init(&mut context) },
            "failed to create Aeron driver context",
        )?;
        Ok(Self(context))
    }

    fn set_dir(&self, dir: &CStr) -> Result<(), AeronError> {
        check_result(
            unsafe { aeron_driver_context_set_dir(self.0, dir.as_ptr()) },
            "failed to configure Aeron driver directory",
        )
    }

    fn set_bool(
        &self,
        setter: unsafe extern "C" fn(*mut aeron_driver_context_t, bool) -> c_int,
        value: bool,
        context: &str,
    ) -> Result<(), AeronError> {
        check_result(unsafe { setter(self.0, value) }, context)
    }

    fn set_size(
        &self,
        setter: unsafe extern "C" fn(*mut aeron_driver_context_t, usize) -> c_int,
        value: usize,
        context: &str,
    ) -> Result<(), AeronError> {
        check_result(unsafe { setter(self.0, value) }, context)
    }

    fn set_u64(
        &self,
        setter: unsafe extern "C" fn(*mut aeron_driver_context_t, u64) -> c_int,
        value: u64,
        context: &str,
    ) -> Result<(), AeronError> {
        check_result(unsafe { setter(self.0, value) }, context)
    }

    fn set_threading_mode(&self, mode: aeron_threading_mode_t) -> Result<(), AeronError> {
        check_result(
            unsafe { aeron_driver_context_set_threading_mode(self.0, mode) },
            "failed to configure Aeron driver threading mode",
        )
    }

    fn set_idle_strategy(&self, value: &CStr) -> Result<(), AeronError> {
        check_result(
            unsafe { aeron_driver_context_set_shared_idle_strategy(self.0, value.as_ptr()) },
            "failed to configure Aeron driver idle strategy",
        )
    }
}

impl Drop for DriverContext {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { aeron_driver_context_close(self.0) };
        }
    }
}

struct Driver(*mut aeron_driver_t);

impl Driver {
    fn new(context: &mut DriverContext) -> Result<Self, AeronError> {
        let mut driver = ptr::null_mut();
        check_result(
            unsafe { aeron_driver_init(&mut driver, context.0) },
            "failed to initialize Aeron media driver",
        )?;
        context.0 = ptr::null_mut();
        Ok(Self(driver))
    }

    fn start(&self, manual_main_loop: bool) -> Result<(), AeronError> {
        check_result(
            unsafe { aeron_driver_start(self.0, manual_main_loop) },
            "failed to start Aeron media driver",
        )
    }

    fn main_do_work(&self) -> Result<c_int, AeronError> {
        let work = unsafe { aeron_driver_main_do_work(self.0) };
        if work < 0 {
            Err(AeronError::last(
                "failed during Aeron media driver work cycle",
            ))
        } else {
            Ok(work)
        }
    }

    fn main_idle_strategy(&self, work_count: c_int) {
        unsafe { aeron_driver_main_idle_strategy(self.0, work_count) };
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { aeron_driver_close(self.0) };
        }
    }
}

struct ClientInner {
    ptr: *mut aeron_t,
}

unsafe impl Send for ClientInner {}
unsafe impl Sync for ClientInner {}

impl Drop for ClientInner {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            let _ = unsafe { aeron_close(self.ptr) };
        }
    }
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

impl Client {
    pub fn connect(dir: Option<&str>) -> Result<Self, AeronError> {
        let mut context = ClientContext::new()?;
        if let Some(dir) = dir {
            let dir = make_c_string(dir, "failed to encode Aeron client directory")?;
            context.set_dir(&dir)?;
        }

        let mut client = ptr::null_mut();
        check_result(
            unsafe { aeron_init(&mut client, context.0) },
            "failed to initialize Aeron client",
        )?;
        context.0 = ptr::null_mut();

        if let Err(error) = check_result(
            unsafe { aeron_start(client) },
            "failed to start Aeron client",
        ) {
            let _ = unsafe { aeron_close(client) };
            return Err(error);
        }

        Ok(Self {
            inner: Arc::new(ClientInner { ptr: client }),
        })
    }

    pub fn add_publication(
        &self,
        channel: &str,
        stream_id: i32,
        timeout: Duration,
    ) -> Result<Publication, AeronError> {
        let channel = make_c_string(channel, "failed to encode Aeron publication channel")?;
        let mut async_cmd: *mut aeron_async_add_publication_t = ptr::null_mut();
        check_result(
            unsafe {
                aeron_async_add_publication(
                    &mut async_cmd,
                    self.inner.ptr,
                    channel.as_ptr(),
                    stream_id,
                )
            },
            "failed to request Aeron publication",
        )?;

        let start = Instant::now();
        loop {
            let mut publication = ptr::null_mut();
            match unsafe { aeron_async_add_publication_poll(&mut publication, async_cmd) } {
                1 => {
                    return Ok(Publication {
                        inner: Arc::new(PublicationInner {
                            ptr: publication,
                            _client: self.clone(),
                        }),
                    });
                }
                0 => {
                    if start.elapsed() >= timeout {
                        unsafe { aeron_async_cmd_free(async_cmd.cast()) };
                        return Err(AeronError::timed_out(
                            "timed out creating Aeron publication",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    return Err(AeronError::last(
                        "failed while polling Aeron publication creation",
                    ))
                }
            }
        }
    }

    pub fn add_subscription(
        &self,
        channel: &str,
        stream_id: i32,
        timeout: Duration,
    ) -> Result<Subscription, AeronError> {
        let channel = make_c_string(channel, "failed to encode Aeron subscription channel")?;
        let mut async_cmd: *mut aeron_async_add_subscription_t = ptr::null_mut();
        check_result(
            unsafe {
                aeron_async_add_subscription(
                    &mut async_cmd,
                    self.inner.ptr,
                    channel.as_ptr(),
                    stream_id,
                    None,
                    ptr::null_mut(),
                    None,
                    ptr::null_mut(),
                )
            },
            "failed to request Aeron subscription",
        )?;

        let start = Instant::now();
        loop {
            let mut subscription = ptr::null_mut();
            match unsafe { aeron_async_add_subscription_poll(&mut subscription, async_cmd) } {
                1 => {
                    return Ok(Subscription {
                        inner: Arc::new(SubscriptionInner {
                            ptr: subscription,
                            _client: self.clone(),
                        }),
                    });
                }
                0 => {
                    if start.elapsed() >= timeout {
                        unsafe { aeron_async_cmd_free(async_cmd.cast()) };
                        return Err(AeronError::timed_out(
                            "timed out creating Aeron subscription",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    return Err(AeronError::last(
                        "failed while polling Aeron subscription creation",
                    ))
                }
            }
        }
    }
}

struct PublicationInner {
    ptr: *mut aeron_publication_t,
    _client: Client,
}

unsafe impl Send for PublicationInner {}
unsafe impl Sync for PublicationInner {}

impl Drop for PublicationInner {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            let _ = unsafe { aeron_publication_close(self.ptr, None, ptr::null_mut()) };
        }
    }
}

#[derive(Clone)]
pub struct Publication {
    inner: Arc<PublicationInner>,
}

impl Publication {
    pub fn offer(&self, buffer: &[u8]) -> i64 {
        unsafe {
            aeron_publication_offer(
                self.inner.ptr,
                buffer.as_ptr(),
                buffer.len(),
                None,
                ptr::null_mut(),
            )
        }
    }
}

struct SubscriptionInner {
    ptr: *mut aeron_subscription_t,
    _client: Client,
}

unsafe impl Send for SubscriptionInner {}
unsafe impl Sync for SubscriptionInner {}

impl Drop for SubscriptionInner {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            let _ = unsafe { aeron_subscription_close(self.ptr, None, ptr::null_mut()) };
        }
    }
}

#[derive(Clone)]
pub struct Subscription {
    inner: Arc<SubscriptionInner>,
}

impl Subscription {
    pub fn poll_once<F: FnMut(&[u8])>(
        &self,
        fragment_limit: usize,
        mut handler: F,
    ) -> Result<i32, AeronError> {
        unsafe extern "C" fn fragment_handler<F: FnMut(&[u8])>(
            clientd: *mut c_void,
            buffer: *const u8,
            length: usize,
            _header: *mut aeron_header_t,
        ) {
            let handler = unsafe { &mut *(clientd.cast::<F>()) };
            let msg = unsafe { std::slice::from_raw_parts(buffer, length) };
            handler(msg);
        }

        let result = unsafe {
            aeron_subscription_poll(
                self.inner.ptr,
                Some(fragment_handler::<F>),
                (&mut handler as *mut F).cast(),
                fragment_limit,
            )
        };

        if result < 0 {
            Err(AeronError::last("failed to poll Aeron subscription"))
        } else {
            Ok(result)
        }
    }
}

pub struct EmbeddedMediaDriver {
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<Result<(), AeronError>>>>,
}

impl EmbeddedMediaDriver {
    pub fn shared(dir: &str, delete_dir_on_lifecycle: bool) -> Result<Arc<Self>, AeronError> {
        let mut registry = driver_registry().lock().unwrap();
        if let Some(existing) = registry.get(dir).and_then(Weak::upgrade) {
            return Ok(existing);
        }

        let dir_owned = dir.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_copy = stop.clone();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let startup_tx = Some(startup_tx);
            let result =
                run_embedded_driver(&dir_owned, delete_dir_on_lifecycle).and_then(|driver| {
                    if let Some(tx) = startup_tx.as_ref() {
                        let _ = tx.send(Ok(()));
                    }

                    while !stop_copy.load(Ordering::Acquire) {
                        let work_count = driver.main_do_work()?;
                        driver.main_idle_strategy(work_count);
                    }

                    Ok(())
                });

            if let Err(error) = &result {
                if let Some(tx) = startup_tx.as_ref() {
                    let _ = tx.send(Err(error.clone()));
                }
            }

            result
        });

        match startup_rx.recv_timeout(DRIVER_STARTUP_TIMEOUT) {
            Ok(Ok(())) => {
                let driver = Arc::new(Self {
                    stop,
                    handle: Mutex::new(Some(handle)),
                });
                registry.insert(dir.to_string(), Arc::downgrade(&driver));
                Ok(driver)
            }
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(AeronError::timed_out(
                    "timed out starting embedded Aeron media driver",
                ))
            }
        }
    }
}

impl Drop for EmbeddedMediaDriver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

fn run_embedded_driver(dir: &str, delete_dir_on_lifecycle: bool) -> Result<Driver, AeronError> {
    let mut context = DriverContext::new()?;
    let dir = make_c_string(dir, "failed to encode Aeron driver directory")?;
    let sleep_ns = CString::new("sleep-ns").unwrap();

    context.set_dir(&dir)?;
    context.set_bool(
        aeron_driver_context_set_dir_delete_on_start,
        delete_dir_on_lifecycle,
        "failed to configure Aeron dir cleanup on start",
    )?;
    context.set_bool(
        aeron_driver_context_set_dir_delete_on_shutdown,
        delete_dir_on_lifecycle,
        "failed to configure Aeron dir cleanup on shutdown",
    )?;
    context.set_threading_mode(aeron_threading_mode_t::AERON_THREADING_MODE_SHARED)?;
    context.set_idle_strategy(&sleep_ns)?;
    context.set_bool(
        aeron_driver_context_set_term_buffer_sparse_file,
        true,
        "failed to enable sparse Aeron term buffers",
    )?;
    context.set_size(
        aeron_driver_context_set_term_buffer_length,
        64 * 1024,
        "failed to configure Aeron term buffer length",
    )?;
    context.set_size(
        aeron_driver_context_set_ipc_term_buffer_length,
        64 * 1024,
        "failed to configure Aeron IPC term buffer length",
    )?;
    context.set_u64(
        aeron_driver_context_set_timer_interval_ns,
        DRIVER_LIVENESS_TIMEOUT_NS / 100,
        "failed to configure Aeron driver timer interval",
    )?;
    context.set_u64(
        aeron_driver_context_set_client_liveness_timeout_ns,
        DRIVER_LIVENESS_TIMEOUT_NS,
        "failed to configure Aeron client liveness timeout",
    )?;
    context.set_u64(
        aeron_driver_context_set_publication_linger_timeout_ns,
        DRIVER_LIVENESS_TIMEOUT_NS / 10,
        "failed to configure Aeron publication linger timeout",
    )?;
    context.set_u64(
        aeron_driver_context_set_image_liveness_timeout_ns,
        DRIVER_LIVENESS_TIMEOUT_NS / 10,
        "failed to configure Aeron image liveness timeout",
    )?;
    context.set_bool(
        aeron_driver_context_set_enable_experimental_features,
        true,
        "failed to enable Aeron experimental features",
    )?;

    let driver = Driver::new(&mut context)?;
    driver.start(true)?;
    Ok(driver)
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_context_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_publication_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_subscription_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_header_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_client_registering_resource_stct {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
type aeron_async_add_publication_t = aeron_client_registering_resource_stct;

#[allow(non_camel_case_types)]
type aeron_async_add_subscription_t = aeron_client_registering_resource_stct;

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_driver_context_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct aeron_driver_t {
    _private: [u8; 0],
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum aeron_threading_mode_t {
    AERON_THREADING_MODE_DEDICATED = 0,
    AERON_THREADING_MODE_SHARED_NETWORK = 1,
    AERON_THREADING_MODE_SHARED = 2,
    AERON_THREADING_MODE_INVOKER = 3,
}

#[allow(non_camel_case_types)]
type aeron_fragment_handler_t = Option<
    unsafe extern "C" fn(
        clientd: *mut c_void,
        buffer: *const u8,
        length: usize,
        header: *mut aeron_header_t,
    ),
>;
#[allow(non_camel_case_types)]
type aeron_notification_t = Option<unsafe extern "C" fn(clientd: *mut c_void)>;
#[allow(non_camel_case_types)]
type aeron_reserved_value_supplier_t = Option<
    unsafe extern "C" fn(
        clientd: *mut c_void,
        term_buffer: *mut u8,
        term_offset: usize,
        frame_length: usize,
    ) -> i64,
>;
#[allow(non_camel_case_types)]
type aeron_on_available_image_t =
    Option<unsafe extern "C" fn(*mut c_void, *mut aeron_subscription_t, *mut c_void)>;
#[allow(non_camel_case_types)]
type aeron_on_unavailable_image_t =
    Option<unsafe extern "C" fn(*mut c_void, *mut aeron_subscription_t, *mut c_void)>;

#[allow(dead_code)]
unsafe extern "C" {
    fn aeron_context_init(context: *mut *mut aeron_context_t) -> c_int;
    fn aeron_context_close(context: *mut aeron_context_t) -> c_int;
    fn aeron_context_set_dir(context: *mut aeron_context_t, value: *const c_char) -> c_int;

    fn aeron_init(client: *mut *mut aeron_t, context: *mut aeron_context_t) -> c_int;
    fn aeron_start(client: *mut aeron_t) -> c_int;
    fn aeron_close(client: *mut aeron_t) -> c_int;

    fn aeron_async_add_publication(
        async_cmd: *mut *mut aeron_async_add_publication_t,
        client: *mut aeron_t,
        uri: *const c_char,
        stream_id: i32,
    ) -> c_int;
    fn aeron_async_add_publication_poll(
        publication: *mut *mut aeron_publication_t,
        async_cmd: *mut aeron_async_add_publication_t,
    ) -> c_int;
    fn aeron_async_add_subscription(
        async_cmd: *mut *mut aeron_async_add_subscription_t,
        client: *mut aeron_t,
        uri: *const c_char,
        stream_id: i32,
        on_available_image_handler: aeron_on_available_image_t,
        on_available_image_clientd: *mut c_void,
        on_unavailable_image_handler: aeron_on_unavailable_image_t,
        on_unavailable_image_clientd: *mut c_void,
    ) -> c_int;
    fn aeron_async_add_subscription_poll(
        subscription: *mut *mut aeron_subscription_t,
        async_cmd: *mut aeron_async_add_subscription_t,
    ) -> c_int;
    fn aeron_async_cmd_free(async_cmd: *mut aeron_client_registering_resource_stct);

    fn aeron_publication_offer(
        publication: *mut aeron_publication_t,
        buffer: *const u8,
        length: usize,
        reserved_value_supplier: aeron_reserved_value_supplier_t,
        clientd: *mut c_void,
    ) -> i64;
    fn aeron_publication_close(
        publication: *mut aeron_publication_t,
        on_close_complete: aeron_notification_t,
        on_close_complete_clientd: *mut c_void,
    ) -> c_int;

    fn aeron_subscription_poll(
        subscription: *mut aeron_subscription_t,
        handler: aeron_fragment_handler_t,
        clientd: *mut c_void,
        fragment_limit: usize,
    ) -> c_int;
    fn aeron_subscription_close(
        subscription: *mut aeron_subscription_t,
        on_close_complete: aeron_notification_t,
        on_close_complete_clientd: *mut c_void,
    ) -> c_int;

    fn aeron_errcode() -> c_int;
    fn aeron_errmsg() -> *const c_char;

    fn aeron_driver_context_init(context: *mut *mut aeron_driver_context_t) -> c_int;
    fn aeron_driver_context_close(context: *mut aeron_driver_context_t) -> c_int;
    fn aeron_driver_context_set_dir(
        context: *mut aeron_driver_context_t,
        value: *const c_char,
    ) -> c_int;
    fn aeron_driver_context_set_dir_delete_on_start(
        context: *mut aeron_driver_context_t,
        value: bool,
    ) -> c_int;
    fn aeron_driver_context_set_dir_delete_on_shutdown(
        context: *mut aeron_driver_context_t,
        value: bool,
    ) -> c_int;
    fn aeron_driver_context_set_threading_mode(
        context: *mut aeron_driver_context_t,
        mode: aeron_threading_mode_t,
    ) -> c_int;
    fn aeron_driver_context_set_shared_idle_strategy(
        context: *mut aeron_driver_context_t,
        value: *const c_char,
    ) -> c_int;
    fn aeron_driver_context_set_term_buffer_sparse_file(
        context: *mut aeron_driver_context_t,
        value: bool,
    ) -> c_int;
    fn aeron_driver_context_set_term_buffer_length(
        context: *mut aeron_driver_context_t,
        value: usize,
    ) -> c_int;
    fn aeron_driver_context_set_ipc_term_buffer_length(
        context: *mut aeron_driver_context_t,
        value: usize,
    ) -> c_int;
    fn aeron_driver_context_set_timer_interval_ns(
        context: *mut aeron_driver_context_t,
        value: u64,
    ) -> c_int;
    fn aeron_driver_context_set_client_liveness_timeout_ns(
        context: *mut aeron_driver_context_t,
        value: u64,
    ) -> c_int;
    fn aeron_driver_context_set_publication_linger_timeout_ns(
        context: *mut aeron_driver_context_t,
        value: u64,
    ) -> c_int;
    fn aeron_driver_context_set_image_liveness_timeout_ns(
        context: *mut aeron_driver_context_t,
        value: u64,
    ) -> c_int;
    fn aeron_driver_context_set_enable_experimental_features(
        context: *mut aeron_driver_context_t,
        value: bool,
    ) -> c_int;

    fn aeron_driver_init(
        driver: *mut *mut aeron_driver_t,
        context: *mut aeron_driver_context_t,
    ) -> c_int;
    fn aeron_driver_start(driver: *mut aeron_driver_t, manual_main_loop: bool) -> c_int;
    fn aeron_driver_main_do_work(driver: *mut aeron_driver_t) -> c_int;
    fn aeron_driver_main_idle_strategy(driver: *mut aeron_driver_t, work_count: c_int);
    fn aeron_driver_close(driver: *mut aeron_driver_t) -> c_int;
}
