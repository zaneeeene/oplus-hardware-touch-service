//! Raise-to-wake through the Xiaomi sensor HAL.
//!
//! The working `oplus-hardware-raise2wake` binary does not read an
//! accelerometer evdev node. It subscribes to the Android sensor manager's
//! `xiaomi.sensor.pickup` sensor, accepts `data[0] == 1.0` as "picked up", and
//! wakes the device with `/system/bin/cmd power wakeup`. That path survives AP
//! suspend because the vendor sensor HAL owns delivery.

use std::ffi::{CStr, CString};
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use tokio::sync::broadcast;

use crate::config::RaiseToWakeConfig;
use crate::error::{TouchError, TouchResult};
use crate::screen_state::ScreenState;
use crate::sysfs::SysfsManager;
use crate::uinput::WakeInjector;
use crate::wake;

const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: i32 = 1;
const ALOOPER_POLL_ERROR: i32 = -4;
const EVENT_BATCH: usize = 8;
const POLL_TIMEOUT_MS: i32 = 500;
const SETTING_RECHECK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct RaiseToWakeEvent {
    pub timestamp: u64,
    pub value: f32,
}

#[repr(C)]
struct ASensorEvent {
    version: i32,
    sensor: i32,
    type_: i32,
    reserved0: i32,
    timestamp: i64,
    data: [f32; 16],
    flags: u32,
    reserved1: [i32; 3],
}

enum ASensorManager {}
enum ASensor {}
enum ASensorEventQueue {}
enum ALooper {}

type ALooperCallbackFunc = Option<unsafe extern "C" fn(i32, i32, *mut libc::c_void) -> i32>;

struct AndroidApi {
    _handle: *mut libc::c_void,
    get_instance: unsafe extern "C" fn(*const libc::c_char) -> *mut ASensorManager,
    get_sensor_list: unsafe extern "C" fn(*mut ASensorManager, *mut *const *const ASensor) -> i32,
    get_type: unsafe extern "C" fn(*const ASensor) -> i32,
    get_string_type: unsafe extern "C" fn(*const ASensor) -> *const libc::c_char,
    is_wakeup_sensor: unsafe extern "C" fn(*const ASensor) -> i32,
    looper_prepare: unsafe extern "C" fn(i32) -> *mut ALooper,
    create_event_queue: unsafe extern "C" fn(
        *mut ASensorManager,
        *mut ALooper,
        i32,
        ALooperCallbackFunc,
        *mut libc::c_void,
    ) -> *mut ASensorEventQueue,
    destroy_event_queue: unsafe extern "C" fn(*mut ASensorManager, *mut ASensorEventQueue),
    enable_sensor: unsafe extern "C" fn(*mut ASensorEventQueue, *const ASensor) -> i32,
    disable_sensor: unsafe extern "C" fn(*mut ASensorEventQueue, *const ASensor) -> i32,
    get_events: unsafe extern "C" fn(*mut ASensorEventQueue, *mut ASensorEvent, usize) -> isize,
    looper_poll_all: unsafe extern "C" fn(i32, *mut i32, *mut i32, *mut *mut libc::c_void) -> i32,
}

impl AndroidApi {
    fn load() -> Option<Self> {
        let handle =
            unsafe { libc::dlopen(c"libandroid.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            warn!("Raise-to-wake: libandroid.so load failed: {}", dlerror());
            return None;
        }

        unsafe {
            Some(Self {
                _handle: handle,
                get_instance: symbol(handle, c"ASensorManager_getInstanceForPackage")?,
                get_sensor_list: symbol(handle, c"ASensorManager_getSensorList")?,
                get_type: symbol(handle, c"ASensor_getType")?,
                get_string_type: symbol(handle, c"ASensor_getStringType")?,
                is_wakeup_sensor: symbol(handle, c"ASensor_isWakeUpSensor")?,
                looper_prepare: symbol(handle, c"ALooper_prepare")?,
                create_event_queue: symbol(handle, c"ASensorManager_createEventQueue")?,
                destroy_event_queue: symbol(handle, c"ASensorManager_destroyEventQueue")?,
                enable_sensor: symbol(handle, c"ASensorEventQueue_enableSensor")?,
                disable_sensor: symbol(handle, c"ASensorEventQueue_disableSensor")?,
                get_events: symbol(handle, c"ASensorEventQueue_getEvents")?,
                looper_poll_all: symbol(handle, c"ALooper_pollAll")?,
            })
        }
    }
}

struct PickupSensor {
    sensor: *const ASensor,
    type_code: i32,
    wakeup: bool,
}

#[derive(Clone)]
pub struct RaiseToWakeManager {
    config: RaiseToWakeConfig,
    sysfs: Arc<SysfsManager>,
    event_tx: broadcast::Sender<RaiseToWakeEvent>,
    running: Arc<AtomicBool>,
    sensitivity: Arc<AtomicU8>,
    thread: Arc<StdMutex<Option<JoinHandle<()>>>>,
}

impl RaiseToWakeManager {
    pub fn new(config: RaiseToWakeConfig, sysfs: Arc<SysfsManager>) -> TouchResult<Self> {
        let (event_tx, _) = broadcast::channel(50);
        Ok(Self {
            sensitivity: Arc::new(AtomicU8::new(config.sensitivity)),
            config,
            sysfs,
            event_tx,
            running: Arc::new(AtomicBool::new(false)),
            thread: Arc::new(StdMutex::new(None)),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RaiseToWakeEvent> {
        self.event_tx.subscribe()
    }

    pub async fn start(&self) -> TouchResult<()> {
        if !self.config.enabled {
            info!("Raise to wake is disabled in config");
            return Ok(());
        }
        if self.running.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        if let Err(error) = self.sysfs.write_bool("raise_to_wake_enable", true).await {
            debug!("Optional raise-to-wake enable node unavailable: {error}");
        }

        let manager = self.clone();
        let handle = thread::Builder::new()
            .name("oplus-raise-to-wake".to_string())
            .spawn(move || manager.run_sensor_loop())
            .map_err(TouchError::Io)?;
        *self.thread.lock().expect("raise-to-wake thread lock") = Some(handle);
        info!(
            "Raise to wake started through Android sensor {}",
            self.config.sensor_type
        );
        Ok(())
    }

    pub async fn stop(&self) -> TouchResult<()> {
        self.running.store(false, Ordering::Release);
        if let Err(error) = self.sysfs.write_bool("raise_to_wake_enable", false).await {
            debug!("Optional raise-to-wake disable node unavailable: {error}");
        }
        let handle = self
            .thread
            .lock()
            .expect("raise-to-wake thread lock")
            .take();
        if let Some(handle) = handle {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
        info!("Raise to wake stopped");
        Ok(())
    }

    pub async fn set_sensitivity(&self, sensitivity: u8) {
        self.sensitivity.store(sensitivity, Ordering::Release);
        info!(
            "Raise-to-wake sensitivity set to {sensitivity}; sensor HAL mode uses the pickup event"
        );
    }

    pub async fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    fn run_sensor_loop(&self) {
        let Some(api) = AndroidApi::load() else {
            self.running.store(false, Ordering::Release);
            return;
        };

        let package = match CString::new(self.config.sensor_package.as_str()) {
            Ok(package) => package,
            Err(_) => {
                warn!("Raise-to-wake sensor package contains a NUL byte");
                self.running.store(false, Ordering::Release);
                return;
            }
        };
        let mut discovery_warning_logged = false;

        // late-fs starts this daemon before sensorservice is guaranteed to be
        // ready. Keep trying instead of disabling raise-to-wake until reboot.
        while self.running.load(Ordering::Acquire) {
            let manager = unsafe { (api.get_instance)(package.as_ptr()) };
            if manager.is_null() {
                if !discovery_warning_logged {
                    warn!("Raise-to-wake: waiting for the Android sensor manager");
                    discovery_warning_logged = true;
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let Some(pickup) = find_pickup_sensor(&api, manager, &self.config) else {
                if !discovery_warning_logged {
                    warn!(
                        "Raise-to-wake: sensor '{}' is not published yet; retrying",
                        self.config.sensor_type
                    );
                    discovery_warning_logged = true;
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            discovery_warning_logged = false;
            info!(
                "Raise-to-wake selected '{}' (type {}, wakeup={})",
                self.config.sensor_type, pickup.type_code, pickup.wakeup
            );

            let looper = unsafe { (api.looper_prepare)(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) };
            if looper.is_null() {
                warn!("Raise-to-wake: ALooper_prepare failed; retrying");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            let queue =
                unsafe { (api.create_event_queue)(manager, looper, 1, None, ptr::null_mut()) };
            if queue.is_null() {
                warn!("Raise-to-wake: sensor event queue creation failed; retrying");
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let mut sensor_enabled = false;
            let mut enabled_at = Instant::now();
            let mut last_setting_check = Instant::now() - SETTING_RECHECK;
            let mut setting_enabled = false;
            let mut last_trigger = Instant::now() - Duration::from_secs(10);
            let mut fallback_injector = None;
            let mut setting_warning_logged = false;

            info!("Raise-to-wake sensor HAL loop is ready");
            while self.running.load(Ordering::Acquire) {
                if last_setting_check.elapsed() >= SETTING_RECHECK {
                    last_setting_check = Instant::now();
                    setting_enabled = self.setting_enabled();
                    if self.config.require_setting && !setting_enabled && !setting_warning_logged {
                        warn!(
                            "Raise-to-wake is disabled by secure setting {}.{}; enable it in Settings",
                            self.config.setting_namespace, self.config.setting_key
                        );
                        setting_warning_logged = true;
                    } else if setting_enabled {
                        setting_warning_logged = false;
                    }
                }

                let screen_off = ScreenState::read().is_screen_off_or_doze();
                let should_enable = screen_off && setting_enabled;
                if should_enable != sensor_enabled {
                    let result = if should_enable {
                        unsafe { (api.enable_sensor)(queue, pickup.sensor) }
                    } else {
                        unsafe { (api.disable_sensor)(queue, pickup.sensor) }
                    };
                    if result == 0 {
                        sensor_enabled = should_enable;
                        enabled_at = Instant::now();
                        info!(
                            "Raise-to-wake sensor {}",
                            if sensor_enabled { "armed" } else { "disarmed" }
                        );
                    } else {
                        warn!(
                            "Raise-to-wake sensor {} failed: {result}",
                            if should_enable { "enable" } else { "disable" }
                        );
                    }
                }

                let poll = unsafe {
                    (api.looper_poll_all)(
                        POLL_TIMEOUT_MS,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    )
                };
                if poll == ALOOPER_POLL_ERROR {
                    warn!("Raise-to-wake sensor poll failed; rebuilding the queue");
                    break;
                }
                if !sensor_enabled {
                    continue;
                }

                // pollAll returns a non-negative identifier only when the
                // looper actually has an event. Do not touch the global wake
                // lock on ordinary timeouts or the device would never sleep.
                if poll >= 0 {
                    // getEvents acknowledges the sensor service's wake-up
                    // wakelock. Hold our own for one second so the userspace
                    // wake request cannot race AP suspend.
                    hold_wake_lock();
                }
                loop {
                    let mut events: [ASensorEvent; EVENT_BATCH] = unsafe { std::mem::zeroed() };
                    let count =
                        unsafe { (api.get_events)(queue, events.as_mut_ptr(), EVENT_BATCH) };
                    if count <= 0 {
                        break;
                    }
                    for event in events.iter().take(count as usize) {
                        if event.type_ != pickup.type_code {
                            continue;
                        }
                        let value = event.data[0];
                        if (value - 1.0).abs() > f32::EPSILON {
                            // 2.0 means put down. Do not wake for the initial
                            // "held" report or a subsequent table placement.
                            continue;
                        }
                        if enabled_at.elapsed() < Duration::from_millis(self.config.arm_grace_ms) {
                            debug!(
                                "Ignoring pickup state during sensor arm grace: {:.1}",
                                value
                            );
                            continue;
                        }

                        let now = Instant::now();
                        if now.duration_since(last_trigger)
                            < Duration::from_millis(self.config.debounce_ms)
                        {
                            continue;
                        }
                        last_trigger = now;

                        let timestamp = now_ms();
                        let _ = self.event_tx.send(RaiseToWakeEvent { timestamp, value });
                        info!("Raise to wake triggered by pickup sensor");

                        if let Err(error) = wake::request_power_wakeup() {
                            warn!("Raise-to-wake power request failed: {error}");
                            if fallback_injector.is_none() {
                                fallback_injector = WakeInjector::open("oplus-raise-to-wake").ok();
                            }
                            match fallback_injector.as_ref() {
                                Some(injector) => {
                                    if let Err(error) = injector.emit_wake() {
                                        warn!("Raise-to-wake uinput fallback failed: {error}");
                                    }
                                }
                                None => warn!(
                                    "Raise-to-wake could not run cmd power wakeup and uinput is unavailable"
                                ),
                            }
                        }
                    }
                }
            }

            if sensor_enabled {
                let _ = unsafe { (api.disable_sensor)(queue, pickup.sensor) };
            }
            unsafe { (api.destroy_event_queue)(manager, queue) };
            if self.running.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(500));
            }
        }
        info!("Raise-to-wake sensor loop stopped");
    }

    fn setting_enabled(&self) -> bool {
        if !self.config.require_setting {
            return true;
        }

        let output = Command::new("/system/bin/settings")
            .args([
                "get",
                self.config.setting_namespace.as_str(),
                self.config.setting_key.as_str(),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(output) if output.status.success() => {
                matches!(String::from_utf8_lossy(&output.stdout).trim(), "1" | "true")
            }
            _ => false,
        }
    }
}

fn find_pickup_sensor(
    api: &AndroidApi,
    manager: *mut ASensorManager,
    config: &RaiseToWakeConfig,
) -> Option<PickupSensor> {
    let expected = CString::new(config.sensor_type.as_str()).ok()?;
    let mut list: *const *const ASensor = ptr::null();
    let count = unsafe { (api.get_sensor_list)(manager, &mut list) };
    if count <= 0 || list.is_null() {
        return None;
    }

    let mut non_wakeup = None;
    let mut wakeup = None;
    for index in 0..count as usize {
        let sensor = unsafe { *list.add(index) };
        if sensor.is_null() {
            continue;
        }
        let type_name = unsafe { (api.get_string_type)(sensor) };
        if type_name.is_null() {
            continue;
        }
        let type_name = unsafe { CStr::from_ptr(type_name) };
        if type_name != expected.as_c_str() {
            continue;
        }

        let is_wakeup = unsafe { (api.is_wakeup_sensor)(sensor) != 0 };
        let type_code = unsafe { (api.get_type)(sensor) };
        let candidate = PickupSensor {
            sensor,
            type_code,
            wakeup: is_wakeup,
        };
        if is_wakeup {
            wakeup.get_or_insert(candidate);
        } else {
            non_wakeup.get_or_insert(candidate);
        }
    }

    if config.prefer_wakeup_sensor {
        wakeup.or(non_wakeup)
    } else {
        // Match the proven OPlus binary: it selects the non-wake-up variant
        // first, then wakes with `cmd power wakeup`.
        non_wakeup.or(wakeup)
    }
}

fn hold_wake_lock() {
    wake::hold_wake_lock("oplus_hardware_touch_service_raise", Duration::from_secs(1));
}

unsafe fn symbol<T: Copy>(handle: *mut libc::c_void, name: &CStr) -> Option<T> {
    let pointer = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute_copy(&pointer) })
    }
}

fn dlerror() -> String {
    let pointer = unsafe { libc::dlerror() };
    if pointer.is_null() {
        return "unknown dlopen error".to_string();
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
