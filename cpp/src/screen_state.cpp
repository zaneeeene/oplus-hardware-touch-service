#include "oplus_touch/screen_state.hpp"

#include "oplus_touch/common.hpp"

#include <array>
#include <cerrno>
#include <cstdlib>

namespace oplus::touch {

ScreenState screen_state_from_raw(int value) {
    switch (value) {
        case 1:
            return ScreenState::Off;
        case 2:
            return ScreenState::On;
        case 3:
            return ScreenState::Doze;
        case 4:
            return ScreenState::DozeSuspend;
        default:
            return ScreenState::Unknown;
    }
}

ScreenState resolve_screen_state(std::optional<ScreenState> tracing,
                                 std::optional<uint32_t> brightness) {
    if (tracing) {
        // The tracing property stays at ON briefly after the panel blanks.
        // Trust a live zero backlight and reclassify the display as off.
        if (*tracing == ScreenState::On && brightness && *brightness == 0) {
            return ScreenState::Off;
        }
        return *tracing;
    }
    if (brightness) {
        return *brightness > 0 ? ScreenState::On : ScreenState::Off;
    }
    return ScreenState::Unknown;
}

std::optional<uint32_t> read_panel_backlight() {
    static constexpr std::array<const char*, 3> paths = {
        "/sys/class/backlight/panel0-backlight/brightness",
        "/sys/class/backlight/panel1-backlight/brightness",
        "/sys/class/backlight/backlight/brightness",
    };
    for (const char* path : paths) {
        auto content = read_file(path);
        if (!content) {
            continue;
        }
        const std::string value = trim(*content);
        if (value.empty()) {
            continue;
        }
        char* end = nullptr;
        errno = 0;
        unsigned long parsed = std::strtoul(value.c_str(), &end, 10);
        if (errno == 0 && end != value.c_str() && parsed <= UINT32_MAX) {
            return static_cast<uint32_t>(parsed);
        }
    }
    return std::nullopt;
}

std::optional<ScreenState> read_tracing_screen_state() {
    auto value = get_property("debug.tracing.screen_state");
    if (!value) {
        return std::nullopt;
    }
    const std::string text = trim(*value);
    if (text.empty()) {
        return std::nullopt;
    }
    char* end = nullptr;
    errno = 0;
    long parsed = std::strtol(text.c_str(), &end, 10);
    if (errno != 0 || end == text.c_str()) {
        return std::nullopt;
    }
    ScreenState state = screen_state_from_raw(static_cast<int>(parsed));
    if (state == ScreenState::Unknown) {
        return std::nullopt;
    }
    return state;
}

ScreenState read_screen_state() {
    return resolve_screen_state(read_tracing_screen_state(), read_panel_backlight());
}

bool screen_is_on(ScreenState state) {
    return state == ScreenState::On;
}

bool screen_is_off(ScreenState state) {
    return state == ScreenState::Off;
}

bool screen_is_doze(ScreenState state) {
    return state == ScreenState::Doze || state == ScreenState::DozeSuspend;
}

bool screen_is_off_or_doze(ScreenState state) {
    return state == ScreenState::Off || screen_is_doze(state);
}

int screen_state_raw(ScreenState state) {
    return static_cast<int>(state);
}

const char* screen_state_name(ScreenState state) {
    switch (state) {
        case ScreenState::Off:
            return "Off";
        case ScreenState::On:
            return "On";
        case ScreenState::Doze:
            return "Doze";
        case ScreenState::DozeSuspend:
            return "DozeSuspend";
        case ScreenState::Unknown:
        default:
            return "Unknown";
    }
}

}  // namespace oplus::touch
