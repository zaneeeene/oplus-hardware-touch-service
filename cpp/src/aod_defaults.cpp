#include "oplus_touch/aod_defaults.hpp"

#include "oplus_touch/common.hpp"

#include <algorithm>
#include <array>
#include <thread>

namespace oplus::touch {
namespace {

struct SettingDefault {
    const char* key;
    const char* value;
};

constexpr std::array<SettingDefault, 27> kAodDefaults = {{
    {"Setting_AodOldDataHasBeenReset", "1"},
    {"Setting_AodEnable", "1"},
    {"Setting_AodSwitchEnable", "1"},
    {"Setting_AodEnableImmediate", "1"},
    {"Setting_AodUserSetTime", "0"},
    {"Setting_AodUserEnergySavingSet", "0"},
    {"Setting_AodAdditionalTime", "1"},
    {"Setting_AodAdditionalNotification", "1"},
    {"Setting_AodAdditionalBattery", "1"},
    {"Setting_AodAdditionalDate", "1"},
    {"Setting_AodAdditionalLunar", "1"},
    {"Setting_AodAdditionalUV", "1"},
    {"Setting_AodSceneInfoSwitchEnable", "1"},
    {"Setting_AodSceneMusicSwitchEnable", "1"},
    {"Setting_Sub_AodSwitchEnable", "1"},
    {"Setting_Sub_AodEnableImmediate", "1"},
    {"Setting_Sub_AodFullScreen", "1"},
    {"Setting_Sub_AodUserEnergySavingSet", "0"},
    {"oplus_keyguard_panoramic_aod_support", "1"},
    {"oplus_keyguard_panoramic_aod_all_day_support", "1"},
    {"oplus_keyguard_workshop_aod_support", "1"},
    {"aod_media_support", "1"},
    {"full_screen_aod", "0"},
    {"Setting_AodStyleInfo", R"({"type":2,"uuid":""})"},
    {"panoramic_aod_enable", "1"},
    {"panoramic_aod_rate", "2000"},
    {"sub_display_aod_on_to_doze_switch", "1"},
}};

constexpr std::array<SettingDefault, 7> kEveryBoot = {{
    {"Setting_AodOldDataHasBeenReset", "1"},
    {"oplus_keyguard_panoramic_aod_support", "1"},
    {"oplus_keyguard_panoramic_aod_all_day_support", "1"},
    {"oplus_keyguard_workshop_aod_support", "1"},
    {"aod_media_support", "1"},
    {"panoramic_aod_enable", "1"},
    {"sub_display_aod_on_to_doze_switch", "1"},
}};

}  // namespace

AodDefaultsWorker::AodDefaultsWorker(AodDefaultsConfig config)
    : config_(std::move(config)) {}

AodDefaultsWorker::~AodDefaultsWorker() {
    stop();
}

bool AodDefaultsWorker::start() {
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
        return true;
    }
    thread_ = std::thread(&AodDefaultsWorker::run, this);
    return true;
}

void AodDefaultsWorker::stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }
    if (thread_.joinable()) {
        thread_.join();
    }
}

void AodDefaultsWorker::run() {
    if (!wait_for_property("sys.boot_completed", "1", running_, 300000, 1000)) {
        if (running_.load(std::memory_order_acquire)) {
            log_message(LogLevel::Warn,
                        "Timed out waiting for sys.boot_completed; AOD defaults were not applied");
        }
        return;
    }

    for (uint64_t second = 0; second < std::max<uint64_t>(1, config_.delay_seconds);
         ++second) {
        if (!running_.load(std::memory_order_acquire)) {
            return;
        }
        sleep_ms(1000);
    }

    apply_every_boot();
    auto marker = read_secure_setting(config_.marker);
    if (!marker || trim(*marker) != "1") {
        log_message(LogLevel::Info,
                    "Applying one-time OPlus full-screen/all-day AOD defaults");
        bool success = true;
        for (const auto& setting : kAodDefaults) {
            success = put_secure_setting(setting.key, setting.value) && success;
        }
        if (success && put_secure_setting(config_.marker, "1")) {
            log_message(LogLevel::Info, "OPlus AOD defaults applied");
        } else {
            log_message(LogLevel::Warn,
                        "Some OPlus AOD defaults failed; they will be retried on next boot");
        }
    } else {
        log_message(LogLevel::Debug, "OPlus AOD defaults marker is already set");
    }

    for (int second = 0; second < 10; ++second) {
        if (!running_.load(std::memory_order_acquire)) {
            return;
        }
        sleep_ms(1000);
    }
    apply_every_boot();
}

void AodDefaultsWorker::apply_every_boot() {
    for (const auto& setting : kEveryBoot) {
        auto current = read_secure_setting(setting.key);
        if (current && trim(*current) == setting.value) {
            continue;
        }
        if (!put_secure_setting(setting.key, setting.value)) {
            log_message(LogLevel::Debug,
                        std::string("Could not set every-boot AOD setting ") + setting.key);
        }
    }
}

std::optional<std::string> AodDefaultsWorker::read_secure_setting(
    const std::string& key) const {
    auto output = run_command_capture(
        {"/system/bin/cmd", "settings", "get", "secure", key});
    if (!output) {
        return std::nullopt;
    }
    return trim(*output);
}

bool AodDefaultsWorker::put_secure_setting(const std::string& key,
                                           const std::string& value) const {
    return run_command_status(
        {"/system/bin/cmd", "settings", "put", "secure", key, value});
}

}  // namespace oplus::touch
