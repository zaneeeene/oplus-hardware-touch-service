#pragma once

#include <cstdint>
#include <optional>

namespace oplus::touch {

// Values used by OPlus/QTI's debug.tracing.screen_state property.
enum class ScreenState : int {
    Unknown = -1,
    Off = 1,
    On = 2,
    Doze = 3,
    DozeSuspend = 4,
};

ScreenState screen_state_from_raw(int value);
ScreenState resolve_screen_state(std::optional<ScreenState> tracing,
                                 std::optional<uint32_t> brightness);
ScreenState read_screen_state();
std::optional<ScreenState> read_tracing_screen_state();

bool screen_is_on(ScreenState state);
bool screen_is_off(ScreenState state);
bool screen_is_doze(ScreenState state);
bool screen_is_off_or_doze(ScreenState state);
int screen_state_raw(ScreenState state);
const char* screen_state_name(ScreenState state);

std::optional<uint32_t> read_panel_backlight();

}  // namespace oplus::touch
