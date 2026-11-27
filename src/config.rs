//! Compiled-in configuration for the touch HAL.
//!
//! The service deliberately has no runtime config file. Device-specific
//! defaults live here and are embedded directly into the executable.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TouchConfig {
    /// Path to xiaomi-touch device
    pub xiaomi_touch_path: PathBuf,

    /// Path to tp_gesture procfs entry
    pub tp_gesture_path: PathBuf,

    /// Path to touchscreen device for gestures
    pub touchscreen_path: PathBuf,

    /// Enable AOD (Always On Display)
    pub enable_aod: bool,

    /// Enable DT2W (Double Tap to Wake)
    pub enable_dt2w: bool,

    /// Enable gestures
    pub enable_gestures: bool,

    /// Enable raise to wake
    pub enable_raise_to_wake: bool,

    /// Allow Linux KEY_WAKEUP through uinput as a fallback when the explicit
    /// Android power-wake request cannot be delivered.
    pub enable_uinput_wake: bool,

    /// Enable full screen AOD
    pub enable_full_screen_aod: bool,

    /// Enable seamless AOD
    pub enable_seamless_aod: bool,

    /// Keep the Xiaomi mi_display panel programmed while it is in doze.
    pub aod_keepalive: AodKeepaliveConfig,

    /// Apply the OPlus AOD defaults once, after the framework has booted.
    pub aod_defaults: AodDefaultsConfig,

    /// Answer OPlus SystemUI's display-panel capability queries. Without
    /// this service, Android treats panoramic AOD as energy-saving AOD and
    /// hides it after a few seconds.
    pub aod_panel_service: AodPanelServiceConfig,

    /// AOD brightness level (0-255)
    pub aod_brightness: u8,

    /// Gesture configurations
    pub gestures: GestureConfig,

    /// Raise to wake config
    pub raise_to_wake: RaiseToWakeConfig,

    /// Sysfs paths for touch features
    pub sysfs: SysfsConfig,
}

#[derive(Debug, Clone)]
pub struct GestureConfig {
    /// Double tap gesture
    pub double_tap: GestureEntry,

    /// Single tap gesture
    pub single_tap: GestureEntry,

    /// Swipe up gesture
    pub swipe_up: GestureEntry,

    /// Swipe down gesture
    pub swipe_down: GestureEntry,

    /// Swipe left gesture
    pub swipe_left: GestureEntry,

    /// Swipe right gesture
    pub swipe_right: GestureEntry,

    /// Custom gestures
    pub custom: Vec<GestureEntry>,
}

#[derive(Debug, Clone)]
pub struct GestureEntry {
    pub enabled: bool,
    pub gesture_id: u8,
    pub action: String,
    pub wake_screen: bool,
}

#[derive(Debug, Clone)]
pub struct RaiseToWakeConfig {
    pub enabled: bool,

    /// Android sensor string type used by the Xiaomi sensor hub.
    pub sensor_type: String,

    /// Sensor-manager package identity used by the working OPlus binary.
    pub sensor_package: String,

    /// Prefer the wake-up variant when both pickup sensors are present. The
    /// proven OPlus binary selects the non-wake-up variant first.
    pub prefer_wakeup_sensor: bool,

    /// Require the OPlus Settings switch before arming the sensor.
    pub require_setting: bool,

    pub setting_namespace: String,

    pub setting_key: String,

    /// Ignore the sensor's initial state report for this long after enabling.
    pub arm_grace_ms: u64,

    /// Repeated pickup reports inside this interval are ignored.
    pub debounce_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AodKeepaliveConfig {
    pub enabled: bool,

    /// Doze level used by clock/static-image styles that provide no AOD DBV.
    pub classic_doze_level: u32,

    /// Panel DBV used by those styles. 808 is the panel's normal AOD level.
    pub classic_dbv: u32,

    /// Match a full-screen AOD entry fade when the framework provides one.
    pub follow_fade: bool,

    pub disp_feature_path: PathBuf,

    pub backlight_path: PathBuf,

    pub doze_brightness_path: PathBuf,

    pub mipi_rw_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AodDefaultsConfig {
    pub enabled: bool,

    /// Wait this long after `sys.boot_completed=1` before touching settings.
    pub delay_seconds: u64,

    /// Marker written after the one-time full-screen/all-day defaults succeed.
    pub marker: String,
}

#[derive(Debug, Clone)]
pub struct AodPanelServiceConfig {
    pub enabled: bool,

    /// Stable AIDL service instance queried by OPlus SystemUI.
    pub service_name: String,
}

#[derive(Debug, Clone)]
pub struct SysfsConfig {
    /// Touch mode sysfs path
    pub touch_mode_path: PathBuf,

    /// AOD enable sysfs path
    pub aod_enable_path: PathBuf,

    /// DT2W enable sysfs path
    pub dt2w_enable_path: PathBuf,

    /// Gesture enable sysfs path
    pub gesture_enable_path: PathBuf,

    /// Panel vendor sysfs path
    pub panel_vendor_path: PathBuf,

    /// FOD enable sysfs path
    pub fod_enable_path: PathBuf,

    /// Touch report rate path
    pub report_rate_path: PathBuf,

    /// Game mode path
    pub game_mode_path: PathBuf,

    /// Optional AOD brightness/doze controls. Missing nodes are skipped.
    pub aod_brightness_path: Option<PathBuf>,

    pub aod_full_screen_path: Option<PathBuf>,

    pub aod_seamless_path: Option<PathBuf>,

    pub aod_expand_music_path: Option<PathBuf>,

    pub wake_screen_path: Option<PathBuf>,

    pub raise_to_wake_enable_path: Option<PathBuf>,
}

fn default_pickup_sensor_type() -> String {
    "xiaomi.sensor.pickup".to_string()
}

fn default_raise_package() -> String {
    "vendor.oplus.hardware.raise2wake".to_string()
}

fn default_raise_setting_namespace() -> String {
    "secure".to_string()
}

fn default_raise_setting_key() -> String {
    "oplus_customize_gesture_wake_up_arouse".to_string()
}

fn default_raise_arm_grace_ms() -> u64 {
    300
}

fn default_raise_debounce_ms() -> u64 {
    500
}

fn default_classic_doze_level() -> u32 {
    1
}

fn default_classic_aod_dbv() -> u32 {
    808
}

fn default_disp_feature_path() -> PathBuf {
    PathBuf::from("/dev/mi_display/disp_feature")
}

fn default_backlight_path() -> PathBuf {
    PathBuf::from("/sys/class/backlight/panel0-backlight/brightness")
}

fn default_doze_brightness_path() -> PathBuf {
    PathBuf::from("/sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/doze_brightness")
}

fn default_mipi_rw_path() -> PathBuf {
    PathBuf::from("/sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/mipi_rw")
}

fn default_aod_defaults_delay_s() -> u64 {
    15
}

fn default_aod_defaults_marker() -> String {
    // v3 intentionally differs from v2: the corrected list must be applied
    // once on devices which already ran an earlier build.
    "oplus_touch_hal_aod_defaults_v3".to_string()
}

fn default_panel_service_name() -> String {
    "vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default".to_string()
}

impl Default for TouchConfig {
    fn default() -> Self {
        Self {
            xiaomi_touch_path: PathBuf::from("/dev/xiaomi-touch"),
            tp_gesture_path: PathBuf::from("/proc/tp_gesture"),
            touchscreen_path: PathBuf::from("/dev/input/touchscreen0"),
            enable_aod: true,
            enable_dt2w: true,
            enable_gestures: true,
            enable_raise_to_wake: true,
            enable_uinput_wake: true,
            enable_full_screen_aod: true,
            enable_seamless_aod: true,
            aod_keepalive: AodKeepaliveConfig::default(),
            aod_defaults: AodDefaultsConfig::default(),
            aod_panel_service: AodPanelServiceConfig::default(),
            aod_brightness: 50,
            gestures: GestureConfig::default(),
            raise_to_wake: RaiseToWakeConfig::default(),
            sysfs: SysfsConfig::default(),
        }
    }
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            double_tap: GestureEntry {
                enabled: true,
                gesture_id: 0x22,
                action: "wake".to_string(),
                wake_screen: true,
            },
            single_tap: GestureEntry {
                enabled: true,
                gesture_id: 0x23,
                action: "aod_preview".to_string(),
                wake_screen: false,
            },
            swipe_up: GestureEntry {
                enabled: true,
                gesture_id: 0x24,
                action: "unlock".to_string(),
                wake_screen: true,
            },
            swipe_down: GestureEntry {
                enabled: true,
                gesture_id: 0x25,
                action: "notification".to_string(),
                wake_screen: true,
            },
            swipe_left: GestureEntry {
                enabled: true,
                gesture_id: 0x26,
                action: "camera".to_string(),
                wake_screen: true,
            },
            swipe_right: GestureEntry {
                enabled: true,
                gesture_id: 0x27,
                action: "none".to_string(),
                wake_screen: false,
            },
            custom: vec![],
        }
    }
}

impl Default for RaiseToWakeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sensor_type: default_pickup_sensor_type(),
            sensor_package: default_raise_package(),
            prefer_wakeup_sensor: false,
            require_setting: true,
            setting_namespace: default_raise_setting_namespace(),
            setting_key: default_raise_setting_key(),
            arm_grace_ms: default_raise_arm_grace_ms(),
            debounce_ms: default_raise_debounce_ms(),
        }
    }
}

impl Default for AodKeepaliveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            classic_doze_level: default_classic_doze_level(),
            classic_dbv: default_classic_aod_dbv(),
            follow_fade: true,
            disp_feature_path: default_disp_feature_path(),
            backlight_path: default_backlight_path(),
            doze_brightness_path: default_doze_brightness_path(),
            mipi_rw_path: default_mipi_rw_path(),
        }
    }
}

impl Default for AodDefaultsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            delay_seconds: default_aod_defaults_delay_s(),
            marker: default_aod_defaults_marker(),
        }
    }
}

impl Default for AodPanelServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            service_name: default_panel_service_name(),
        }
    }
}

impl Default for SysfsConfig {
    fn default() -> Self {
        Self {
            touch_mode_path: PathBuf::from("/sys/class/touch/touch_mode"),
            aod_enable_path: PathBuf::from("/sys/class/touch/aod_enable"),
            dt2w_enable_path: PathBuf::from("/sys/class/touch/dt2w_enable"),
            gesture_enable_path: PathBuf::from("/sys/class/touch/gesture_enable"),
            panel_vendor_path: PathBuf::from("/sys/class/touch/panel_vendor"),
            fod_enable_path: PathBuf::from("/sys/class/touch/fod_enable"),
            report_rate_path: PathBuf::from("/sys/class/touch/report_rate"),
            game_mode_path: PathBuf::from("/sys/class/touch/game_mode"),
            aod_brightness_path: None,
            aod_full_screen_path: None,
            aod_seamless_path: None,
            aod_expand_music_path: None,
            wake_screen_path: None,
            raise_to_wake_enable_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procfs::gesture_ids;

    #[test]
    fn standalone_defaults_enable_required_features() {
        let config = TouchConfig::default();

        assert_eq!(config.tp_gesture_path, PathBuf::from("/proc/tp_gesture"));
        assert!(config.enable_aod);
        assert!(config.enable_dt2w);
        assert!(config.enable_gestures);
        assert!(config.enable_raise_to_wake);
        assert!(config.enable_uinput_wake);
        assert!(config.enable_full_screen_aod);
        assert!(config.enable_seamless_aod);
        assert!(config.aod_keepalive.enabled);
        assert!(config.aod_defaults.enabled);
        assert!(config.aod_panel_service.enabled);
        assert!(config.gestures.double_tap.enabled);
        assert_eq!(
            config.gestures.double_tap.gesture_id,
            gesture_ids::DOUBLE_TAP
        );
    }

    #[test]
    fn aod_defaults_marker_is_versioned_for_reapplication() {
        assert_eq!(
            TouchConfig::default().aod_defaults.marker,
            "oplus_touch_hal_aod_defaults_v3"
        );
    }
}
