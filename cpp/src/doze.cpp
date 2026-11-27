#include "oplus_touch/doze.hpp"

#include "oplus_touch/common.hpp"

#include <algorithm>
#include <array>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <thread>
#include <unistd.h>
#include <vector>

#ifdef __ANDROID__
#include <sys/system_properties.h>
#endif

namespace oplus::touch {
namespace {

constexpr unsigned long kMiDispIoctlSetDozeBrightness = 0x400c4402UL;
constexpr uint32_t kDozeOff = 0;
constexpr uint32_t kDozeHigh = 1;
constexpr uint32_t kDozeLow = 2;

constexpr float kAodGain = 1945.0F / 808.0F;
constexpr int32_t kAodDbvMin = 16;
constexpr int32_t kAodDbvMax = 808;
constexpr int32_t kMinFadeRegister = 40;
constexpr int32_t kLowAodMaxRegister = 200;

constexpr int32_t kApplyLateMs = 100;
constexpr int32_t kSettleMs = 400;
constexpr int32_t kClassicDbvCheckMs = 300;
constexpr int32_t kDbvFallbackWriteMs = 440;
constexpr std::array<int32_t, 2> kAwakeRecheckMs = {300, 800};
constexpr std::array<int32_t, 13> kRecheckMs = {
    100, 120, 160, 200, 240, 280, 300, 340, 440, 600, 800, 1500, 3000,
};

int32_t read_int(const std::string& path) {
    auto value = read_int_file(path);
    return value.value_or(-1);
}

bool write_mipi(const std::string& path, const std::string& command) {
    return write_file(path, command);
}

#ifdef __ANDROID__
const prop_info* property_find(const char* name) {
    return __system_property_find(name);
}

uint32_t property_serial() {
    uint32_t serial = 0;
    __system_property_wait(nullptr, 0, &serial, nullptr);
    return serial;
}

bool property_wait(const prop_info* info, uint32_t serial, uint32_t* next,
                   uint64_t timeout_ms) {
    if (info == nullptr) {
        sleep_ms(timeout_ms);
        return false;
    }
    timespec timeout{};
    timeout.tv_sec = static_cast<time_t>(timeout_ms / 1000);
    timeout.tv_nsec = static_cast<long>((timeout_ms % 1000) * 1000000);
    return __system_property_wait(info, serial, next, &timeout);
}
#else
struct prop_info;
const prop_info* property_find(const char*) {
    return nullptr;
}

uint32_t property_serial() {
    return 0;
}

bool property_wait(const prop_info*, uint32_t, uint32_t*, uint64_t timeout_ms) {
    sleep_ms(timeout_ms);
    return false;
}
#endif

}  // namespace

std::optional<int32_t> parse_dbv(std::string_view text) {
    const size_t start = text.find("0x");
    if (start == std::string_view::npos) {
        return std::nullopt;
    }
    std::vector<uint32_t> values;
    size_t position = start;
    while (position < text.size()) {
        if (text[position] != '0' || position + 2 >= text.size() ||
            (text[position + 1] != 'x' && text[position + 1] != 'X')) {
            ++position;
            continue;
        }
        size_t end = position + 2;
        while (end < text.size() &&
               ((text[end] >= '0' && text[end] <= '9') ||
                (text[end] >= 'a' && text[end] <= 'f') ||
                (text[end] >= 'A' && text[end] <= 'F'))) {
            ++end;
        }
        if (end > position + 2) {
            const std::string token(text.substr(position + 2, end - position - 2));
            char* parsed_end = nullptr;
            unsigned long value = std::strtoul(token.c_str(), &parsed_end, 16);
            if (parsed_end != token.c_str() && value <= UINT32_MAX) {
                values.push_back(static_cast<uint32_t>(value));
            }
        }
        position = end;
    }
    if (values.size() < 2) {
        return std::nullopt;
    }
    return static_cast<int32_t>((values[0] << 8) | values[1]);
}

bool permits_doze_level(std::optional<ScreenState> state, uint32_t level) {
    if (!state) {
        return false;
    }
    if (level == kDozeOff) {
        return *state == ScreenState::On;
    }
    if (level == kDozeHigh || level == kDozeLow) {
        return screen_is_doze(*state);
    }
    return false;
}

bool should_write_dbv(int32_t observed, int32_t target, int32_t backlight, bool settled) {
    return observed >= 0 && observed != target && (settled || observed != backlight);
}

bool should_write_dbv_fallback(bool settled, bool fallback_allowed, bool already_written) {
    return settled && fallback_allowed && !already_written;
}

bool should_reclassify_fade(bool follow_fade, bool dbv_pending, int32_t register_value) {
    return follow_fade && dbv_pending && register_value >= 0 &&
           register_value < kMinFadeRegister;
}

std::pair<uint32_t, int32_t> classic_aod_target(const AodKeepaliveConfig& config) {
    const uint32_t level =
        std::clamp<uint32_t>(config.classic_doze_level, kDozeHigh, kDozeLow);
    const int32_t dbv =
        static_cast<int32_t>(std::clamp<uint32_t>(config.classic_dbv, kAodDbvMin, kAodDbvMax));
    return {level, dbv};
}

bool should_retry_awake_cleanup(std::optional<ScreenState> state, int32_t doze_level) {
    return state && *state == ScreenState::On && doze_level != 0;
}

DozeKeeper::DozeKeeper(AodKeepaliveConfig config) : config_(std::move(config)) {}

DozeKeeper::~DozeKeeper() {
    stop();
    if (disp_feature_fd_ >= 0) {
        close(disp_feature_fd_);
        disp_feature_fd_ = -1;
    }
}

bool DozeKeeper::start() {
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
        return true;
    }
    if (!path_exists(config_.disp_feature_path)) {
        log_message(LogLevel::Warn, "AOD doze keeper disabled: " + config_.disp_feature_path +
                                        " does not exist");
        running_.store(false, std::memory_order_release);
        return false;
    }
    thread_ = std::thread(&DozeKeeper::run, this);
    return true;
}

void DozeKeeper::stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }
    if (thread_.joinable()) {
        thread_.join();
    }
}

void DozeKeeper::run() {
    log_message(LogLevel::Info,
                "AOD doze keeper started (classic level " +
                    std::to_string(config_.classic_doze_level) + ", DBV " +
                    std::to_string(config_.classic_dbv) + ")");

    const prop_info* property = property_find("debug.tracing.screen_state");
    if (property == nullptr) {
        log_message(LogLevel::Warn,
                    "AOD doze keeper: debug.tracing.screen_state is unavailable");
        return;
    }

    uint32_t serial = property_serial();
    int previous = -1;
    while (running_.load(std::memory_order_acquire)) {
        const auto tracing = read_tracing_screen_state();
        const int state = tracing ? screen_state_raw(*tracing) : -1;
        const bool in_aod = state == 3 || state == 4;
        bool dbv_pending = false;
        bool apply_late = false;

        if (in_aod) {
            const bool entered = !(previous == 3 || previous == 4);
            if (entered) {
                dbv_fallback_written_ = false;
            }
            const int32_t reg = read_int(config_.backlight_path);

            if (previous == 2) {
                follow_fade_ = config_.follow_fade && reg >= kMinFadeRegister;
            } else if (previous == 1) {
                follow_fade_ = false;
            }

            int32_t dbv = dbv_;
            if (follow_fade_) {
                if (reg >= kMinFadeRegister) {
                    level_ = reg < kLowAodMaxRegister ? kDozeLow : kDozeHigh;
                    dbv = static_cast<int32_t>(static_cast<float>(reg) / kAodGain + 0.5F);
                    dbv = std::clamp(dbv, kAodDbvMin, kAodDbvMax);
                }
            } else {
                auto [level, classic_dbv] = classic_aod_target(config_);
                level_ = level;
                dbv = classic_dbv;
            }

            const bool dbv_changed = dbv != dbv_;
            dbv_ = dbv;
            if (entered || read_int(config_.doze_brightness_path) !=
                              static_cast<int32_t>(level_)) {
                if (entered && previous == 2) {
                    apply_late = true;
                } else {
                    apply(level_);
                }
                dbv_pending = true;
            } else if (dbv_changed) {
                write_aod_dbv();
            }
        } else if (state == 1) {
            // The panel can remain lit briefly after screen_state flips to
            // OFF. Do not reprogram it during that transition.
            follow_fade_ = false;
            dbv_fallback_written_ = false;
            awake_cleanup_done_ = false;
        } else if (state == 2 && previous != 2) {
            follow_fade_ = false;
            dbv_fallback_written_ = false;
            awake_cleanup_done_ = false;
            apply(kDozeOff);
        } else if (state == -1) {
            follow_fade_ = false;
        }

        previous = state;
        serial = wait_for_change(property, serial, state, dbv_pending, apply_late);
    }

    if (read_tracing_screen_state() &&
        read_tracing_screen_state().value() == ScreenState::On) {
        apply(kDozeOff);
    }
    log_message(LogLevel::Info, "AOD doze keeper stopped");
}

uint32_t DozeKeeper::wait_for_change(const void* property, uint32_t serial, int state,
                                     bool dbv_pending, bool apply_late) {
    uint32_t next = serial;
    int32_t elapsed = 0;
    const bool in_aod = state == 3 || state == 4;
    const bool awake = state == 2;

    for (const int32_t at : kRecheckMs) {
        if (!running_.load(std::memory_order_acquire)) {
            return next;
        }

        const bool late_at = at == kApplyLateMs;
        const bool classic = in_aod && !follow_fade_;
        const bool level_at =
            !late_at && classic &&
            (at == 300 || at == 800 || at == 1500 || at == 3000) &&
            read_int(config_.doze_brightness_path) >= 0;
        const bool dbv_at =
            !late_at && in_aod && (dbv_pending || (classic && at == kClassicDbvCheckMs));
        const bool awake_at =
            !late_at && awake &&
            std::find(kAwakeRecheckMs.begin(), kAwakeRecheckMs.end(), at) !=
                kAwakeRecheckMs.end();

        if ((late_at && !apply_late) || (!late_at && !level_at && !dbv_at && !awake_at)) {
            continue;
        }

        const uint64_t gap = static_cast<uint64_t>(std::max(1, at - elapsed));
        elapsed = at;
        if (property_wait(static_cast<const prop_info*>(property), next, &next, gap)) {
            return next;
        }

        if (late_at) {
            apply(level_);
            continue;
        }

        if (awake_at && !awake_cleanup_done_) {
            const int32_t doze_level = read_int(config_.doze_brightness_path);
            if (should_retry_awake_cleanup(read_tracing_screen_state(), doze_level)) {
                log_message(LogLevel::Debug,
                            "AOD level still present after wake (" +
                                std::to_string(doze_level) + "); reapplying DOZE_OFF");
                apply(kDozeOff);
                awake_cleanup_done_ = true;
            }
        }

        if (dbv_at && follow_fade_) {
            const int32_t reg = read_int(config_.backlight_path);
            if (should_reclassify_fade(follow_fade_, dbv_pending, reg)) {
                log_message(LogLevel::Debug,
                            "Full-screen AOD fade collapsed at " + std::to_string(at) +
                                " ms (backlight " + std::to_string(reg) +
                                "); using classic AOD");
                follow_fade_ = false;
                auto [level, classic_dbv] = classic_aod_target(config_);
                dbv_ = classic_dbv;
                if (level_ != level) {
                    level_ = level;
                    apply(level);
                }
                dbv_pending = true;
            }
        }

        if (level_at && read_int(config_.doze_brightness_path) !=
                           static_cast<int32_t>(level_)) {
            apply(level_);
            dbv_pending = true;
        }

        if (dbv_at) {
            const bool fallback_allowed = at == kDbvFallbackWriteMs;
            if (sync_dbv(at >= kSettleMs, fallback_allowed) && at >= kSettleMs) {
                dbv_pending = false;
            }
        }
    }

    if (property_wait(static_cast<const prop_info*>(property), next, &next, 1000)) {
        return next;
    }
    return next;
}

bool DozeKeeper::sync_dbv(bool settled, bool fallback_allowed) {
    const auto state = read_tracing_screen_state();
    if (!state || !screen_is_doze(*state)) {
        return true;
    }

    const int32_t dbv = read_dbv();
    if (dbv == dbv_) {
        return true;
    }
    if (dbv < 0) {
        if (should_write_dbv_fallback(settled, fallback_allowed, dbv_fallback_written_) &&
            write_dbv(dbv_)) {
            log_message(LogLevel::Debug,
                        "AOD DCS readback unavailable; wrote one settled DBV fallback");
            dbv_fallback_written_ = true;
            return true;
        }
        return false;
    }

    const int32_t backlight = read_int(config_.backlight_path);
    if (should_write_dbv(dbv, dbv_, backlight, settled) && write_dbv(dbv_)) {
        return true;
    }
    return false;
}

void DozeKeeper::write_aod_dbv() {
    const auto state = read_tracing_screen_state();
    if (state && screen_is_doze(*state)) {
        (void)write_dbv(dbv_);
    }
}

bool DozeKeeper::write_dbv(int32_t dbv) {
    char command[64];
    std::snprintf(command, sizeof(command),
                  "00 00 00 39 00 00 00 00 00 03 51 %02X %02X",
                  (dbv >> 8) & 0xff, dbv & 0xff);
    if (write_mipi(config_.mipi_rw_path, command)) {
        return true;
    }
    log_message(LogLevel::Debug, "mi_display DBV write failed through " +
                                    config_.mipi_rw_path);
    return false;
}

int32_t DozeKeeper::read_dbv() const {
    if (!write_mipi(config_.mipi_rw_path, "01 02 00 06 01 00 00 00 00 01 52")) {
        return -1;
    }
    auto content = read_file(config_.mipi_rw_path);
    if (!content) {
        return -1;
    }
    auto parsed = parse_dbv(*content);
    return parsed.value_or(-1);
}

void DozeKeeper::apply(uint32_t value) {
    for (int attempt = 0; attempt < 25; ++attempt) {
        if (!running_.load(std::memory_order_acquire)) {
            return;
        }
        if (!permits_doze_level(read_tracing_screen_state(), value)) {
            return;
        }
        if (apply_once(value)) {
            return;
        }
        if (attempt < 24) {
            sleep_ms(150);
        }
    }
    log_message(LogLevel::Debug,
                "mi_display rejected doze level " + std::to_string(value) +
                    " after 25 attempts");
}

bool DozeKeeper::apply_once(uint32_t value) {
    if (disp_feature_fd_ < 0) {
        disp_feature_fd_ = open(config_.disp_feature_path.c_str(), O_RDWR | O_CLOEXEC);
        if (disp_feature_fd_ < 0) {
            log_message(LogLevel::Warn,
                        "Cannot open " + config_.disp_feature_path + ": " +
                            std::strerror(errno));
            return false;
        }
    }
    DozeRequest request{};
    request.flag = 0;
    request.display_id = 0;
    request.value = value;
    return ioctl(disp_feature_fd_, kMiDispIoctlSetDozeBrightness, &request) == 0;
}

}  // namespace oplus::touch
