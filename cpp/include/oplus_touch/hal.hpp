#pragma once

#include "oplus_touch/aod.hpp"
#include "oplus_touch/config.hpp"
#include "oplus_touch/gesture.hpp"
#include "oplus_touch/procfs.hpp"
#include "oplus_touch/raise_to_wake.hpp"
#include "oplus_touch/sysfs.hpp"
#include "oplus_touch/xiaomi_touch.hpp"

#include <atomic>
#include <memory>
#include <mutex>

namespace oplus::touch {

class TouchHal {
  public:
    explicit TouchHal(TouchConfig config);
    ~TouchHal();

    TouchHal(const TouchHal&) = delete;
    TouchHal& operator=(const TouchHal&) = delete;

    bool start_all();
    void stop_all();

    bool is_running() const { return running_.load(std::memory_order_acquire); }
    AodState aod_state() const;

    std::shared_ptr<XiaomiTouch> xiaomi_touch() const { return xiaomi_touch_; }
    std::shared_ptr<SysfsManager> sysfs() const { return sysfs_; }
    std::shared_ptr<ProcfsTouch> procfs() const { return procfs_; }

    bool set_dt2w(bool enable);
    bool set_aod(bool enable);
    bool set_aod_brightness(uint8_t brightness);
    bool set_full_screen_aod(bool enable);
    bool set_seamless_aod(bool enable);
    bool set_game_mode(bool enable);
    bool set_report_rate(uint32_t rate);
    bool handle_kernel_gesture(uint8_t gesture_id);
    bool handle_proximity(bool near);

  private:
    bool start_gestures();
    bool start_aod();
    bool start_raise_to_wake();
    void handle_gesture_event(const GestureEvent& event);

    TouchConfig config_;
    std::shared_ptr<XiaomiTouch> xiaomi_touch_;
    std::shared_ptr<SysfsManager> sysfs_;
    std::shared_ptr<ProcfsTouch> procfs_;

    mutable std::mutex components_mutex_;
    std::shared_ptr<GestureHandler> gesture_handler_;
    std::shared_ptr<AodManager> aod_manager_;
    std::shared_ptr<RaiseToWakeManager> raise_to_wake_;
    AodGestureHandler aod_gesture_handler_;
    std::shared_ptr<std::atomic_bool> dt2w_enabled_;
    std::atomic_bool running_{false};
};

}  // namespace oplus::touch
