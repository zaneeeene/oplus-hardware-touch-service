#pragma once

#include "oplus_touch/config.hpp"
#include "oplus_touch/screen_state.hpp"

#include <atomic>
#include <cstdint>
#include <string>
#include <thread>

namespace oplus::touch {

class DozeKeeper {
  public:
    explicit DozeKeeper(AodKeepaliveConfig config);
    ~DozeKeeper();

    DozeKeeper(const DozeKeeper&) = delete;
    DozeKeeper& operator=(const DozeKeeper&) = delete;

    bool start();
    void stop();
    bool running() const { return running_.load(std::memory_order_acquire); }

  private:
    struct DozeRequest {
        uint32_t flag = 0;
        uint32_t display_id = 0;
        uint32_t value = 0;
    };

    void run();
    uint32_t wait_for_change(const void* property, uint32_t serial, int state,
                             bool dbv_pending, bool apply_late);
    void apply(uint32_t value);
    bool apply_once(uint32_t value);
    bool sync_dbv(bool settled, bool fallback_allowed);
    bool write_dbv(int32_t dbv);
    int32_t read_dbv() const;
    void write_aod_dbv();

    AodKeepaliveConfig config_;
    std::atomic_bool running_{false};
    std::thread thread_;
    int disp_feature_fd_ = -1;
    uint32_t level_ = 1;
    int32_t dbv_ = 808;
    bool follow_fade_ = false;
    bool dbv_fallback_written_ = false;
    bool awake_cleanup_done_ = false;
};

std::optional<int32_t> parse_dbv(std::string_view text);
bool permits_doze_level(std::optional<ScreenState> state, uint32_t level);
bool should_write_dbv(int32_t observed, int32_t target, int32_t backlight, bool settled);
bool should_write_dbv_fallback(bool settled, bool fallback_allowed, bool already_written);
bool should_reclassify_fade(bool follow_fade, bool dbv_pending, int32_t register_value);
std::pair<uint32_t, int32_t> classic_aod_target(const AodKeepaliveConfig& config);
bool should_retry_awake_cleanup(std::optional<ScreenState> state, int32_t doze_level);

}  // namespace oplus::touch
