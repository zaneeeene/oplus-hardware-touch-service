//! Xiaomi mi_display doze/backlight keeper.
//!
//! OPlus SystemUI does not program the touch panel's AOD brightness on this
//! port. Classic clock/static-image styles therefore fall back to a nearly
//! black DBV, while full-screen styles lose their fade-matched brightness.
//! This worker owns the mi_display doze level and DCS brightness commands.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::{debug, info, warn};

use crate::config::AodKeepaliveConfig;
use crate::screen_state::ScreenState;

const MI_DISP_IOCTL_SET_DOZE_BRIGHTNESS: libc::c_ulong = 0x400c_4402;
const DOZE_OFF: u32 = 0;
const DOZE_HIGH: u32 = 1;
const DOZE_LOW: u32 = 2;

const AOD_GAIN: f32 = 1945.0 / 808.0;
const AOD_DBV_MIN: i32 = 16;
const AOD_DBV_MAX: i32 = 808;
const MIN_FADE_REGISTER: i32 = 40;
const LOW_AOD_MAX_REGISTER: i32 = 200;

const APPLY_LATE_MS: i32 = 100;
const SETTLE_MS: i32 = 400;
const CLASSIC_DBV_CHECK_MS: i32 = 300;
const DBV_FALLBACK_WRITE_MS: i32 = 440;
const AWAKE_RECHECK_MS: [i32; 2] = [300, 800];
const RECHECK_MS: [i32; 13] = [
    100, 120, 160, 200, 240, 280, 300, 340, 440, 600, 800, 1500, 3000,
];

#[cfg(target_os = "android")]
type PropInfo = libc::prop_info;

#[cfg(not(target_os = "android"))]
struct PropInfo;

#[repr(C)]
struct DozeRequest {
    flag: u32,
    display_id: u32,
    value: u32,
}

pub struct DozeKeeperHandle {
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DozeKeeperHandle {
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(config: AodKeepaliveConfig, running: Arc<AtomicBool>) -> DozeKeeperHandle {
    let thread_running = running.clone();
    let thread = match thread::Builder::new()
        .name("oplus-aod-doze".to_string())
        .spawn(move || {
            let mut keeper = DozeKeeper::new(config);
            keeper.run(&thread_running);
        }) {
        Ok(thread) => Some(thread),
        Err(error) => {
            warn!("Could not start the AOD doze keeper: {error}");
            running.store(false, Ordering::Release);
            None
        }
    };

    DozeKeeperHandle { running, thread }
}

struct DozeKeeper {
    config: AodKeepaliveConfig,
    disp_feature: Option<File>,
    level: u32,
    dbv: i32,
    follow_fade: bool,
    dbv_fallback_written: bool,
    awake_cleanup_done: bool,
}

impl DozeKeeper {
    fn new(config: AodKeepaliveConfig) -> Self {
        Self {
            config,
            disp_feature: None,
            level: DOZE_HIGH,
            dbv: AOD_DBV_MAX,
            follow_fade: false,
            dbv_fallback_written: false,
            awake_cleanup_done: false,
        }
    }

    fn run(&mut self, running: &AtomicBool) {
        if !Path::new(&self.config.disp_feature_path).exists() {
            warn!(
                "AOD doze keeper disabled: {} does not exist",
                self.config.disp_feature_path.display()
            );
            return;
        }

        info!(
            "AOD doze keeper started (classic level {}, DBV {})",
            self.config.classic_doze_level, self.config.classic_dbv
        );

        let Some(prop) = property_find(c"debug.tracing.screen_state") else {
            warn!("AOD doze keeper: debug.tracing.screen_state is unavailable");
            return;
        };
        let mut serial = property_serial();
        let mut previous = -1i32;

        while running.load(Ordering::Acquire) {
            let state = ScreenState::read_tracing()
                .map(ScreenState::raw)
                .unwrap_or(-1);
            let in_aod = matches!(state, 3 | 4);
            let mut dbv_pending = false;
            let mut apply_late = false;

            if in_aod {
                let entered = !matches!(previous, 3 | 4);
                if entered {
                    self.dbv_fallback_written = false;
                }
                let register = read_int(&self.config.backlight_path);

                if previous == 2 {
                    self.follow_fade = self.config.follow_fade && register >= MIN_FADE_REGISTER;
                } else if previous == 1 {
                    self.follow_fade = false;
                }

                let mut dbv = self.dbv;
                if self.follow_fade {
                    if register >= MIN_FADE_REGISTER {
                        self.level = if register < LOW_AOD_MAX_REGISTER {
                            DOZE_LOW
                        } else {
                            DOZE_HIGH
                        };
                        dbv = ((register as f32 / AOD_GAIN) + 0.5) as i32;
                        dbv = dbv.clamp(AOD_DBV_MIN, AOD_DBV_MAX);
                    }
                } else {
                    // Classic clocks and static images pass no AOD brightness.
                    let (level, classic_dbv) = classic_aod_target(&self.config);
                    self.level = level;
                    dbv = classic_dbv;
                }

                let dbv_changed = dbv != self.dbv;
                self.dbv = dbv;

                if entered || read_int(&self.config.doze_brightness_path) != self.level as i32 {
                    if entered && previous == 2 {
                        // The panel is still lit in normal mode for ~120 ms.
                        // Applying level 1 immediately can flash DBV 808.
                        apply_late = true;
                    } else {
                        self.apply(self.level);
                    }
                    dbv_pending = true;
                } else if dbv_changed {
                    self.write_aod_dbv();
                }
            } else if state == 1 {
                // The panel can remain lit briefly after screen_state flips
                // to OFF. Writing an AOD level here re-lights or dims that
                // transition, and the kernel reprograms the level on the next
                // AOD entry anyway.
                self.follow_fade = false;
                self.dbv_fallback_written = false;
                self.awake_cleanup_done = false;
            } else if state == 2 && previous != 2 {
                self.follow_fade = false;
                self.dbv_fallback_written = false;
                self.awake_cleanup_done = false;
                self.apply(DOZE_OFF);
            } else if state == -1 {
                self.follow_fade = false;
            }

            previous = state;
            serial = self.wait_for_change(prop, serial, state, dbv_pending, apply_late, running);
        }

        // Leaving doze restores normal panel brightness. Do not issue a doze
        // level from a stale state.
        if ScreenState::read_tracing().is_some_and(|state| state.is_on()) {
            self.apply(DOZE_OFF);
        }
        info!("AOD doze keeper stopped");
    }

    fn wait_for_change(
        &mut self,
        prop: *const PropInfo,
        serial: u32,
        state: i32,
        mut dbv_pending: bool,
        apply_late: bool,
        running: &AtomicBool,
    ) -> u32 {
        let mut next = serial;
        let mut elapsed = 0i32;
        let in_aod = matches!(state, 3 | 4);
        let awake = state == 2;

        for at in RECHECK_MS {
            if !running.load(Ordering::Acquire) {
                return next;
            }

            let late_at = at == APPLY_LATE_MS;
            let classic = in_aod && !self.follow_fade;
            // Only correct a level when the driver gives a real readback. If
            // the node is absent, repeatedly issuing the ioctl re-sends the
            // command set's normal-mode DBV 808 and produces the black-screen
            // flash while entering AOD.
            let level_at = !late_at
                && classic
                && matches!(at, 300 | 800 | 1500 | 3000)
                && read_int(&self.config.doze_brightness_path) >= 0;
            let dbv_at =
                !late_at && in_aod && (dbv_pending || (classic && at == CLASSIC_DBV_CHECK_MS));
            let awake_at = !late_at && awake && AWAKE_RECHECK_MS.contains(&at);

            if (late_at && !apply_late) || (!late_at && !level_at && !dbv_at && !awake_at) {
                continue;
            }

            let gap = (at - elapsed).max(1) as u64;
            elapsed = at;
            if property_wait(prop, serial, &mut next, Some(gap)) {
                return next;
            }

            if late_at {
                self.apply(self.level);
                continue;
            }

            if awake_at && !self.awake_cleanup_done {
                let live_state = ScreenState::read_tracing();
                let doze_level = read_int(&self.config.doze_brightness_path);
                if should_retry_awake_cleanup(live_state, doze_level) {
                    debug!(
                        "AOD level still present after wake ({doze_level}); reapplying DOZE_OFF"
                    );
                    self.apply(DOZE_OFF);
                    self.awake_cleanup_done = true;
                }
            }

            if dbv_at && self.follow_fade {
                let register = read_int(&self.config.backlight_path);
                if should_reclassify_fade(self.follow_fade, dbv_pending, register) {
                    // A full-screen fade that collapsed to SystemUI's fallback
                    // is actually a classic/static style. Check this at every
                    // dense DBV point: waiting until 300 ms leaves a black
                    // frame/mask visible during the ON -> AOD transition.
                    debug!(
                        "Full-screen AOD fade collapsed at {at} ms (backlight {register}); using classic AOD"
                    );
                    self.follow_fade = false;
                    let (level, classic_dbv) = classic_aod_target(&self.config);
                    self.dbv = classic_dbv;
                    if self.level != level {
                        self.level = level;
                        self.apply(level);
                    }
                    dbv_pending = true;
                }
            }

            if level_at && read_int(&self.config.doze_brightness_path) != self.level as i32 {
                self.apply(self.level);
                dbv_pending = true;
            }

            if dbv_at {
                let fallback_allowed = at == DBV_FALLBACK_WRITE_MS;
                if self.sync_dbv(at >= SETTLE_MS, fallback_allowed) && at >= SETTLE_MS {
                    dbv_pending = false;
                }
            }
        }

        // A bounded wait keeps stop responsive while still sleeping on the
        // property futex instead of polling every frame.
        if property_wait(prop, serial, &mut next, Some(1000)) {
            return next;
        }
        next
    }

    fn sync_dbv(&mut self, settled: bool, fallback_allowed: bool) -> bool {
        if !ScreenState::read_tracing().is_some_and(ScreenState::is_doze) {
            return true;
        }

        let dbv = self.read_dbv();
        if dbv == self.dbv {
            return true;
        }
        if dbv < 0 {
            // Some mi_display kernels do not implement DCS readback. Wait
            // until the panel has definitely entered AOD, then make at most
            // one fallback write per AOD entry. Repeating this at 1.5/3 s can
            // visibly step the brightness of static/classic AOD styles.
            if should_write_dbv_fallback(settled, fallback_allowed, self.dbv_fallback_written)
                && self.write_dbv(self.dbv)
            {
                debug!("AOD DCS readback unavailable; wrote one settled DBV fallback");
                self.dbv_fallback_written = true;
                return true;
            }
            return false;
        }
        // A framework fallback can make DCS read back the same low value as
        // the normal backlight node. Once the transition has settled, that is
        // not proof the panel is still in normal mode; reassert the AOD DBV.
        if should_write_dbv(
            dbv,
            self.dbv,
            read_int(&self.config.backlight_path),
            settled,
        ) && self.write_dbv(self.dbv)
        {
            return true;
        }
        false
    }

    fn write_aod_dbv(&self) {
        if ScreenState::read_tracing().is_some_and(ScreenState::is_doze) {
            let _ = self.write_dbv(self.dbv);
        }
    }

    fn write_dbv(&self, dbv: i32) -> bool {
        let command = format!(
            "00 00 00 39 00 00 00 00 00 03 51 {:02X} {:02X}",
            (dbv >> 8) & 0xff,
            dbv & 0xff
        );
        match write_mipi(&self.config.mipi_rw_path, &command) {
            Ok(()) => true,
            Err(error) => {
                debug!(
                    "mi_display DBV write failed through {}: {error}",
                    self.config.mipi_rw_path.display()
                );
                false
            }
        }
    }

    fn read_dbv(&self) -> i32 {
        if write_mipi(
            &self.config.mipi_rw_path,
            "01 02 00 06 01 00 00 00 00 01 52",
        )
        .is_err()
        {
            return -1;
        }

        let mut file = match File::open(&self.config.mipi_rw_path) {
            Ok(file) => file,
            Err(_) => return -1,
        };
        let mut buffer = String::new();
        if file.read_to_string(&mut buffer).is_err() {
            return -1;
        }
        parse_dbv(&buffer).unwrap_or(-1)
    }

    fn apply(&mut self, value: u32) {
        for attempt in 0..25 {
            if !permits_doze_level(ScreenState::read_tracing(), value) {
                return;
            }
            if self.apply_once(value) {
                return;
            }
            if attempt < 24 {
                thread::sleep(Duration::from_millis(150));
            }
        }
        debug!("mi_display rejected doze level {value} after 25 attempts");
    }

    fn apply_once(&mut self, value: u32) -> bool {
        if self.disp_feature.is_none() {
            self.disp_feature = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.config.disp_feature_path)
                .map_err(|error| {
                    warn!(
                        "Cannot open {}: {error}",
                        self.config.disp_feature_path.display()
                    );
                    error
                })
                .ok();
        }
        let Some(file) = self.disp_feature.as_ref() else {
            return false;
        };

        let mut request = DozeRequest {
            flag: 0,
            display_id: 0,
            value,
        };
        let result = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                MI_DISP_IOCTL_SET_DOZE_BRIGHTNESS as _,
                &mut request as *mut DozeRequest,
            )
        };
        result == 0
    }
}

fn should_write_dbv(observed: i32, target: i32, backlight: i32, settled: bool) -> bool {
    observed >= 0 && observed != target && (settled || observed != backlight)
}

fn should_write_dbv_fallback(settled: bool, fallback_allowed: bool, already_written: bool) -> bool {
    settled && fallback_allowed && !already_written
}

fn should_reclassify_fade(follow_fade: bool, dbv_pending: bool, register: i32) -> bool {
    follow_fade && dbv_pending && (0..MIN_FADE_REGISTER).contains(&register)
}

fn classic_aod_target(config: &AodKeepaliveConfig) -> (u32, i32) {
    (
        config.classic_doze_level.clamp(DOZE_HIGH, DOZE_LOW),
        config
            .classic_dbv
            .clamp(AOD_DBV_MIN as u32, AOD_DBV_MAX as u32) as i32,
    )
}

fn should_retry_awake_cleanup(state: Option<ScreenState>, doze_level: i32) -> bool {
    state.is_some_and(ScreenState::is_on) && doze_level != 0
}

fn permits_doze_level(state: Option<ScreenState>, level: u32) -> bool {
    match level {
        DOZE_OFF => state.is_some_and(ScreenState::is_on),
        DOZE_HIGH | DOZE_LOW => state.is_some_and(ScreenState::is_doze),
        _ => false,
    }
}

fn read_int(path: &Path) -> i32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(-1)
}

fn write_mipi(path: &Path, command: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.write_all(command.as_bytes())
}

fn parse_dbv(text: &str) -> Option<i32> {
    let start = text.find("0x")?;
    let mut values = text[start..]
        .split(|character: char| !character.is_ascii_hexdigit() && character != 'x')
        .filter(|part| part.starts_with("0x") && part.len() > 2)
        .filter_map(|part| u32::from_str_radix(&part[2..], 16).ok());
    let high = values.next()?;
    let low = values.next()?;
    Some(((high << 8) | low) as i32)
}

#[cfg(target_os = "android")]
fn property_find(name: &std::ffi::CStr) -> Option<*const PropInfo> {
    let info = unsafe { libc::__system_property_find(name.as_ptr()) };
    (!info.is_null()).then_some(info)
}

#[cfg(not(target_os = "android"))]
fn property_find(_name: &std::ffi::CStr) -> Option<*const PropInfo> {
    None
}

#[cfg(target_os = "android")]
fn property_serial() -> u32 {
    let mut serial = 0u32;
    unsafe {
        libc::__system_property_wait(std::ptr::null(), 0, &mut serial, std::ptr::null());
    }
    serial
}

#[cfg(not(target_os = "android"))]
fn property_serial() -> u32 {
    0
}

#[cfg(target_os = "android")]
fn property_wait(
    info: *const PropInfo,
    serial: u32,
    next: &mut u32,
    timeout_ms: Option<u64>,
) -> bool {
    let timeout = timeout_ms.map(|milliseconds| libc::timespec {
        tv_sec: (milliseconds / 1000) as libc::time_t,
        tv_nsec: ((milliseconds % 1000) * 1_000_000) as libc::c_long,
    });
    let timeout_ptr = timeout
        .as_ref()
        .map_or(std::ptr::null(), |value| value as *const libc::timespec);
    unsafe { libc::__system_property_wait(info, serial, next, timeout_ptr) }
}

#[cfg(not(target_os = "android"))]
fn property_wait(
    _info: *const PropInfo,
    _serial: u32,
    _next: &mut u32,
    timeout_ms: Option<u64>,
) -> bool {
    thread::sleep(Duration::from_millis(timeout_ms.unwrap_or(1000)));
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mipi_brightness_readback() {
        assert_eq!(parse_dbv("0x03 ,0x28"), Some(808));
        assert_eq!(parse_dbv("noise 0x00 ,0x10 trailing"), Some(16));
        assert_eq!(parse_dbv("busy"), None);
    }

    #[test]
    fn doze_levels_are_state_guarded() {
        assert!(permits_doze_level(Some(ScreenState::On), DOZE_OFF));
        assert!(permits_doze_level(Some(ScreenState::Doze), DOZE_HIGH));
        assert!(permits_doze_level(Some(ScreenState::DozeSuspend), DOZE_LOW));
        assert!(!permits_doze_level(Some(ScreenState::On), DOZE_HIGH));
        assert!(!permits_doze_level(Some(ScreenState::Off), DOZE_OFF));
        assert!(!permits_doze_level(Some(ScreenState::Doze), DOZE_OFF));
        assert!(!permits_doze_level(None, DOZE_HIGH));
    }

    #[test]
    fn settled_framework_fallback_reasserts_the_aod_dbv() {
        // During the transition the panel can still report the normal-mode
        // register, so an equal backlight value must not be written yet.
        assert!(!should_write_dbv(15, 808, 15, false));
        // Once settled, DCS 15 is SystemUI's fallback and will leave a
        // classic AOD almost black unless the keeper corrects it.
        assert!(should_write_dbv(15, 808, 15, true));
        assert!(should_write_dbv(42, 808, 5, false));
        assert!(!should_write_dbv(808, 808, 15, true));
        assert!(!should_write_dbv(-1, 808, 15, true));
    }

    #[test]
    fn unavailable_dbv_readback_has_bounded_settled_retries() {
        assert!(!should_write_dbv_fallback(false, true, false));
        assert!(!should_write_dbv_fallback(true, false, false));
        assert!(should_write_dbv_fallback(true, true, false));
        assert!(!should_write_dbv_fallback(true, true, true));
    }

    #[test]
    fn collapsed_full_screen_fade_switches_to_classic_immediately() {
        assert!(should_reclassify_fade(true, true, 8));
        assert!(should_reclassify_fade(true, true, 0));
        assert!(!should_reclassify_fade(true, true, MIN_FADE_REGISTER));
        assert!(!should_reclassify_fade(true, false, 8));
        assert!(!should_reclassify_fade(false, true, 8));
        assert!(!should_reclassify_fade(true, true, -1));
    }

    #[test]
    fn classic_target_is_clamped_once() {
        let mut config = AodKeepaliveConfig::default();
        assert_eq!(classic_aod_target(&config), (DOZE_HIGH, AOD_DBV_MAX));

        config.classic_doze_level = 99;
        config.classic_dbv = 1;
        assert_eq!(classic_aod_target(&config), (DOZE_LOW, AOD_DBV_MIN));

        config.classic_doze_level = 0;
        config.classic_dbv = 9999;
        assert_eq!(classic_aod_target(&config), (DOZE_HIGH, AOD_DBV_MAX));
    }

    #[test]
    fn awake_cleanup_only_retries_on_a_live_lockscreen() {
        assert!(should_retry_awake_cleanup(Some(ScreenState::On), -1));
        assert!(should_retry_awake_cleanup(
            Some(ScreenState::On),
            DOZE_HIGH as i32
        ));
        assert!(!should_retry_awake_cleanup(Some(ScreenState::On), 0));
        assert!(!should_retry_awake_cleanup(Some(ScreenState::Doze), -1));
        assert!(!should_retry_awake_cleanup(None, -1));
    }
}
