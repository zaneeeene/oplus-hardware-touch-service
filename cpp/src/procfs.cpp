#include "oplus_touch/procfs.hpp"

#include "oplus_touch/common.hpp"

#include <cerrno>
#include <cstdlib>
#include <iterator>
#include <sstream>
#include <vector>

namespace oplus::touch {

ProcfsTouch::ProcfsTouch(std::string tp_gesture_path, std::string xiaomi_touch_path)
    : tp_gesture_path_(std::move(tp_gesture_path)),
      xiaomi_touch_path_(std::move(xiaomi_touch_path)) {}

bool ProcfsTouch::write_gesture(const std::string& gesture) const {
    const bool success = write_file(tp_gesture_path_, gesture + "\n");
    if (success) {
        log_message(LogLevel::Debug, "Wrote gesture to tp_gesture: " + gesture);
    }
    return success;
}

std::optional<std::string> ProcfsTouch::read_gesture() const {
    auto content = read_file(tp_gesture_path_);
    if (!content) {
        return std::nullopt;
    }
    return trim(*content);
}

bool ProcfsTouch::enable_dt2w() const {
    return write_gesture("1");
}

bool ProcfsTouch::disable_dt2w() const {
    return write_gesture("0");
}

bool ProcfsTouch::set_gesture_mask(uint64_t mask) const {
    // topaz's Novatek interface accepts one boolean, not the bitmap.
    return write_gesture(mask == 0 ? "0" : "1");
}

std::optional<uint64_t> ProcfsTouch::get_gesture_mask() const {
    auto content = read_gesture();
    if (!content || content->empty()) {
        return std::nullopt;
    }
    char* end = nullptr;
    errno = 0;
    unsigned long long mask = std::strtoull(content->c_str(), &end, 10);
    if (errno != 0 || end == content->c_str()) {
        return std::nullopt;
    }
    return static_cast<uint64_t>(mask);
}

std::optional<std::string> ProcfsTouch::read_xiaomi_touch() const {
    return read_file(xiaomi_touch_path_);
}

bool ProcfsTouch::write_xiaomi_touch(const std::string& data) const {
    return write_file(xiaomi_touch_path_, data);
}

bool ProcfsTouch::tp_gesture_exists() const {
    return path_exists(tp_gesture_path_);
}

bool ProcfsTouch::xiaomi_touch_exists() const {
    return path_exists(xiaomi_touch_path_);
}

std::optional<GestureInfo> ProcfsTouch::parse_gesture_info(const std::string& content) const {
    std::istringstream stream(content);
    std::vector<std::string> parts;
    std::string part;
    while (stream >> part) {
        parts.push_back(part);
    }

    if (parts.size() >= 3) {
        GestureInfo info;
        info.gesture_id = static_cast<uint8_t>(std::strtoul(parts[0].c_str(), nullptr, 10));
        info.x = static_cast<int>(std::strtol(parts[1].c_str(), nullptr, 10));
        info.y = static_cast<int>(std::strtol(parts[2].c_str(), nullptr, 10));
        if (parts.size() >= 4) {
            info.timestamp =
                static_cast<uint64_t>(std::strtoull(parts[3].c_str(), nullptr, 10));
        }
        for (size_t index = 4; index < parts.size(); ++index) {
            if (!info.extra.empty()) {
                info.extra.push_back(' ');
            }
            info.extra += parts[index];
        }
        return info;
    }

    auto first = content.find_first_not_of(" \t\r\n");
    if (first != std::string::npos && content[first] == '{') {
        GestureInfo info;
        info.extra = content;
        return info;
    }
    return std::nullopt;
}

uint64_t create_gesture_mask(const uint8_t* gestures, size_t count) {
    uint64_t mask = 0;
    if (gestures == nullptr) {
        return mask;
    }
    for (size_t index = 0; index < count; ++index) {
        if (gestures[index] < 64) {
            mask |= uint64_t{1} << gestures[index];
        }
    }
    return mask;
}

uint64_t default_gesture_mask() {
    static constexpr uint8_t gestures[] = {
        gesture_ids::DoubleTap, gesture_ids::SwipeUp, gesture_ids::SwipeDown,
        gesture_ids::SwipeLeft, gesture_ids::SwipeRight,
    };
    return create_gesture_mask(gestures, std::size(gestures));
}

}  // namespace oplus::touch
