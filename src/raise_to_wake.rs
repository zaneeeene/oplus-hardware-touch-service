//! Raise-to-wake through the Xiaomi sensor HAL.
//!
//! The working `oplus-hardware-raise2wake` binary does not read an
//! accelerometer evdev node. It subscribes to the Android sensor manager's
//! `xiaomi.sensor.pickup` sensor, accepts `data[0] == 1.0` as "picked up", and
//! wakes the device with `/system/bin/cmd power wakeup`. That path survives AP
//! suspend because the vendor sensor HAL owns delivery.

use std::ffi::{CStr, CString};
use std::fmt::Write as _;
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
const DISCOVERY_WARNING_INTERVAL: Duration = Duration::from_secs(30);
const PICKUP_TYPE_CODE: i32 = 33171036;

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
    get_default_sensor: unsafe extern "C" fn(*mut ASensorManager, i32, bool) -> *const ASensor,
    get_type: unsafe extern "C" fn(*const ASensor) -> i32,
    get_name: unsafe extern "C" fn(*const ASensor) -> *const libc::c_char,
    get_string_type: unsafe extern "C" fn(*const ASensor) -> *const libc::c_char,
    get_handle: unsafe extern "C" fn(*const ASensor) -> i32,
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

        let api = unsafe {
            (|| {
                Some(Self {
                    _handle: handle,
                    get_instance: required_symbol(handle, c"ASensorManager_getInstanceForPackage")?,
                    get_sensor_list: required_symbol(handle, c"ASensorManager_getSensorList")?,
                    get_default_sensor: required_symbol(
                        handle,
                        c"ASensorManager_getDefaultSensorEx",
                    )?,
                    get_type: required_symbol(handle, c"ASensor_getType")?,
                    get_name: required_symbol(handle, c"ASensor_getName")?,
                    get_string_type: required_symbol(handle, c"ASensor_getStringType")?,
                    get_handle: required_symbol(handle, c"ASensor_getHandle")?,
                    is_wakeup_sensor: required_symbol(handle, c"ASensor_isWakeUpSensor")?,
                    looper_prepare: required_symbol(handle, c"ALooper_prepare")?,
                    create_event_queue: required_symbol(
                        handle,
                        c"ASensorManager_createEventQueue",
                    )?,
                    destroy_event_queue: required_symbol(
                        handle,
                        c"ASensorManager_destroyEventQueue",
                    )?,
                    enable_sensor: required_symbol(handle, c"ASensorEventQueue_enableSensor")?,
                    disable_sensor: required_symbol(handle, c"ASensorEventQueue_disableSensor")?,
                    get_events: required_symbol(handle, c"ASensorEventQueue_getEvents")?,
                    looper_poll_all: required_symbol(handle, c"ALooper_pollAll")?,
                })
            })()
        };
        if api.is_none() {
            unsafe { libc::dlclose(handle) };
        }
        api
    }
}

impl Drop for AndroidApi {
    fn drop(&mut self) {
        unsafe { libc::dlclose(self._handle) };
    }
}

#[derive(Clone)]
struct PickupSensor {
    sensor: *const ASensor,
    name: String,
    string_type: String,
    handle: i32,
    type_code: i32,
    wakeup: bool,
    source: &'static str,
}

#[derive(Clone)]
struct SensorDescriptor {
    sensor: *const ASensor,
    name: String,
    string_type: String,
    handle: i32,
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
        let mut last_load_warning = Instant::now() - DISCOVERY_WARNING_INTERVAL;
        let api = loop {
            if !self.running.load(Ordering::Acquire) {
                return;
            }
            if let Some(api) = AndroidApi::load() {
                break api;
            }
            if last_load_warning.elapsed() >= DISCOVERY_WARNING_INTERVAL {
                warn!("Raise-to-wake: waiting for the Android sensor NDK API");
                last_load_warning = Instant::now();
            }
            thread::sleep(Duration::from_secs(1));
        };

        let package = match CString::new(self.config.sensor_package.as_str()) {
            Ok(package) => package,
            Err(_) => {
                warn!("Raise-to-wake sensor package contains a NUL byte");
                self.running.store(false, Ordering::Release);
                return;
            }
        };
        let mut last_discovery_warning = Instant::now() - DISCOVERY_WARNING_INTERVAL;

        // late-fs starts this daemon before sensorservice is guaranteed to be
        // ready. Keep trying instead of disabling raise-to-wake until reboot.
        while self.running.load(Ordering::Acquire) {
            let manager = unsafe { (api.get_instance)(package.as_ptr()) };
            if manager.is_null() {
                if last_discovery_warning.elapsed() >= DISCOVERY_WARNING_INTERVAL {
                    warn!("Raise-to-wake: waiting for the Android sensor manager");
                    last_discovery_warning = Instant::now();
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let descriptors = enumerate_sensors(&api, manager);
            let Some(pickup) = select_pickup_sensor(&api, manager, &self.config, &descriptors)
            else {
                if last_discovery_warning.elapsed() >= DISCOVERY_WARNING_INTERVAL {
                    warn!(
                        "Raise-to-wake: sensor '{}' / type {} is not published yet; retrying ({})",
                        self.config.sensor_type,
                        PICKUP_TYPE_CODE,
                        summarize_sensors(&descriptors)
                    );
                    last_discovery_warning = Instant::now();
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            last_discovery_warning = Instant::now();
            info!(
                "Raise-to-wake selected name='{}' string='{}' type={} handle={} wakeup={} source={}",
                pickup.name,
                pickup.string_type,
                pickup.type_code,
                pickup.handle,
                pickup.wakeup,
                pickup.source
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
            let mut last_setting_state = None;
            let mut last_setting_error = Instant::now() - DISCOVERY_WARNING_INTERVAL;
            let mut last_screen_state = None;
            let mut last_trigger = Instant::now() - Duration::from_secs(10);
            let mut last_value = None;
            let mut fallback_injector = None;
            let mut last_arm_attempt = Instant::now() - Duration::from_secs(5);

            info!("Raise-to-wake sensor HAL loop is ready");
            while self.running.load(Ordering::Acquire) {
                if last_setting_check.elapsed() >= SETTING_RECHECK {
                    last_setting_check = Instant::now();
                    match self.setting_enabled() {
                        Ok(enabled) => {
                            setting_enabled = enabled;
                            if last_setting_state != Some(enabled) {
                                if enabled || !self.config.require_setting {
                                    info!(
                                        "Raise-to-wake setting {}.{} is enabled",
                                        self.config.setting_namespace, self.config.setting_key
                                    );
                                } else {
                                    warn!(
                                        "Raise-to-wake is disabled by secure setting {}.{}; enable it in Settings",
                                        self.config.setting_namespace, self.config.setting_key
                                    );
                                }
                                last_setting_state = Some(enabled);
                            }
                        }
                        Err(error) => {
                            setting_enabled = false;
                            if last_setting_error.elapsed() >= DISCOVERY_WARNING_INTERVAL {
                                warn!(
                                    "Raise-to-wake could not read secure setting {}.{}: {error}",
                                    self.config.setting_namespace, self.config.setting_key
                                );
                                last_setting_error = Instant::now();
                            }
                        }
                    }
                }

                let screen_state = ScreenState::read();
                if last_screen_state != Some(screen_state) {
                    debug!("Raise-to-wake screen state changed to {screen_state:?}");
                    last_screen_state = Some(screen_state);
                }
                let should_enable = screen_state.is_screen_off_or_doze() && setting_enabled;
                let retry_allowed = last_arm_attempt.elapsed() >= Duration::from_secs(5);
                if should_enable != sensor_enabled && retry_allowed {
                    let result = if should_enable {
                        unsafe { (api.enable_sensor)(queue, pickup.sensor) }
                    } else {
                        unsafe { (api.disable_sensor)(queue, pickup.sensor) }
                    };
                    last_arm_attempt = Instant::now();
                    if result == 0 {
                        sensor_enabled = should_enable;
                        enabled_at = Instant::now();
                        last_value = None;
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
                        if last_value != Some(value) {
                            info!(
                                "Raise-to-wake sensor value {:.1} ({})",
                                value,
                                pickup_value_label(value)
                            );
                            last_value = Some(value);
                        }
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

    fn setting_enabled(&self) -> Result<bool, String> {
        if !self.config.require_setting {
            return Ok(true);
        }

        let output = Command::new("/system/bin/settings")
            .args([
                "get",
                self.config.setting_namespace.as_str(),
                self.config.setting_key.as_str(),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let value = String::from_utf8_lossy(&output.stdout);
                match value.trim() {
                    "1" | "true" => Ok(true),
                    "0" | "false" | "null" | "" => Ok(false),
                    other => Err(format!("unexpected value {other:?}")),
                }
            }
            Ok(output) => Err(format!(
                "settings exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => Err(format!("failed to run /system/bin/settings: {error}")),
        }
    }
}

/// Enumerate the framework's sensors and print pickup events for on-device
/// troubleshooting. This is intentionally independent of the Settings gate and
/// screen state so it can answer whether the sensor itself is usable.
pub fn diagnose_sensors(duration_secs: u64) -> TouchResult<()> {
    let config = RaiseToWakeConfig::default();
    let duration_secs = duration_secs.clamp(5, 600);
    let api = AndroidApi::load()
        .ok_or_else(|| TouchError::RaiseToWake("Android sensor NDK API is unavailable".into()))?;
    let package = CString::new(config.sensor_package.as_str())
        .map_err(|_| TouchError::RaiseToWake("sensor package contains a NUL byte".into()))?;
    let manager = unsafe { (api.get_instance)(package.as_ptr()) };
    if manager.is_null() {
        return Err(TouchError::RaiseToWake(
            "ASensorManager_getInstanceForPackage returned null".into(),
        ));
    }

    println!(
        "Sensor package: {}; screen state: {:?}",
        config.sensor_package,
        ScreenState::read()
    );
    if config.require_setting {
        let output = Command::new("/system/bin/settings")
            .args([
                "get",
                config.setting_namespace.as_str(),
                config.setting_key.as_str(),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match output {
            Ok(output) if output.status.success() => println!(
                "Setting {}.{}={}",
                config.setting_namespace,
                config.setting_key,
                String::from_utf8_lossy(&output.stdout).trim()
            ),
            Ok(output) => println!(
                "Setting read failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => println!("Setting read failed: {error}"),
        }
    } else {
        println!("Setting gate disabled");
    }

    let descriptors = enumerate_sensors(&api, manager);
    println!("Android sensors ({}):", descriptors.len());
    for sensor in &descriptors {
        println!(
            "  name='{}' string='{}' type={} handle={} wakeup={}",
            sensor.name, sensor.string_type, sensor.type_code, sensor.handle, sensor.wakeup
        );
    }

    let pickup = select_pickup_sensor(&api, manager, &config, &descriptors).ok_or_else(|| {
        TouchError::RaiseToWake(format!(
            "no pickup sensor matched '{}' or type {} ({})",
            config.sensor_type,
            PICKUP_TYPE_CODE,
            summarize_sensors(&descriptors)
        ))
    })?;
    println!(
        "Selected name='{}' string='{}' type={} handle={} wakeup={} source={}",
        pickup.name,
        pickup.string_type,
        pickup.type_code,
        pickup.handle,
        pickup.wakeup,
        pickup.source
    );

    let looper = unsafe { (api.looper_prepare)(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) };
    if looper.is_null() {
        return Err(TouchError::RaiseToWake("ALooper_prepare failed".into()));
    }
    let queue = unsafe { (api.create_event_queue)(manager, looper, 1, None, ptr::null_mut()) };
    if queue.is_null() {
        return Err(TouchError::RaiseToWake(
            "sensor event queue creation failed".into(),
        ));
    }
    let enable_result = unsafe { (api.enable_sensor)(queue, pickup.sensor) };
    if enable_result != 0 {
        unsafe { (api.destroy_event_queue)(manager, queue) };
        return Err(TouchError::RaiseToWake(format!(
            "ASensorEventQueue_enableSensor returned {enable_result}"
        )));
    }

    const PROBE_LOCK: &str = "oplus_touch_sensor_probe";
    wake::hold_wake_lock(PROBE_LOCK, Duration::from_secs(duration_secs + 5));
    println!(
        "Listening for {duration_secs}s. Keep the screen off, leave the phone still, then lift it."
    );

    let started = Instant::now();
    let mut event_count = 0usize;
    while started.elapsed() < Duration::from_secs(duration_secs) {
        let poll = unsafe {
            (api.looper_poll_all)(
                POLL_TIMEOUT_MS,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if poll == ALOOPER_POLL_ERROR {
            println!("Sensor poll error");
            break;
        }
        if poll >= 0 {
            wake::hold_wake_lock(PROBE_LOCK, Duration::from_secs(1));
        }

        loop {
            let mut events: [ASensorEvent; EVENT_BATCH] = unsafe { std::mem::zeroed() };
            let count = unsafe { (api.get_events)(queue, events.as_mut_ptr(), EVENT_BATCH) };
            if count <= 0 {
                break;
            }
            for event in events.iter().take(count as usize) {
                event_count += 1;
                println!(
                    "event type={} value={:.3} label={} raw=[{:.3}, {:.3}, {:.3}, {:.3}]",
                    event.type_,
                    event.data[0],
                    pickup_value_label(event.data[0]),
                    event.data[0],
                    event.data[1],
                    event.data[2],
                    event.data[3]
                );
            }
        }
    }

    unsafe { (api.disable_sensor)(queue, pickup.sensor) };
    unsafe { (api.destroy_event_queue)(manager, queue) };
    wake::release_wake_lock(PROBE_LOCK);
    println!("Sensor probe complete: {event_count} event(s)");

    if event_count == 0 {
        return Err(TouchError::RaiseToWake(
            "sensor enabled but delivered no events".into(),
        ));
    }
    Ok(())
}

fn enumerate_sensors(api: &AndroidApi, manager: *mut ASensorManager) -> Vec<SensorDescriptor> {
    let mut list: *const *const ASensor = ptr::null();
    let count = unsafe { (api.get_sensor_list)(manager, &mut list) };
    if count <= 0 || list.is_null() {
        return Vec::new();
    }

    let mut sensors = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let sensor = unsafe { *list.add(index) };
        if sensor.is_null() {
            continue;
        }
        sensors.push(describe_sensor(api, sensor));
    }
    sensors
}

fn describe_sensor(api: &AndroidApi, sensor: *const ASensor) -> SensorDescriptor {
    let name = unsafe { (api.get_name)(sensor) };
    let string_type = unsafe { (api.get_string_type)(sensor) };
    SensorDescriptor {
        sensor,
        name: c_string(name),
        string_type: c_string(string_type),
        handle: unsafe { (api.get_handle)(sensor) },
        type_code: unsafe { (api.get_type)(sensor) },
        wakeup: unsafe { (api.is_wakeup_sensor)(sensor) != 0 },
    }
}

fn select_pickup_sensor(
    api: &AndroidApi,
    manager: *mut ASensorManager,
    config: &RaiseToWakeConfig,
    descriptors: &[SensorDescriptor],
) -> Option<PickupSensor> {
    let mut exact_non_wakeup = None;
    let mut exact_wakeup = None;
    let mut typed_non_wakeup = None;
    let mut typed_wakeup = None;

    for descriptor in descriptors {
        let exact = descriptor.string_type == config.sensor_type;
        if !exact && descriptor.type_code != PICKUP_TYPE_CODE {
            continue;
        }

        if exact {
            if descriptor.wakeup {
                exact_wakeup.get_or_insert_with(|| pickup_from(descriptor, "string-type"));
            } else {
                exact_non_wakeup.get_or_insert_with(|| pickup_from(descriptor, "string-type"));
            }
        } else if descriptor.wakeup {
            typed_wakeup.get_or_insert_with(|| pickup_from(descriptor, "vendor-type"));
        } else {
            typed_non_wakeup.get_or_insert_with(|| pickup_from(descriptor, "vendor-type"));
        }
    }

    let selected = if config.prefer_wakeup_sensor {
        exact_wakeup
            .or(exact_non_wakeup)
            .or(typed_wakeup)
            .or(typed_non_wakeup)
    } else {
        // Match the proven OPlus binary: it selects the non-wake-up variant
        // first, then wakes with `cmd power wakeup`.
        exact_non_wakeup
            .or(exact_wakeup)
            .or(typed_non_wakeup)
            .or(typed_wakeup)
    };
    if selected.is_some() {
        return selected;
    }

    // Some vendor sensor services hide the string type from third-party
    // callers but still resolve the Xiaomi vendor type code. Try both
    // variants so a renamed sensor in a ported HAL does not disable the
    // feature completely.
    for wakeup in if config.prefer_wakeup_sensor {
        [true, false]
    } else {
        [false, true]
    } {
        let sensor = unsafe { (api.get_default_sensor)(manager, PICKUP_TYPE_CODE, wakeup) };
        if !sensor.is_null() {
            let descriptor = describe_sensor(api, sensor);
            return Some(pickup_from(&descriptor, "default-sensor"));
        }
    }
    None
}

fn pickup_from(descriptor: &SensorDescriptor, source: &'static str) -> PickupSensor {
    PickupSensor {
        sensor: descriptor.sensor,
        name: descriptor.name.clone(),
        string_type: descriptor.string_type.clone(),
        handle: descriptor.handle,
        type_code: descriptor.type_code,
        wakeup: descriptor.wakeup,
        source,
    }
}

fn summarize_sensors(descriptors: &[SensorDescriptor]) -> String {
    let mut candidates: Vec<_> = descriptors
        .iter()
        .filter(|sensor| {
            sensor.type_code == PICKUP_TYPE_CODE
                || sensor.string_type.to_ascii_lowercase().contains("pickup")
                || sensor.name.to_ascii_lowercase().contains("pickup")
        })
        .collect();
    if candidates.is_empty() {
        candidates.extend(descriptors.iter().take(8));
    }

    let mut summary = format!("{} Android sensors", descriptors.len());
    for sensor in candidates.into_iter().take(8) {
        let _ = write!(
            summary,
            "; name='{}' string='{}' type={} handle={} wakeup={}",
            sensor.name, sensor.string_type, sensor.type_code, sensor.handle, sensor.wakeup
        );
    }
    summary
}

fn pickup_value_label(value: f32) -> &'static str {
    if (value - 1.0).abs() <= f32::EPSILON {
        "picked up"
    } else if (value - 2.0).abs() <= f32::EPSILON {
        "put down"
    } else {
        "unknown state"
    }
}

fn c_string(pointer: *const libc::c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
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

unsafe fn required_symbol<T: Copy>(handle: *mut libc::c_void, name: &CStr) -> Option<T> {
    let value = unsafe { symbol(handle, name) };
    if value.is_none() {
        warn!(
            "Raise-to-wake: libandroid.so is missing {}",
            name.to_string_lossy()
        );
    }
    value
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickup_values_match_xiaomi_hub_states() {
        assert_eq!(pickup_value_label(1.0), "picked up");
        assert_eq!(pickup_value_label(2.0), "put down");
        assert_eq!(pickup_value_label(0.0), "unknown state");
    }

    #[test]
    fn empty_sensor_list_has_useful_diagnostic() {
        assert_eq!(summarize_sensors(&[]), "0 Android sensors");
    }
}
