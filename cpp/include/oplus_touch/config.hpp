#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace oplus::touch {

struct GestureEntry {
    bool enabled = false;
    uint8_t gesture_id = 0;
    std::string action;
    bool wake_screen = false;
};

struct GestureConfig {
    GestureEntry double_tap{true, 0x22, "wake", true};
    GestureEntry single_tap{true, 0x23, "aod_preview", false};
    GestureEntry swipe_up{true, 0x24, "unlock", true};
    GestureEntry swipe_down{true, 0x25, "notification", true};
    GestureEntry swipe_left{true, 0x26, "camera", true};
    GestureEntry swipe_right{true, 0x27, "none", false};
    std::vector<GestureEntry> custom;
};

struct RaiseToWakeConfig {
    bool enabled = true;
    std::string sensor_type = "xiaomi.sensor.pickup";
    std::string sensor_package = "vendor.oplus.hardware.raise2wake";
    bool prefer_wakeup_sensor = false;
    bool require_setting = true;
    std::string setting_namespace = "secure";
    std::string setting_key = "oplus_customize_gesture_wake_up_arouse";
    uint64_t arm_grace_ms = 300;
    uint64_t debounce_ms = 500;
};

struct AodKeepaliveConfig {
    bool enabled = true;
    uint32_t classic_doze_level = 1;
    uint32_t classic_dbv = 808;
    bool follow_fade = true;
    std::string disp_feature_path = "/dev/mi_display/disp_feature";
    std::string backlight_path = "/sys/class/backlight/panel0-backlight/brightness";
    std::string doze_brightness_path =
        "/sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/doze_brightness";
    std::string mipi_rw_path =
        "/sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/mipi_rw";
};

struct AodDefaultsConfig {
    bool enabled = true;
    uint64_t delay_seconds = 15;
    std::string marker = "oplus_touch_hal_aod_defaults_v3";
};

struct AodPanelServiceConfig {
    bool enabled = true;
    std::string service_name =
        "vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default";
};

struct SysfsConfig {
    std::string touch_mode_path = "/sys/class/touch/touch_mode";
    std::string aod_enable_path = "/sys/class/touch/aod_enable";
    std::string dt2w_enable_path = "/sys/class/touch/dt2w_enable";
    std::string gesture_enable_path = "/sys/class/touch/gesture_enable";
    std::string panel_vendor_path = "/sys/class/touch/panel_vendor";
    std::string fod_enable_path = "/sys/class/touch/fod_enable";
    std::string report_rate_path = "/sys/class/touch/report_rate";
    std::string game_mode_path = "/sys/class/touch/game_mode";
    std::optional<std::string> aod_brightness_path;
    std::optional<std::string> aod_full_screen_path;
    std::optional<std::string> aod_seamless_path;
    std::optional<std::string> aod_expand_music_path;
    std::optional<std::string> wake_screen_path;
    std::optional<std::string> raise_to_wake_enable_path;
};

struct TouchConfig {
    std::string xiaomi_touch_path = "/dev/xiaomi-touch";
    std::string tp_gesture_path = "/proc/tp_gesture";
    std::string touchscreen_path = "/dev/input/touchscreen0";

    bool enable_aod = true;
    bool enable_dt2w = true;
    bool enable_gestures = true;
    bool enable_raise_to_wake = true;
    bool enable_uinput_wake = true;
    bool enable_full_screen_aod = true;
    bool enable_seamless_aod = true;

    AodKeepaliveConfig aod_keepalive;
    AodDefaultsConfig aod_defaults;
    AodPanelServiceConfig aod_panel_service;
    uint8_t aod_brightness = 50;
    GestureConfig gestures;
    RaiseToWakeConfig raise_to_wake;
    SysfsConfig sysfs;
};

TouchConfig default_config();

}  // namespace oplus::touch
