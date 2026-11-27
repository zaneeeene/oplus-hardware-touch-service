#pragma once

#include <cstdint>
#include <optional>
#include <string>

namespace oplus::touch {

namespace gesture_ids {
constexpr uint8_t DoubleTap = 0x22;
constexpr uint8_t SingleTap = 0x23;
constexpr uint8_t SwipeUp = 0x24;
constexpr uint8_t SwipeDown = 0x25;
constexpr uint8_t SwipeLeft = 0x26;
constexpr uint8_t SwipeRight = 0x27;
constexpr uint8_t LongPress = 0x28;
constexpr uint8_t DoubleSwipe = 0x29;
constexpr uint8_t HeartRate = 0x30;
constexpr uint8_t FodPress = 0x31;
constexpr uint8_t FodRelease = 0x32;
}  // namespace gesture_ids

struct GestureInfo {
    uint8_t gesture_id = 0;
    int x = 0;
    int y = 0;
    uint64_t timestamp = 0;
    std::string extra;
};

class ProcfsTouch {
  public:
    ProcfsTouch(std::string tp_gesture_path, std::string xiaomi_touch_path);

    bool write_gesture(const std::string& gesture) const;
    std::optional<std::string> read_gesture() const;
    bool enable_dt2w() const;
    bool disable_dt2w() const;
    bool set_gesture_mask(uint64_t mask) const;
    std::optional<uint64_t> get_gesture_mask() const;
    std::optional<std::string> read_xiaomi_touch() const;
    bool write_xiaomi_touch(const std::string& data) const;
    bool tp_gesture_exists() const;
    bool xiaomi_touch_exists() const;
    std::optional<GestureInfo> parse_gesture_info(const std::string& content) const;

    const std::string& tp_gesture_path() const { return tp_gesture_path_; }

  private:
    std::string tp_gesture_path_;
    std::string xiaomi_touch_path_;
};

uint64_t create_gesture_mask(const uint8_t* gestures, size_t count);
uint64_t default_gesture_mask();

}  // namespace oplus::touch
