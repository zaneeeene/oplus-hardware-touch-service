#pragma once

#include "oplus_touch/config.hpp"
#include "oplus_touch/sysfs.hpp"
#include "oplus_touch/xiaomi_touch.hpp"

#include <atomic>
#include <memory>
#include <mutex>
#include <optional>
#include <thread>

namespace oplus::touch {

enum class AodState {
    Off,
    On,
    Dimmed,
    Seamless,
    FullScreen,
};

class DozeKeeper;
class AodDefaultsWorker;

class AodManager {
  public:
    AodManager(const TouchConfig& config, std::shared_ptr<XiaomiTouch> xiaomi_touch,
               std::shared_ptr<SysfsManager> sysfs);
    ~AodManager();

    AodManager(const AodManager&) = delete;
    AodManager& operator=(const AodManager&) = delete;

    bool start();
    void stop();
    bool running() const { return running_.load(std::memory_order_acquire); }

    bool enable_aod();
    bool disable_aod();
    bool set_brightness(uint8_t brightness);
    bool enable_full_screen();
    bool disable_full_screen();
    bool enable_seamless();
    bool disable_seamless();
    bool dim();
    bool undim();
    bool on_proximity(bool near);
    bool on_dt2w();
    bool on_music_double_tap(int x, int y);
    bool collapse_music_view();

    AodState state() const;
    bool is_enabled() const;

  private:
    struct AodConfig {
        bool enabled = true;
        uint8_t brightness = 50;
        bool full_screen = true;
        bool seamless = true;
        bool music_control = true;
        AodKeepaliveConfig keepalive;
        AodDefaultsConfig defaults;
    };

    void monitor_loop();

    AodConfig config_;
    std::shared_ptr<XiaomiTouch> xiaomi_touch_;
    std::shared_ptr<SysfsManager> sysfs_;

    mutable std::mutex state_mutex_;
    AodState state_ = AodState::Off;
    bool desired_on_ = true;
    std::atomic_bool running_{false};
    std::thread monitor_thread_;
    std::unique_ptr<DozeKeeper> doze_keeper_;
    std::unique_ptr<AodDefaultsWorker> defaults_worker_;
};

}  // namespace oplus::touch
