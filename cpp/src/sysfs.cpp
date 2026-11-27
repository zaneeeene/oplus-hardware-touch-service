#include "oplus_touch/sysfs.hpp"

#include "oplus_touch/common.hpp"

#include <cerrno>
#include <cstdlib>
#include <limits>

namespace oplus::touch {
namespace {

std::optional<std::string> optional_path(const std::optional<std::string>& path,
                                         const std::string& feature) {
    if (!path) {
        log_message(LogLevel::Debug,
                    feature + " is not exposed by this kernel; configure its sysfs path if present");
    }
    return path;
}

}  // namespace

std::optional<std::string> SysfsManager::get_path(const std::string& name) const {
    if (name == "touch_mode") {
        return config_.touch_mode_path;
    }
    if (name == "aod_enable") {
        return config_.aod_enable_path;
    }
    if (name == "aod_brightness") {
        if (config_.aod_brightness_path && path_exists(*config_.aod_brightness_path)) {
            return config_.aod_brightness_path;
        }
        static constexpr const char* candidates[] = {
            "/sys/class/touch/aod_brightness",
            "/sys/class/touch/touch_dev/aod_brightness",
        };
        for (const char* candidate : candidates) {
            if (path_exists(candidate)) {
                return std::string(candidate);
            }
        }
        log_message(LogLevel::Debug, "no AOD brightness node found");
        return std::nullopt;
    }
    if (name == "aod_full_screen") {
        return optional_path(config_.aod_full_screen_path, "full-screen AOD");
    }
    if (name == "aod_seamless") {
        return optional_path(config_.aod_seamless_path, "seamless AOD");
    }
    if (name == "aod_expand_music") {
        return optional_path(config_.aod_expand_music_path, "AOD music expansion");
    }
    if (name == "wake_screen") {
        return optional_path(config_.wake_screen_path, "touch wake node");
    }
    if (name == "dt2w_enable") {
        return config_.dt2w_enable_path;
    }
    if (name == "gesture_enable") {
        return config_.gesture_enable_path;
    }
    if (name == "panel_vendor") {
        return config_.panel_vendor_path;
    }
    if (name == "fod_enable") {
        return config_.fod_enable_path;
    }
    if (name == "report_rate") {
        return config_.report_rate_path;
    }
    if (name == "game_mode") {
        return config_.game_mode_path;
    }
    if (name == "raise_to_wake_enable") {
        return optional_path(config_.raise_to_wake_enable_path, "raise-to-wake");
    }
    log_message(LogLevel::Debug, "Unknown sysfs entry: " + name);
    return std::nullopt;
}

bool SysfsManager::write_bool(const std::string& name, bool value) {
    return write_string(name, value ? "1" : "0");
}

bool SysfsManager::write_u8(const std::string& name, uint8_t value) {
    return write_string(name, std::to_string(value));
}

bool SysfsManager::write_u32(const std::string& name, uint32_t value) {
    return write_string(name, std::to_string(value));
}

bool SysfsManager::write_string(const std::string& name, const std::string& value) {
    auto path = get_path(name);
    if (!path) {
        return false;
    }
    if (!write_file(*path, value)) {
        log_message(LogLevel::Error, "Failed to write " + value + " to " + *path);
        return false;
    }
    return true;
}

std::optional<std::string> SysfsManager::read_string(const std::string& name) const {
    auto path = get_path(name);
    if (!path) {
        return std::nullopt;
    }
    auto content = read_file(*path);
    if (!content) {
        log_message(LogLevel::Error, "Failed to read " + *path);
    }
    return content;
}

std::optional<bool> SysfsManager::read_bool(const std::string& name) const {
    auto content = read_string(name);
    if (!content) {
        return std::nullopt;
    }
    return trim(*content) == "1";
}

std::optional<uint8_t> SysfsManager::read_u8(const std::string& name) const {
    auto content = read_string(name);
    if (!content) {
        return std::nullopt;
    }
    const std::string text = trim(*content);
    char* end = nullptr;
    errno = 0;
    const unsigned long value = std::strtoul(text.c_str(), &end, 10);
    if (errno != 0 || end == text.c_str() || *end != '\0' ||
        value > std::numeric_limits<uint8_t>::max()) {
        return std::nullopt;
    }
    return static_cast<uint8_t>(value);
}

std::optional<uint32_t> SysfsManager::read_u32(const std::string& name) const {
    auto content = read_string(name);
    if (!content) {
        return std::nullopt;
    }
    const std::string text = trim(*content);
    char* end = nullptr;
    errno = 0;
    const unsigned long value = std::strtoul(text.c_str(), &end, 10);
    if (errno != 0 || end == text.c_str() || *end != '\0' ||
        value > std::numeric_limits<uint32_t>::max()) {
        return std::nullopt;
    }
    return static_cast<uint32_t>(value);
}

bool SysfsManager::exists(const std::string& name) const {
    auto path = get_path(name);
    return path && path_exists(*path);
}

void SysfsManager::initialize() {
    set_touch_mode("normal");
    write_bool("dt2w_enable", true);
    write_bool("gesture_enable", true);
    set_report_rate(60);
    write_bool("fod_enable", true);
    (void)read_string("panel_vendor");
    log_message(LogLevel::Info, "Sysfs entries initialized");
}

bool SysfsManager::set_touch_mode(const std::string& mode) {
    return write_string("touch_mode", mode);
}

bool SysfsManager::set_report_rate(uint32_t rate) {
    return write_u32("report_rate", rate);
}

bool SysfsManager::set_game_mode(bool enable) {
    return write_bool("game_mode", enable);
}

bool SysfsManager::wake_screen() {
    if (!write_bool("wake_screen", true)) {
        return false;
    }
    sleep_ms(100);
    return write_bool("wake_screen", false);
}

}  // namespace oplus::touch
