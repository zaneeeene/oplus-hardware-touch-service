//! Gesture arming, touch classification, and wake/AOD key injection.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use libc::{c_ushort, input_event};
use log::{debug, info, warn};
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use tokio::sync::{broadcast, Mutex};
use tokio::time::Duration;

use crate::config::{GestureConfig, GestureEntry};
use crate::error::{TouchError, TouchResult};
use crate::evdev::configured_or_discover;
use crate::procfs::ProcfsTouch;
use crate::screen_state::ScreenState;
use crate::uinput::WakeInjector;
use crate::wake;
use crate::xiaomi_touch::XiaomiTouch;

const EV_KEY: c_ushort = 0x01;
const EV_ABS: c_ushort = 0x03;
const ABS_MT_POSITION_X: c_ushort = 0x35;
const ABS_MT_POSITION_Y: c_ushort = 0x36;
const ABS_MT_TRACKING_ID: c_ushort = 0x39;
const BTN_TOUCH: c_ushort = 0x14a;
const KEY_WAKEUP: c_ushort = 143;
const KEY_POWER: c_ushort = 116;
const KEY_F4: c_ushort = 62;
const KEY_GOTO: c_ushort = 0x0162;

const TAP_WINDOW_MS: u64 = 350;
const PREVIEW_DEBOUNCE_MS: u64 = 500;

#[cfg(feature = "gesture-backlight")]
const TOUCHSCREEN_NAMES: &[&str] = &[
    "touchscreen",
    "touch screen",
    "nt36",
    "focaltech",
    "goodix",
    "fts",
];
#[cfg(not(feature = "gesture-backlight"))]
const TOUCHSCREEN_NAMES: &[&str] = &["touchscreen", "touch screen", "nt36", "focaltech", "goodix"];

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureType {
    DoubleTap = 0x22,
    SingleTap = 0x23,
    SwipeUp = 0x24,
    SwipeDown = 0x25,
    SwipeLeft = 0x26,
    SwipeRight = 0x27,
    Custom(u8),
}

impl From<u8> for GestureType {
    fn from(val: u8) -> Self {
        match val {
            0x22 => GestureType::DoubleTap,
            0x23 => GestureType::SingleTap,
            0x24 => GestureType::SwipeUp,
            0x25 => GestureType::SwipeDown,
            0x26 => GestureType::SwipeLeft,
            0x27 => GestureType::SwipeRight,
            value => GestureType::Custom(value),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GestureEvent {
    pub gesture_type: GestureType,
    pub timestamp: u64,
    pub x: i32,
    pub y: i32,
    pub wake_screen: bool,
    pub action: String,
}

#[derive(Debug)]
struct TapState {
    x: i32,
    y: i32,
    finger_down: bool,
    tap_count: u32,
    last_tap_ms: u64,
    sequence_id: u64,
    second_tap_started: bool,
    sequence_screen_off: bool,
    last_preview_ms: u64,
}

impl Default for TapState {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            finger_down: false,
            tap_count: 0,
            last_tap_ms: 0,
            sequence_id: 0,
            second_tap_started: false,
            sequence_screen_off: false,
            last_preview_ms: 0,
        }
    }
}

impl TapState {
    fn note_down(&mut self, now: u64, screen_off: bool) {
        self.finger_down = true;

        if self.tap_count == 1 && now.saturating_sub(self.last_tap_ms) <= TAP_WINDOW_MS {
            // Cancel the pending single-tap preview as soon as the second
            // finger-down is seen, even if the user holds it for >350 ms.
            self.second_tap_started = true;
            self.sequence_id = self.sequence_id.wrapping_add(1);
            return;
        }

        self.sequence_id = self.sequence_id.wrapping_add(1);
        self.tap_count = 0;
        self.second_tap_started = false;
        self.sequence_screen_off = screen_off;
    }

    fn finish(
        &mut self,
        now: u64,
        double_enabled: bool,
        single_enabled: bool,
    ) -> Option<TapOutcome> {
        self.finger_down = false;
        self.last_tap_ms = now;

        if self.second_tap_started {
            self.tap_count = 0;
            self.second_tap_started = false;
            if double_enabled && self.sequence_screen_off {
                return Some(TapOutcome::Double {
                    x: self.x,
                    y: self.y,
                });
            }
            return None;
        }

        self.tap_count = 1;
        if single_enabled && self.sequence_screen_off {
            Some(TapOutcome::Single {
                sequence_id: self.sequence_id,
                x: self.x,
                y: self.y,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TapOutcome {
    Double { x: i32, y: i32 },
    Single { sequence_id: u64, x: i32, y: i32 },
}

#[derive(Clone)]
pub struct GestureHandler {
    config: GestureConfig,
    xiaomi_touch: Arc<XiaomiTouch>,
    touchscreen: Option<Arc<OwnedFd>>,
    tp_gesture_path: String,
    event_tx: broadcast::Sender<GestureEvent>,
    running: Arc<Mutex<bool>>,
    tap_state: Arc<Mutex<TapState>>,
    wake_injector: Arc<Mutex<Option<WakeInjector>>>,
    dt2w_allowed: Arc<AtomicBool>,
    wake_retry_running: Arc<AtomicBool>,
}

impl GestureHandler {
    pub fn new(
        config: GestureConfig,
        xiaomi_touch: Arc<XiaomiTouch>,
        touchscreen_path: &str,
        tp_gesture_path: &str,
        enable_uinput_wake: bool,
        dt2w_allowed: Arc<AtomicBool>,
    ) -> TouchResult<Self> {
        let configured = std::path::Path::new(touchscreen_path);
        let discovered = configured_or_discover(configured, TOUCHSCREEN_NAMES)
            .or_else(crate::evdev::find_touchscreen);

        let touchscreen = discovered.and_then(|path| {
            match open(
                path.as_os_str(),
                OFlag::O_RDONLY | OFlag::O_NONBLOCK,
                Mode::empty(),
            ) {
                Ok(fd) => {
                    info!("Watching touchscreen {}", path.display());
                    Some(Arc::new(unsafe { OwnedFd::from_raw_fd(fd) }))
                }
                Err(error) => {
                    warn!("Cannot open touchscreen {}: {error}", path.display());
                    None
                }
            }
        });

        if touchscreen.is_none() {
            warn!(
                "No touchscreen input event found; kernel gestures remain armed through {}",
                tp_gesture_path
            );
        }

        let wake_injector = if enable_uinput_wake {
            match WakeInjector::open("oplus-touch-gesture") {
                Ok(injector) => Some(injector),
                Err(error) => {
                    debug!("uinput gesture injector unavailable: {error}");
                    None
                }
            }
        } else {
            None
        };

        let (event_tx, _) = broadcast::channel(100);
        Ok(Self {
            config,
            xiaomi_touch,
            touchscreen,
            tp_gesture_path: tp_gesture_path.to_string(),
            event_tx,
            running: Arc::new(Mutex::new(false)),
            tap_state: Arc::new(Mutex::new(TapState::default())),
            wake_injector: Arc::new(Mutex::new(wake_injector)),
            dt2w_allowed,
            wake_retry_running: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GestureEvent> {
        self.event_tx.subscribe()
    }

    /// Arm gestures and launch the event loop as a background task.
    pub async fn start(&self) -> TouchResult<()> {
        {
            let mut running = self.running.lock().await;
            if *running {
                return Ok(());
            }
            *running = true;
        }

        if self.config.double_tap.enabled {
            if let Err(error) = self.xiaomi_touch.enable_dt2w(true).await {
                debug!("xiaomi-touch DT2W setup skipped: {error}");
            }
        }
        self.enable_kernel_gestures().await;

        if let Some(touchscreen) = self.touchscreen.clone() {
            let handler = self.clone();
            tokio::spawn(async move {
                if let Err(error) = handler.run_event_loop(touchscreen).await {
                    log::error!("Touch event loop stopped: {error}");
                }
            });
        }

        info!(
            "Gesture handler started (single-tap AOD preview {}, double-tap {})",
            self.config.single_tap.enabled, self.config.double_tap.enabled
        );
        Ok(())
    }

    pub async fn stop(&self) -> TouchResult<()> {
        *self.running.lock().await = false;
        if self.config.double_tap.enabled {
            if let Err(error) = self.xiaomi_touch.enable_dt2w(false).await {
                debug!("xiaomi-touch DT2W disable skipped: {error}");
            }
        }
        info!("Gesture handler stopped");
        Ok(())
    }

    async fn enable_kernel_gestures(&self) {
        let entries = [
            &self.config.double_tap,
            &self.config.single_tap,
            &self.config.swipe_up,
            &self.config.swipe_down,
            &self.config.swipe_left,
            &self.config.swipe_right,
        ];
        let mut mask = 0u64;
        for entry in entries {
            if entry.enabled && entry.gesture_id < 64 {
                mask |= 1u64 << entry.gesture_id;
            }
        }
        for entry in &self.config.custom {
            if entry.enabled && entry.gesture_id < 64 {
                mask |= 1u64 << entry.gesture_id;
            }
        }

        let procfs = ProcfsTouch::new(&self.tp_gesture_path, "/proc/xiaomi-touch");
        match procfs.set_gesture_mask(mask) {
            Ok(()) => info!("Kernel gestures armed through {}", self.tp_gesture_path),
            Err(error) => warn!(
                "Could not arm {}: {error}; xiaomi-touch remains the fallback",
                self.tp_gesture_path
            ),
        }
    }

    async fn run_event_loop(&self, fd: Arc<OwnedFd>) -> TouchResult<()> {
        let mut buffer: [input_event; 64] = unsafe { std::mem::zeroed() };
        loop {
            if !*self.running.lock().await {
                break;
            }

            let count = unsafe {
                libc::read(
                    fd.as_raw_fd(),
                    buffer.as_mut_ptr() as *mut libc::c_void,
                    std::mem::size_of_val(&buffer),
                )
            };
            if count < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    continue;
                }
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(TouchError::Io(error));
            }
            if count == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }

            let count = count as usize / std::mem::size_of::<input_event>();
            self.process_events(&buffer[..count]).await?;
        }
        Ok(())
    }

    async fn process_events(&self, events: &[input_event]) -> TouchResult<()> {
        let mut outcome = None;
        let mut kernel_reported_wake = false;
        let mut direct_preview = None;
        let mut state = self.tap_state.lock().await;

        for event in events {
            match (event.type_, event.code) {
                (EV_ABS, ABS_MT_POSITION_X) => state.x = event.value,
                (EV_ABS, ABS_MT_POSITION_Y) => state.y = event.value,
                (EV_ABS, ABS_MT_TRACKING_ID) => {
                    let down = event.value >= 0;
                    if down != state.finger_down {
                        if down {
                            self.note_touch_down(&mut state);
                        } else {
                            outcome = self.finish_tap(&mut state);
                        }
                    }
                }
                (EV_KEY, BTN_TOUCH) => {
                    let down = event.value != 0;
                    if down != state.finger_down {
                        if down {
                            self.note_touch_down(&mut state);
                        } else {
                            outcome = self.finish_tap(&mut state);
                        }
                    }
                }
                (EV_KEY, KEY_GOTO) if event.value == 1 => {
                    // The Novatek/focaltech driver can report its own AOD
                    // single-tap as Linux KEY_GOTO. Treat it as authoritative
                    // and cancel the userspace release timer.
                    let now = now_ms();
                    let screen = ScreenState::read();
                    debug!(
                        "Kernel AOD single-tap KEY_GOTO at ({}, {}), screen={screen:?}",
                        state.x, state.y
                    );
                    if now.saturating_sub(state.last_preview_ms) >= PREVIEW_DEBOUNCE_MS
                        && screen_is_off_or_doze(screen)
                    {
                        state.sequence_id = state.sequence_id.wrapping_add(1);
                        state.tap_count = 0;
                        state.second_tap_started = false;
                        state.last_preview_ms = now;
                        direct_preview = Some((state.x, state.y));
                    }
                }
                (EV_KEY, KEY_WAKEUP | KEY_POWER | KEY_F4) if event.value == 1 => {
                    if screen_is_off_or_doze(ScreenState::read()) {
                        kernel_reported_wake = true;
                        outcome = Some(TapOutcome::Double {
                            x: state.x,
                            y: state.y,
                        });
                    }
                }
                _ => {}
            }
        }
        drop(state);

        if let Some((x, y)) = direct_preview {
            self.emit_gesture(GestureType::SingleTap, x, y).await?;
            return Ok(());
        }

        match outcome {
            Some(TapOutcome::Double { x, y }) => {
                if kernel_reported_wake {
                    debug!(
                        "Touch driver reported a wake key; requesting an explicit power wake anyway"
                    );
                }
                self.emit_gesture(GestureType::DoubleTap, x, y).await?;
            }
            Some(TapOutcome::Single { sequence_id, x, y }) => {
                self.schedule_single_tap(sequence_id, x, y);
            }
            None => {}
        }
        Ok(())
    }

    fn note_touch_down(&self, state: &mut TapState) {
        let screen_off = match ScreenState::read() {
            ScreenState::Unknown => {
                debug!("Screen state unavailable; suppressing userspace tap actions");
                screen_is_off_or_doze(ScreenState::Unknown)
            }
            state => screen_is_off_or_doze(state),
        };
        state.note_down(now_ms(), screen_off);
    }

    fn finish_tap(&self, state: &mut TapState) -> Option<TapOutcome> {
        state.finish(
            now_ms(),
            self.config.double_tap.enabled && self.dt2w_allowed.load(Ordering::Acquire),
            self.config.single_tap.enabled,
        )
    }

    fn schedule_single_tap(&self, sequence_id: u64, x: i32, y: i32) {
        let handler = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(TAP_WINDOW_MS)).await;
            if let Err(error) = handler.finish_pending_single(sequence_id, x, y).await {
                debug!("Single-tap AOD preview skipped: {error}");
            }
        });
    }

    async fn finish_pending_single(&self, sequence_id: u64, x: i32, y: i32) -> TouchResult<()> {
        if !*self.running.lock().await {
            return Ok(());
        }

        let now = now_ms();
        let mut state = self.tap_state.lock().await;
        if state.sequence_id != sequence_id
            || state.tap_count != 1
            || state.finger_down
            || state.second_tap_started
            || now.saturating_sub(state.last_preview_ms) < PREVIEW_DEBOUNCE_MS
        {
            return Ok(());
        }

        if !state.sequence_screen_off || screen_is_awake(ScreenState::read()) {
            return Ok(());
        }

        state.tap_count = 0;
        state.sequence_id = state.sequence_id.wrapping_add(1);
        state.last_preview_ms = now;
        drop(state);

        info!("Userspace AOD single-tap at ({x}, {y}); requesting preview dispatch");
        self.emit_gesture(GestureType::SingleTap, x, y).await
    }

    async fn emit_gesture(&self, gesture_type: GestureType, x: i32, y: i32) -> TouchResult<()> {
        let (enabled, action, wake_screen) = self.entry_for(gesture_type);
        if !enabled {
            return Ok(());
        }

        if gesture_type == GestureType::SingleTap && action == "aod_preview" && !wake_screen {
            if !screen_is_off_or_doze(ScreenState::read()) {
                debug!("AOD preview suppressed because the display is awake");
                return Ok(());
            }
            self.inject_aod_preview().await;
        } else if wake_screen {
            self.request_dt2w_wake(gesture_type);
        }

        let event = GestureEvent {
            gesture_type,
            timestamp: now_ms(),
            x,
            y,
            wake_screen,
            action,
        };
        let _ = self.event_tx.send(event);
        debug!("Emitted gesture {gesture_type:?}");
        Ok(())
    }

    async fn inject_aod_preview(&self) {
        // Use Android's InputManager path first. A raw uinput KEY_F4 can be
        // accepted by the kernel yet never be routed through OPlus SystemUI's
        // screen-off gesture dispatcher, which made preview look successful in
        // logs while doing nothing on screen.
        hold_wake_lock("oplus_hardware_touch_service_preview 1500000000");
        let command_result = tokio::task::spawn_blocking(|| {
            aod_preview_command()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        })
        .await;

        match command_result {
            Ok(Ok(status)) if status.success() => {
                info!("Dispatched Android keycode 134 for AOD single-tap preview");
                return;
            }
            Ok(Ok(status)) => {
                warn!("AOD preview cmd exited with {status}; trying the uinput KEY_F4 fallback")
            }
            Ok(Err(error)) => {
                warn!("AOD preview cmd failed: {error}; trying the uinput KEY_F4 fallback")
            }
            Err(error) => {
                warn!("AOD preview cmd task failed: {error}; trying the uinput KEY_F4 fallback")
            }
        }

        let injector = self.wake_injector.lock().await;
        match injector.as_ref().map(WakeInjector::emit_aod_preview) {
            Some(Ok(())) => info!("Injected uinput KEY_F4 for AOD single-tap preview fallback"),
            Some(Err(error)) => warn!("uinput AOD preview fallback failed: {error}"),
            None => warn!("AOD preview has no dispatch path: uinput injector is unavailable"),
        }
    }

    fn request_dt2w_wake(&self, gesture_type: GestureType) {
        if self.wake_retry_running.swap(true, Ordering::AcqRel) {
            debug!("Wake gesture {gesture_type:?} arrived while a wake request is already pending");
            return;
        }

        let handler = self.clone();
        tokio::spawn(async move {
            for attempt in 1..=2 {
                if !screen_is_off_or_doze(ScreenState::read()) {
                    debug!("Wake gesture {gesture_type:?}: display became awake before attempt {attempt}");
                    break;
                }

                wake::hold_wake_lock("oplus_hardware_touch_service_dt2w", Duration::from_secs(2));
                let command_result = tokio::task::spawn_blocking(wake::request_power_wakeup).await;
                match command_result {
                    Ok(Ok(())) => {
                        info!(
                            "DT2W attempt {attempt}: requested Android power wake for {gesture_type:?}"
                        );
                    }
                    Ok(Err(error)) => {
                        warn!("DT2W power wake attempt {attempt} failed: {error}");
                        handler.inject_uinput_wake().await;
                    }
                    Err(error) => {
                        warn!("DT2W power wake task failed: {error}");
                        handler.inject_uinput_wake().await;
                    }
                }

                tokio::time::sleep(Duration::from_millis(250)).await;
                let state = ScreenState::read();
                if !screen_is_off_or_doze(state) {
                    info!("DT2W wake confirmed after attempt {attempt} (state={state:?})");
                    break;
                }

                if attempt == 2 {
                    warn!("DT2W wake request completed but the display is still {state:?}");
                } else {
                    debug!("DT2W wake not visible after attempt {attempt}; retrying once");
                }
            }
            handler.wake_retry_running.store(false, Ordering::Release);
        });
    }

    async fn inject_uinput_wake(&self) {
        let injector = self.wake_injector.lock().await;
        match injector.as_ref() {
            Some(injector) => match injector.emit_wake() {
                Ok(()) => info!("Injected uinput KEY_WAKEUP fallback"),
                Err(error) => warn!("uinput KEY_WAKEUP fallback failed: {error}"),
            },
            None => warn!("No uinput wake injector is available"),
        }
    }

    fn entry_for(&self, gesture_type: GestureType) -> (bool, String, bool) {
        let entry: &GestureEntry = match gesture_type {
            GestureType::DoubleTap => &self.config.double_tap,
            GestureType::SingleTap => &self.config.single_tap,
            GestureType::SwipeUp => &self.config.swipe_up,
            GestureType::SwipeDown => &self.config.swipe_down,
            GestureType::SwipeLeft => &self.config.swipe_left,
            GestureType::SwipeRight => &self.config.swipe_right,
            GestureType::Custom(id) => {
                if let Some(entry) = self
                    .config
                    .custom
                    .iter()
                    .find(|entry| entry.gesture_id == id)
                {
                    entry
                } else {
                    return (false, "none".to_string(), false);
                }
            }
        };
        (entry.enabled, entry.action.clone(), entry.wake_screen)
    }

    pub async fn handle_kernel_gesture(&self, gesture_id: u8) -> TouchResult<()> {
        self.emit_gesture(GestureType::from(gesture_id), 0, 0).await
    }
}

#[derive(Clone)]
pub struct AodGestureHandler {
    expanded: Arc<Mutex<bool>>,
}

impl AodGestureHandler {
    pub fn new(_gesture_handler: Arc<GestureHandler>) -> Self {
        Self {
            expanded: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn handle_aod_double_tap(&self, x: i32, y: i32) -> TouchResult<bool> {
        let mut expanded = self.expanded.lock().await;
        if !*expanded {
            *expanded = true;
            info!("AOD: expanding music/activity at ({x}, {y})");
        } else {
            *expanded = false;
            info!("AOD: collapsing music/activity view");
        }
        Ok(*expanded)
    }

    pub async fn is_expanded(&self) -> bool {
        *self.expanded.lock().await
    }

    pub async fn collapse(&self) {
        *self.expanded.lock().await = false;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(feature = "gesture-backlight")]
fn panel_backlight_is_zero() -> bool {
    std::fs::read_to_string("/sys/class/backlight/panel0-backlight/brightness")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .is_some_and(|brightness| brightness == 0)
}

#[cfg(feature = "gesture-backlight")]
fn screen_is_off_or_doze(state: ScreenState) -> bool {
    state.is_screen_off_or_doze() || panel_backlight_is_zero()
}

#[cfg(not(feature = "gesture-backlight"))]
fn screen_is_off_or_doze(state: ScreenState) -> bool {
    state.is_screen_off_or_doze()
}

#[cfg(feature = "gesture-backlight")]
fn screen_is_awake(state: ScreenState) -> bool {
    state.is_on() && !panel_backlight_is_zero()
}

#[cfg(not(feature = "gesture-backlight"))]
fn screen_is_awake(state: ScreenState) -> bool {
    state.is_on()
}

fn aod_preview_command() -> Command {
    #[cfg(feature = "gesture-backlight")]
    {
        let mut command = Command::new("/system/bin/input");
        command.args(["keyevent", "134"]);
        command
    }

    #[cfg(not(feature = "gesture-backlight"))]
    {
        let mut command = Command::new("/system/bin/cmd");
        command.args(["input", "keyevent", "134"]);
        command
    }
}

fn hold_wake_lock(request: &str) {
    // A timed wake lock keeps the AP out of suspend until InputManager has
    // consumed the key. It does not light the panel.
    let _ = std::fs::write("/sys/power/wake_lock", request);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awake_tap_sequence_cannot_wake_after_screen_off() {
        let mut state = TapState {
            x: 100,
            y: 200,
            ..TapState::default()
        };

        state.note_down(1_000, false);
        assert_eq!(state.finish(1_100, true, true), None);

        state.note_down(1_200, false);
        assert!(state.second_tap_started);
        assert_eq!(state.finish(1_300, true, true), None);
    }

    #[test]
    fn asleep_double_tap_produces_one_double_outcome() {
        let mut state = TapState {
            x: 540,
            y: 1_200,
            ..TapState::default()
        };

        state.note_down(1_000, true);
        assert!(matches!(
            state.finish(1_100, true, true),
            Some(TapOutcome::Single { .. })
        ));
        state.note_down(1_200, true);
        assert!(state.second_tap_started);
        assert!(matches!(
            state.finish(1_300, true, true),
            Some(TapOutcome::Double { x: 540, y: 1_200 })
        ));
    }

    #[test]
    fn second_touch_down_cancels_pending_single_preview() {
        let mut state = TapState::default();
        state.note_down(1_000, true);
        let first = state.finish(1_100, true, true).expect("first tap");
        let TapOutcome::Single { sequence_id, .. } = first else {
            panic!("first tap must be a single");
        };

        state.note_down(1_200, true);
        assert_ne!(state.sequence_id, sequence_id);
        assert!(state.second_tap_started);
    }

    #[test]
    fn slow_second_tap_starts_a_new_sequence() {
        let mut state = TapState::default();
        state.note_down(1_000, true);
        let first = state.finish(1_100, true, true).expect("first tap");
        let TapOutcome::Single { sequence_id, .. } = first else {
            panic!("first tap must be a single");
        };

        state.note_down(1_500, true);
        assert_ne!(state.sequence_id, sequence_id);
        assert!(!state.second_tap_started);
        assert_eq!(state.tap_count, 0);
    }

    #[test]
    fn selected_gesture_backend_uses_expected_touchscreen_names() {
        assert!(TOUCHSCREEN_NAMES.contains(&"goodix"));
        assert_eq!(
            TOUCHSCREEN_NAMES.contains(&"fts"),
            cfg!(feature = "gesture-backlight")
        );
    }
}
