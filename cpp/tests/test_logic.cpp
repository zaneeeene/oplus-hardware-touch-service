#include "oplus_touch/config.hpp"
#include "oplus_touch/doze.hpp"
#include "oplus_touch/gesture.hpp"
#include "oplus_touch/panel_feature.hpp"
#include "oplus_touch/procfs.hpp"
#include "oplus_touch/screen_state.hpp"
#include "oplus_touch/xiaomi_touch.hpp"

#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <unistd.h>

namespace {

int g_failures = 0;

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            std::cerr << __FILE__ << ":" << __LINE__ << ": CHECK failed: "    \
                      << #condition << "\n";                                   \
            ++g_failures;                                                      \
        }                                                                      \
    } while (false)

void test_config() {
    const oplus::touch::TouchConfig config;
    CHECK(config.xiaomi_touch_path == "/dev/xiaomi-touch");
    CHECK(config.tp_gesture_path == "/proc/tp_gesture");
    CHECK(config.enable_aod);
    CHECK(config.enable_dt2w);
    CHECK(config.enable_gestures);
    CHECK(config.enable_raise_to_wake);
    CHECK(config.enable_uinput_wake);
    CHECK(config.enable_full_screen_aod);
    CHECK(config.enable_seamless_aod);
    CHECK(config.aod_keepalive.enabled);
    CHECK(config.aod_defaults.enabled);
    CHECK(config.aod_panel_service.enabled);
    CHECK(config.aod_defaults.marker == "oplus_touch_hal_aod_defaults_v3");
    CHECK(config.gestures.double_tap.gesture_id == 0x22);
}

void test_screen_state() {
    using oplus::touch::ScreenState;
    using oplus::touch::resolve_screen_state;
    using oplus::touch::screen_is_doze;
    using oplus::touch::screen_is_off_or_doze;
    using oplus::touch::screen_state_from_raw;

    CHECK(screen_state_from_raw(1) == ScreenState::Off);
    CHECK(screen_state_from_raw(2) == ScreenState::On);
    CHECK(screen_state_from_raw(3) == ScreenState::Doze);
    CHECK(screen_state_from_raw(4) == ScreenState::DozeSuspend);
    CHECK(screen_state_from_raw(99) == ScreenState::Unknown);
    CHECK(screen_is_doze(ScreenState::Doze));
    CHECK(screen_is_doze(ScreenState::DozeSuspend));
    CHECK(screen_is_off_or_doze(ScreenState::Off));
    CHECK(!screen_is_off_or_doze(ScreenState::On));

    CHECK(resolve_screen_state(ScreenState::On, 0) == ScreenState::Off);
    CHECK(resolve_screen_state(ScreenState::On, 100) == ScreenState::On);
    CHECK(resolve_screen_state(ScreenState::Doze, 0) == ScreenState::Doze);
    CHECK(resolve_screen_state(std::nullopt, 100) == ScreenState::On);
    CHECK(resolve_screen_state(std::nullopt, std::nullopt) ==
          ScreenState::Unknown);
}

void test_taps() {
    using oplus::touch::TapOutcomeKind;
    using oplus::touch::TapState;

    TapState awake;
    awake.note_down(1000, false);
    CHECK(awake.finish(1100, true, true).kind == TapOutcomeKind::None);
    awake.note_down(1200, false);
    CHECK(awake.second_tap_started);
    CHECK(awake.finish(1300, true, true).kind == TapOutcomeKind::None);

    TapState asleep;
    asleep.x = 540;
    asleep.y = 1200;
    asleep.note_down(1000, true);
    const auto first = asleep.finish(1100, true, true);
    CHECK(first.kind == TapOutcomeKind::Single);
    asleep.note_down(1200, true);
    CHECK(asleep.second_tap_started);
    const auto second = asleep.finish(1300, true, true);
    CHECK(second.kind == TapOutcomeKind::Double);
    CHECK(second.x == 540);
    CHECK(second.y == 1200);

    TapState slow;
    slow.note_down(1000, true);
    const auto first_single = slow.finish(1100, true, true);
    CHECK(first_single.kind == TapOutcomeKind::Single);
    slow.note_down(1500, true);
    CHECK(!slow.second_tap_started);
    CHECK(slow.sequence_id != first_single.sequence_id);
}

void test_procfs() {
    using namespace oplus::touch;
    const std::string path =
        (std::filesystem::temp_directory_path() /
         ("oplus-touch-cpp-test-" + std::to_string(getpid())))
            .string();
    {
        std::ofstream stream(path, std::ios::trunc);
        stream << "0\n";
    }

    ProcfsTouch procfs(path, "/nonexistent");
    CHECK(procfs.set_gesture_mask(uint64_t{1} << gesture_ids::DoubleTap));
    CHECK(procfs.read_gesture().value_or("") == "1");
    CHECK(procfs.set_gesture_mask(0));
    CHECK(procfs.read_gesture().value_or("") == "0");

    const auto parsed = procfs.parse_gesture_info("34 540 1200 99");
    CHECK(parsed.has_value());
    if (parsed) {
        CHECK(parsed->gesture_id == 34);
        CHECK(parsed->x == 540);
        CHECK(parsed->y == 1200);
        CHECK(parsed->timestamp == 99);
    }

    const uint8_t ids[] = {gesture_ids::DoubleTap, gesture_ids::FodRelease};
    CHECK(create_gesture_mask(ids, 2) ==
          ((uint64_t{1} << gesture_ids::DoubleTap) |
           (uint64_t{1} << gesture_ids::FodRelease)));
    std::filesystem::remove(path);
}

void test_xiaomi_abi() {
    using namespace oplus::touch;
    CHECK(XiaomiTouch::ioctl_request(XiaomiTouchCmd::SetCurValue) == 0x5400UL);
    CHECK(XiaomiTouch::ioctl_request(XiaomiTouchCmd::SetLongValue) == 0x5407UL);
    CHECK(static_cast<uint32_t>(XiaomiTouchMode::AodEnable) == 11);
    CHECK(static_cast<uint32_t>(XiaomiTouchMode::DoubletapMode) == 14);
    CHECK(static_cast<uint32_t>(XiaomiTouchMode::ModeNum) == 16);
}

void test_doze_logic() {
    using namespace oplus::touch;
    CHECK(parse_dbv("0x03 ,0x28") == 808);
    CHECK(parse_dbv("noise 0x00 ,0x10 trailing") == 16);
    CHECK(!parse_dbv("busy"));

    CHECK(permits_doze_level(ScreenState::On, 0));
    CHECK(permits_doze_level(ScreenState::Doze, 1));
    CHECK(permits_doze_level(ScreenState::DozeSuspend, 2));
    CHECK(!permits_doze_level(ScreenState::On, 1));
    CHECK(!permits_doze_level(ScreenState::Off, 0));
    CHECK(!permits_doze_level(ScreenState::Doze, 0));
    CHECK(!permits_doze_level(std::nullopt, 1));

    CHECK(!should_write_dbv(15, 808, 15, false));
    CHECK(should_write_dbv(15, 808, 15, true));
    CHECK(should_write_dbv(42, 808, 5, false));
    CHECK(!should_write_dbv(808, 808, 15, true));
    CHECK(!should_write_dbv(-1, 808, 15, true));
    CHECK(!should_write_dbv_fallback(false, true, false));
    CHECK(!should_write_dbv_fallback(true, false, false));
    CHECK(should_write_dbv_fallback(true, true, false));
    CHECK(!should_write_dbv_fallback(true, true, true));
    CHECK(should_reclassify_fade(true, true, 8));
    CHECK(should_reclassify_fade(true, true, 0));
    CHECK(!should_reclassify_fade(true, true, 40));
    CHECK(!should_reclassify_fade(true, false, 8));
    CHECK(!should_reclassify_fade(false, true, 8));
    CHECK(!should_reclassify_fade(true, true, -1));

    AodKeepaliveConfig config;
    auto [level, dbv] = classic_aod_target(config);
    CHECK(level == 1);
    CHECK(dbv == 808);
    config.classic_doze_level = 99;
    config.classic_dbv = 1;
    auto [clamped_level, clamped_dbv] = classic_aod_target(config);
    CHECK(clamped_level == 2);
    CHECK(clamped_dbv == 16);

    CHECK(should_retry_awake_cleanup(ScreenState::On, -1));
    CHECK(should_retry_awake_cleanup(ScreenState::On, 1));
    CHECK(!should_retry_awake_cleanup(ScreenState::On, 0));
    CHECK(!should_retry_awake_cleanup(ScreenState::Doze, -1));
    CHECK(!should_retry_awake_cleanup(std::nullopt, -1));
}

void test_panel_feature() {
    using namespace oplus::touch;
    CHECK(kPanelAodCapabilities == 15);
    CHECK(kPanelUdfpsLocalHbm == 16);
    CHECK(kPanelTransactionGetFeatureValue == 1);
    CHECK(kPanelTransactionSetFeatureValue == 2);
    CHECK(kPanelTransactionGetInfo == 3);
    CHECK(kPanelTransactionGetInterfaceVersion == 16'777'215);
    CHECK(kPanelTransactionGetInterfaceHash == 16'777'214);
    CHECK(panel_feature_value(kPanelFeatureAodCaps) == 15);
    CHECK(panel_feature_value(kPanelFeatureUdfpsType) == 16);
    CHECK(!panel_feature_value(999));
}

}  // namespace

int main() {
    test_config();
    test_screen_state();
    test_taps();
    test_procfs();
    test_xiaomi_abi();
    test_doze_logic();
    test_panel_feature();
    if (g_failures != 0) {
        std::cerr << g_failures << " test assertion(s) failed\n";
        return 1;
    }
    std::cout << "C++ logic tests passed\n";
    return 0;
}
