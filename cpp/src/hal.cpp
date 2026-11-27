#include "oplus_touch/hal.hpp"

#include "oplus_touch/common.hpp"

#include <utility>

namespace oplus::touch {

TouchHal::TouchHal(TouchConfig config)
    : config_(std::move(config)),
      xiaomi_touch_(std::make_shared<XiaomiTouch>(
          XiaomiTouch::open_optional(config_.xiaomi_touch_path))),
      sysfs_(std::make_shared<SysfsManager>(config_.sysfs)),
      procfs_(std::make_shared<ProcfsTouch>(config_.tp_gesture_path,
                                            "/proc/xiaomi-touch")),
      dt2w_enabled_(
          std::make_shared<std::atomic_bool>(config_.enable_dt2w)) {
    log_message(LogLevel::Info,
                "Initializing Touch HAL for topaz (Redmi Note 12 4G)");
    sysfs_->initialize();

    if (!xiaomi_touch_->is_available()) {
        log_message(LogLevel::Warn, "Continuing without /dev/xiaomi-touch");
    }
    if (!procfs_->tp_gesture_exists()) {
        log_message(LogLevel::Warn,
                    "tp_gesture not found at " + config_.tp_gesture_path);
    }
    if (!procfs_->xiaomi_touch_exists()) {
        log_message(LogLevel::Debug, "/proc/xiaomi-touch not present");
    }
}

TouchHal::~TouchHal() {
    stop_all();
}

bool TouchHal::start_all() {
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true,
                                          std::memory_order_acq_rel)) {
        return true;
    }

    log_message(LogLevel::Info, "Starting all touch services");

    // Gestures are installed first so the AOD manager can consume the same
    // double-tap stream for music/activity expansion.
    if (config_.enable_gestures || config_.enable_dt2w) {
        if (!start_gestures()) {
            stop_all();
            return false;
        }
    }

    if (config_.enable_aod || config_.enable_raise_to_wake) {
        if (!start_aod()) {
            stop_all();
            return false;
        }
    }

    if (config_.enable_raise_to_wake) {
        if (!start_raise_to_wake()) {
            stop_all();
            return false;
        }
    }

    if (!xiaomi_touch_->enable_aod(config_.enable_aod)) {
        log_message(LogLevel::Warn,
                    "xiaomi-touch AOD setup skipped or rejected");
    }
    if (!xiaomi_touch_->enable_dt2w(config_.enable_dt2w)) {
        log_message(LogLevel::Warn,
                    "xiaomi-touch DT2W setup skipped or rejected");
    }
    if (!xiaomi_touch_->enable_fod(true)) {
        log_message(LogLevel::Debug,
                    "xiaomi-touch FOD setup skipped or rejected");
    }
    if (!xiaomi_touch_->set_report_rate(60)) {
        log_message(LogLevel::Debug,
                    "xiaomi-touch report-rate setup skipped or rejected");
    }

    log_message(LogLevel::Info, "All touch services started");
    return true;
}

void TouchHal::stop_all() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }

    log_message(LogLevel::Info, "Stopping all touch services");

    std::shared_ptr<RaiseToWakeManager> raise_to_wake;
    std::shared_ptr<AodManager> aod;
    std::shared_ptr<GestureHandler> gesture;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        raise_to_wake = std::move(raise_to_wake_);
        aod = std::move(aod_manager_);
        gesture = std::move(gesture_handler_);
    }

    if (raise_to_wake) {
        raise_to_wake->stop();
    }
    if (aod) {
        aod->stop();
    }
    if (gesture) {
        gesture->set_event_callback({});
        gesture->stop();
    }
    (void)xiaomi_touch_->enable_dt2w(false);
    if (!procfs_->disable_dt2w()) {
        log_message(LogLevel::Debug, "Could not disarm tp_gesture");
    }

    log_message(LogLevel::Info, "All touch services stopped");
}

bool TouchHal::start_gestures() {
    GestureConfig gestures = config_.gestures;
    if (!config_.enable_dt2w) {
        gestures.double_tap.enabled = false;
    }

    auto handler = std::make_shared<GestureHandler>(
        std::move(gestures), xiaomi_touch_, config_.touchscreen_path,
        config_.tp_gesture_path, config_.enable_uinput_wake, dt2w_enabled_);
    handler->set_event_callback(
        [this](const GestureEvent& event) { handle_gesture_event(event); });
    if (!handler->start()) {
        log_message(LogLevel::Error, "Could not start gesture handler");
        return false;
    }

    std::lock_guard<std::mutex> lock(components_mutex_);
    gesture_handler_ = std::move(handler);
    return true;
}

bool TouchHal::start_aod() {
    auto manager = std::make_shared<AodManager>(
        config_, xiaomi_touch_, sysfs_);
    if (!manager->start()) {
        log_message(LogLevel::Error, "Could not start AOD manager");
        return false;
    }
    std::lock_guard<std::mutex> lock(components_mutex_);
    aod_manager_ = std::move(manager);
    return true;
}

bool TouchHal::start_raise_to_wake() {
    auto manager =
        std::make_shared<RaiseToWakeManager>(config_.raise_to_wake, sysfs_);
    if (!manager->start()) {
        log_message(LogLevel::Error, "Could not start raise-to-wake");
        return false;
    }
    std::lock_guard<std::mutex> lock(components_mutex_);
    raise_to_wake_ = std::move(manager);
    return true;
}

void TouchHal::handle_gesture_event(const GestureEvent& event) {
    if (event.identity.type != GestureType::DoubleTap) {
        return;
    }

    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    if (!aod || !aod->is_enabled()) {
        return;
    }

    (void)aod->on_music_double_tap(event.x, event.y);
    (void)aod_gesture_handler_.handle_aod_double_tap(event.x, event.y);
}

AodState TouchHal::aod_state() const {
    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    return aod ? aod->state() : AodState::Off;
}

bool TouchHal::set_dt2w(bool enable) {
    dt2w_enabled_->store(enable, std::memory_order_release);
    const bool touch_success = xiaomi_touch_->enable_dt2w(enable);
    const bool procfs_success =
        enable ? procfs_->enable_dt2w() : procfs_->disable_dt2w();
    return touch_success || procfs_success;
}

bool TouchHal::set_aod(bool enable) {
    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    if (!aod) {
        return false;
    }
    return enable ? aod->enable_aod() : aod->disable_aod();
}

bool TouchHal::set_aod_brightness(uint8_t brightness) {
    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    return aod && aod->set_brightness(brightness);
}

bool TouchHal::set_full_screen_aod(bool enable) {
    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    if (!aod) {
        return false;
    }
    return enable ? aod->enable_full_screen() : aod->disable_full_screen();
}

bool TouchHal::set_seamless_aod(bool enable) {
    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    if (!aod) {
        return false;
    }
    return enable ? aod->enable_seamless() : aod->disable_seamless();
}

bool TouchHal::set_game_mode(bool enable) {
    const bool touch_success = xiaomi_touch_->set_game_mode(enable);
    const bool sysfs_success = sysfs_->set_game_mode(enable);
    return touch_success || sysfs_success;
}

bool TouchHal::set_report_rate(uint32_t rate) {
    const bool touch_success =
        xiaomi_touch_->set_report_rate(static_cast<int32_t>(rate));
    const bool sysfs_success = sysfs_->set_report_rate(rate);
    return touch_success || sysfs_success;
}

bool TouchHal::handle_kernel_gesture(uint8_t gesture_id) {
    std::shared_ptr<GestureHandler> gesture;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        gesture = gesture_handler_;
    }
    if (!gesture || !gesture->handle_kernel_gesture(gesture_id)) {
        return false;
    }
    if (gesture_id == gesture_ids::DoubleTap) {
        (void)aod_gesture_handler_.handle_aod_double_tap(0, 0);
    }
    return true;
}

bool TouchHal::handle_proximity(bool near) {
    std::shared_ptr<AodManager> aod;
    {
        std::lock_guard<std::mutex> lock(components_mutex_);
        aod = aod_manager_;
    }
    return aod && aod->on_proximity(near);
}

}  // namespace oplus::touch
