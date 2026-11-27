#include "oplus_touch/aod.hpp"

#include "oplus_touch/aod_defaults.hpp"
#include "oplus_touch/common.hpp"
#include "oplus_touch/doze.hpp"

#include <thread>

namespace oplus::touch {

AodManager::AodManager(const TouchConfig& config,
                       std::shared_ptr<XiaomiTouch> xiaomi_touch,
                       std::shared_ptr<SysfsManager> sysfs)
    : xiaomi_touch_(std::move(xiaomi_touch)), sysfs_(std::move(sysfs)) {
    config_.enabled = config.enable_aod;
    config_.brightness = config.aod_brightness;
    config_.full_screen = config.enable_full_screen_aod;
    config_.seamless = config.enable_seamless_aod;
    config_.keepalive = config.aod_keepalive;
    config_.defaults = config.aod_defaults;
    desired_on_ = config.enable_aod;
}

AodManager::~AodManager() {
    stop();
}

bool AodManager::start() {
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
        return true;
    }

    if (config_.enabled) {
        enable_aod();
    }

    if (config_.keepalive.enabled) {
        doze_keeper_ = std::make_unique<DozeKeeper>(config_.keepalive);
        (void)doze_keeper_->start();
    }
    if (config_.defaults.enabled) {
        defaults_worker_ = std::make_unique<AodDefaultsWorker>(config_.defaults);
        (void)defaults_worker_->start();
    }
    monitor_thread_ = std::thread(&AodManager::monitor_loop, this);
    log_message(LogLevel::Info, "AOD manager started");
    return true;
}

void AodManager::stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }
    if (monitor_thread_.joinable()) {
        monitor_thread_.join();
    }
    if (doze_keeper_) {
        doze_keeper_->stop();
        doze_keeper_.reset();
    }
    if (defaults_worker_) {
        defaults_worker_->stop();
        defaults_worker_.reset();
    }
    if (config_.enabled) {
        disable_aod();
    }
    log_message(LogLevel::Info, "AOD manager stopped");
}

bool AodManager::enable_aod() {
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        desired_on_ = true;
    }
    if (xiaomi_touch_ && !xiaomi_touch_->enable_aod(true)) {
        log_message(LogLevel::Warn, "Could not enable AOD through xiaomi-touch");
    }
    if (!sysfs_->write_bool("aod_enable", true)) {
        log_message(LogLevel::Warn, "Could not enable AOD through sysfs");
    }
    (void)set_brightness(config_.brightness);
    if (config_.full_screen) {
        (void)enable_full_screen();
    }
    if (config_.seamless) {
        (void)enable_seamless();
    }
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::On;
    }
    log_message(LogLevel::Info,
                "AOD enabled (full_screen_requested=" +
                    std::string(config_.full_screen ? "true" : "false") +
                    ", seamless_requested=" +
                    std::string(config_.seamless ? "true" : "false") + ")");
    return true;
}

bool AodManager::disable_aod() {
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        desired_on_ = false;
    }
    if (xiaomi_touch_) {
        (void)xiaomi_touch_->enable_aod(false);
    }
    (void)sysfs_->write_bool("aod_enable", false);
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::Off;
    }
    log_message(LogLevel::Info, "AOD disabled");
    return true;
}

bool AodManager::set_brightness(uint8_t brightness) {
    const bool success = sysfs_->write_u8("aod_brightness", brightness);
    if (success) {
        log_message(LogLevel::Debug,
                    "AOD brightness set to " + std::to_string(brightness));
    }
    return success;
}

bool AodManager::enable_full_screen() {
    if (!sysfs_->write_bool("aod_full_screen", true)) {
        return false;
    }
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::FullScreen;
    }
    log_message(LogLevel::Info, "Full-screen AOD enabled");
    return true;
}

bool AodManager::disable_full_screen() {
    if (!sysfs_->write_bool("aod_full_screen", false)) {
        return false;
    }
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::On;
    }
    log_message(LogLevel::Info, "Full-screen AOD disabled");
    return true;
}

bool AodManager::enable_seamless() {
    if (!sysfs_->write_bool("aod_seamless", true)) {
        return false;
    }
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::Seamless;
    }
    log_message(LogLevel::Info, "Seamless AOD enabled");
    return true;
}

bool AodManager::disable_seamless() {
    if (!sysfs_->write_bool("aod_seamless", false)) {
        return false;
    }
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::On;
    }
    log_message(LogLevel::Info, "Seamless AOD disabled");
    return true;
}

bool AodManager::dim() {
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        state_ = AodState::Dimmed;
    }
    const bool success = set_brightness(10);
    if (success) {
        log_message(LogLevel::Debug, "AOD dimmed");
    }
    return success;
}

bool AodManager::undim() {
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        if (state_ != AodState::Dimmed) {
            return true;
        }
        state_ = AodState::On;
    }
    const bool success = set_brightness(config_.brightness);
    if (success) {
        log_message(LogLevel::Debug, "AOD undimmed");
    }
    return success;
}

bool AodManager::on_proximity(bool near) {
    return near ? dim() : undim();
}

bool AodManager::on_dt2w() {
    if (!is_enabled()) {
        return true;
    }
    log_message(LogLevel::Info, "DT2W on AOD - waking screen");
    (void)sysfs_->write_bool("wake_screen", true);
    return true;
}

bool AodManager::on_music_double_tap(int x, int y) {
    if (!config_.music_control || !is_enabled()) {
        return false;
    }
    log_message(LogLevel::Info,
                "AOD music/activity double tap at (" + std::to_string(x) + ", " +
                    std::to_string(y) + ")");
    (void)sysfs_->write_bool("aod_expand_music", true);
    return true;
}

bool AodManager::collapse_music_view() {
    return sysfs_->write_bool("aod_expand_music", false);
}

AodState AodManager::state() const {
    std::lock_guard<std::mutex> lock(state_mutex_);
    return state_;
}

bool AodManager::is_enabled() const {
    const AodState current = state();
    return current == AodState::On || current == AodState::Dimmed ||
           current == AodState::Seamless || current == AodState::FullScreen;
}

void AodManager::monitor_loop() {
    while (running_.load(std::memory_order_acquire)) {
        // Keep shutdown responsive while still avoiding a wakeup-heavy poll.
        for (int tick = 0;
             tick < 100 && running_.load(std::memory_order_acquire); ++tick) {
            sleep_ms(100);
        }
        if (!running_.load(std::memory_order_acquire)) {
            break;
        }
        bool desired = false;
        {
            std::lock_guard<std::mutex> lock(state_mutex_);
            desired = desired_on_;
        }
        if (desired) {
            if (xiaomi_touch_) {
                (void)xiaomi_touch_->enable_aod(true);
            }
            (void)sysfs_->write_bool("aod_enable", true);
        }
    }
}

}  // namespace oplus::touch
