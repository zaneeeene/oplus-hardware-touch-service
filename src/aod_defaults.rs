//! One-time OPlus AOD settings bootstrap.
//!
//! The OPlus AOD application resets full-screen/always-on choices early in a
//! fresh port. These vendor settings are the same defaults used by the
//! working OPlus sensor bridge. They are applied only once, guarded by a
//! marker, so later changes made in Settings are never overwritten.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::{debug, info, warn};

use crate::config::AodDefaultsConfig;

const AOD_DEFAULTS: &[(&str, &str)] = &[
    ("Setting_AodOldDataHasBeenReset", "1"),
    ("Setting_AodEnable", "1"),
    ("Setting_AodSwitchEnable", "1"),
    ("Setting_AodEnableImmediate", "1"),
    ("Setting_AodUserSetTime", "0"),
    ("Setting_AodUserEnergySavingSet", "0"),
    ("Setting_AodAdditionalTime", "1"),
    ("Setting_AodAdditionalNotification", "1"),
    ("Setting_AodAdditionalBattery", "1"),
    ("Setting_AodAdditionalDate", "1"),
    ("Setting_AodAdditionalLunar", "1"),
    ("Setting_AodAdditionalUV", "1"),
    ("Setting_AodSceneInfoSwitchEnable", "1"),
    ("Setting_AodSceneMusicSwitchEnable", "1"),
    ("Setting_Sub_AodSwitchEnable", "1"),
    ("Setting_Sub_AodEnableImmediate", "1"),
    ("Setting_Sub_AodFullScreen", "1"),
    ("Setting_Sub_AodUserEnergySavingSet", "0"),
    ("oplus_keyguard_panoramic_aod_support", "1"),
    ("oplus_keyguard_panoramic_aod_all_day_support", "1"),
    ("oplus_keyguard_workshop_aod_support", "1"),
    ("aod_media_support", "1"),
    ("full_screen_aod", "0"),
    ("Setting_AodStyleInfo", r#"{"type":2,"uuid":""}"#),
    ("panoramic_aod_enable", "1"),
    ("panoramic_aod_rate", "2000"),
    ("sub_display_aod_on_to_doze_switch", "1"),
];

// These flags describe panel/SystemUI capabilities rather than a clock style.
// OPlus components rewrite them during startup, so restoring them only once
// per flash makes full-screen all-day AOD disappear after a later boot.
const EVERY_BOOT: &[(&str, &str)] = &[
    ("Setting_AodOldDataHasBeenReset", "1"),
    ("oplus_keyguard_panoramic_aod_support", "1"),
    ("oplus_keyguard_panoramic_aod_all_day_support", "1"),
    ("oplus_keyguard_workshop_aod_support", "1"),
    ("aod_media_support", "1"),
    ("panoramic_aod_enable", "1"),
    ("sub_display_aod_on_to_doze_switch", "1"),
];

pub struct AodDefaultsHandle {
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AodDefaultsHandle {
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(config: AodDefaultsConfig, running: Arc<AtomicBool>) -> AodDefaultsHandle {
    let thread_running = running.clone();
    let thread = match thread::Builder::new()
        .name("oplus-aod-defaults".to_string())
        .spawn(move || run(config, &thread_running))
    {
        Ok(thread) => Some(thread),
        Err(error) => {
            warn!("Could not start the AOD defaults worker: {error}");
            running.store(false, Ordering::Release);
            None
        }
    };
    AodDefaultsHandle { running, thread }
}

fn run(config: AodDefaultsConfig, running: &AtomicBool) {
    if !wait_for_boot_completed(running) {
        return;
    }

    for _ in 0..config.delay_seconds.max(1) {
        if !running.load(Ordering::Acquire) {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }

    // Capability flags are rewritten by SystemUI/AOD during every boot.
    apply_every_boot();

    let marker = read_secure_setting(&config.marker);
    if marker.as_deref() != Some("1") {
        info!("Applying one-time OPlus full-screen/all-day AOD defaults");
        let mut success = true;
        for &(key, value) in AOD_DEFAULTS {
            success &= put_secure_setting(key, value);
        }
        if success && put_secure_setting(&config.marker, "1") {
            info!("OPlus AOD defaults applied");
        } else {
            warn!("Some OPlus AOD defaults failed; they will be retried on next boot");
        }
    } else {
        debug!("OPlus AOD defaults marker is already set");
    }

    // SystemUI's startup checks can run a few seconds after boot completed.
    for _ in 0..10 {
        if !running.load(Ordering::Acquire) {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
    apply_every_boot();
}

fn apply_every_boot() {
    for &(key, value) in EVERY_BOOT {
        match read_secure_setting(key) {
            Some(current) if current == value => {}
            _ => {
                if !put_secure_setting(key, value) {
                    debug!("Could not set every-boot AOD setting {key}");
                }
            }
        }
    }
}

fn read_secure_setting(key: &str) -> Option<String> {
    let output = Command::new("/system/bin/cmd")
        .args(["settings", "get", "secure", key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn put_secure_setting(key: &str, value: &str) -> bool {
    let result = Command::new("/system/bin/cmd")
        .args(["settings", "put", "secure", key, value])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(result, Ok(status) if status.success())
}

fn wait_for_boot_completed(running: &AtomicBool) -> bool {
    for _ in 0..300 {
        if !running.load(Ordering::Acquire) {
            return false;
        }
        if property_get(c"sys.boot_completed").as_deref() == Some("1") {
            return true;
        }
        thread::sleep(Duration::from_secs(1));
    }
    warn!("Timed out waiting for sys.boot_completed; AOD defaults were not applied");
    false
}

#[cfg(target_os = "android")]
fn property_get(name: &std::ffi::CStr) -> Option<String> {
    use std::ffi::CStr;

    let mut value = [0 as libc::c_char; 92];
    let length = unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if length <= 0 {
        return None;
    }
    let text = unsafe { CStr::from_ptr(value.as_ptr()) }.to_str().ok()?;
    Some(text.to_string())
}

#[cfg(not(target_os = "android"))]
fn property_get(_name: &std::ffi::CStr) -> Option<String> {
    None
}
