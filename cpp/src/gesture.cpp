#include "oplus_touch/gesture.hpp"

#include "oplus_touch/common.hpp"
#include "oplus_touch/evdev.hpp"
#include "oplus_touch/procfs.hpp"
#include "oplus_touch/screen_state.hpp"
#include "oplus_touch/uinput.hpp"
#include "oplus_touch/wake.hpp"

#include <array>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <linux/input.h>
#include <unistd.h>
#include <vector>

namespace oplus::touch {
namespace {

constexpr uint16_t kEvKey = 0x01;
constexpr uint16_t kEvAbs = 0x03;
constexpr uint16_t kAbsMtPositionX = 0x35;
constexpr uint16_t kAbsMtPositionY = 0x36;
constexpr uint16_t kAbsMtTrackingId = 0x39;
constexpr uint16_t kBtnTouch = 0x14a;
constexpr uint16_t kKeyWakeup = 143;
constexpr uint16_t kKeyPower = 116;
constexpr uint16_t kKeyF4 = 62;
constexpr uint16_t kKeyGoto = 0x0162;

constexpr uint64_t kTapWindowMs = 350;
constexpr uint64_t kPreviewDebounceMs = 500;

#ifdef OPLUS_GESTURE_BACKLIGHT
const std::vector<std::string> kTouchscreenNames = {
    "touchscreen", "touch screen", "nt36", "focaltech", "goodix", "fts",
};
#else
const std::vector<std::string> kTouchscreenNames = {
    "touchscreen", "touch screen", "nt36", "focaltech", "goodix",
};
#endif

}  // namespace

GestureIdentity gesture_from_id(uint8_t id) {
    switch (id) {
        case 0x22:
            return {GestureType::DoubleTap, id};
        case 0x23:
            return {GestureType::SingleTap, id};
        case 0x24:
            return {GestureType::SwipeUp, id};
        case 0x25:
            return {GestureType::SwipeDown, id};
        case 0x26:
            return {GestureType::SwipeLeft, id};
        case 0x27:
            return {GestureType::SwipeRight, id};
        default:
            return {GestureType::Custom, id};
    }
}

TapOutcome TapOutcome::none() {
    return {};
}

TapOutcome TapOutcome::single(uint64_t sequence_id, int x, int y) {
    TapOutcome outcome;
    outcome.kind = TapOutcomeKind::Single;
    outcome.sequence_id = sequence_id;
    outcome.x = x;
    outcome.y = y;
    return outcome;
}

TapOutcome TapOutcome::double_tap(int x, int y) {
    TapOutcome outcome;
    outcome.kind = TapOutcomeKind::Double;
    outcome.x = x;
    outcome.y = y;
    return outcome;
}

void TapState::note_down(uint64_t now, bool screen_off) {
    finger_down = true;
    if (tap_count == 1 && now >= last_tap_ms && now - last_tap_ms <= kTapWindowMs) {
        second_tap_started = true;
        ++sequence_id;
        return;
    }

    ++sequence_id;
    tap_count = 0;
    second_tap_started = false;
    sequence_screen_off = screen_off;
}

TapOutcome TapState::finish(uint64_t now, bool double_enabled, bool single_enabled) {
    finger_down = false;
    last_tap_ms = now;

    if (second_tap_started) {
        tap_count = 0;
        second_tap_started = false;
        if (double_enabled && sequence_screen_off) {
            return TapOutcome::double_tap(x, y);
        }
        return TapOutcome::none();
    }

    tap_count = 1;
    if (single_enabled && sequence_screen_off) {
        return TapOutcome::single(sequence_id, x, y);
    }
    return TapOutcome::none();
}

GestureHandler::GestureHandler(GestureConfig config,
                               std::shared_ptr<XiaomiTouch> xiaomi_touch,
                               std::string touchscreen_path, std::string tp_gesture_path,
                               bool enable_uinput_wake,
                               std::shared_ptr<std::atomic_bool> dt2w_allowed)
    : config_(std::move(config)),
      xiaomi_touch_(std::move(xiaomi_touch)),
      tp_gesture_path_(std::move(tp_gesture_path)),
      dt2w_allowed_(std::move(dt2w_allowed)) {
    auto discovered = configured_or_discover(touchscreen_path, kTouchscreenNames);
    if (!discovered) {
        discovered = find_touchscreen();
    }
    if (discovered) {
        touchscreen_fd_ = ::open(discovered->c_str(), O_RDONLY | O_NONBLOCK | O_CLOEXEC);
        if (touchscreen_fd_ >= 0) {
            log_message(LogLevel::Info, "Watching touchscreen " + *discovered);
        } else {
            log_message(LogLevel::Warn, "Cannot open touchscreen " + *discovered + ": " +
                                            std::strerror(errno));
        }
    }
    if (touchscreen_fd_ < 0) {
        log_message(LogLevel::Warn,
                    "No touchscreen input event found; kernel gestures remain armed through " +
                        tp_gesture_path_);
    }

    if (enable_uinput_wake) {
        auto injector = WakeInjector::open("oplus-touch-gesture");
        if (injector.valid()) {
            wake_injector_ = std::make_unique<WakeInjector>(std::move(injector));
        } else {
            log_message(LogLevel::Debug, "uinput gesture injector unavailable");
        }
    }
}

GestureHandler::~GestureHandler() {
    stop();
    if (touchscreen_fd_ >= 0) {
        close(touchscreen_fd_);
        touchscreen_fd_ = -1;
    }
}

bool GestureHandler::start() {
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
        return true;
    }

    if (config_.double_tap.enabled && xiaomi_touch_) {
        (void)xiaomi_touch_->enable_dt2w(true);
    }
    enable_kernel_gestures();

    if (touchscreen_fd_ >= 0) {
        event_thread_ = std::thread(&GestureHandler::event_loop, this);
    }

    log_message(LogLevel::Info,
                "Gesture handler started (single-tap AOD preview " +
                    std::string(config_.single_tap.enabled ? "true" : "false") +
                    ", double-tap " +
                    std::string(config_.double_tap.enabled ? "true" : "false") + ")");
    return true;
}

void GestureHandler::stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }
    if (event_thread_.joinable()) {
        event_thread_.join();
    }
    {
        std::lock_guard<std::mutex> lock(wake_thread_mutex_);
        if (wake_thread_.joinable()) {
            wake_thread_.join();
        }
    }
    if (config_.double_tap.enabled && xiaomi_touch_) {
        (void)xiaomi_touch_->enable_dt2w(false);
    }
    log_message(LogLevel::Info, "Gesture handler stopped");
}

void GestureHandler::set_event_callback(EventCallback callback) {
    std::lock_guard<std::mutex> lock(callback_mutex_);
    callback_ = std::move(callback);
}

void GestureHandler::enable_kernel_gestures() {
    std::array<const GestureEntry*, 6> entries = {
        &config_.double_tap, &config_.single_tap, &config_.swipe_up,
        &config_.swipe_down, &config_.swipe_left, &config_.swipe_right,
    };
    uint64_t mask = 0;
    for (const GestureEntry* entry : entries) {
        if (entry->enabled && entry->gesture_id < 64) {
            mask |= uint64_t{1} << entry->gesture_id;
        }
    }
    for (const auto& entry : config_.custom) {
        if (entry.enabled && entry.gesture_id < 64) {
            mask |= uint64_t{1} << entry.gesture_id;
        }
    }

    ProcfsTouch procfs(tp_gesture_path_, "/proc/xiaomi-touch");
    if (procfs.set_gesture_mask(mask)) {
        log_message(LogLevel::Info, "Kernel gestures armed through " + tp_gesture_path_);
    } else {
        log_message(LogLevel::Warn,
                    "Could not arm " + tp_gesture_path_ +
                        "; xiaomi-touch remains the fallback");
    }
}

void GestureHandler::event_loop() {
    std::array<struct input_event, 64> buffer{};
    while (running_.load(std::memory_order_acquire)) {
        ssize_t count = read(touchscreen_fd_, buffer.data(), sizeof(buffer));
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                sleep_ms(20);
            } else {
                log_message(LogLevel::Error,
                            std::string("Touch event loop stopped: ") + std::strerror(errno));
                break;
            }
        } else if (count > 0) {
            process_events(buffer.data(), static_cast<size_t>(count) / sizeof(input_event));
        } else {
            sleep_ms(20);
        }

        if (pending_single_.active && now_ms() >= pending_single_.due_ms) {
            finish_pending_single();
        }
    }
}

void GestureHandler::process_events(const struct input_event* events, size_t count) {
    TapOutcome outcome;
    bool kernel_reported_wake = false;
    bool direct_preview = false;
    int direct_x = 0;
    int direct_y = 0;

    for (size_t index = 0; index < count; ++index) {
        const auto& event = events[index];
        if (event.type == kEvAbs && event.code == kAbsMtPositionX) {
            tap_state_.x = event.value;
            continue;
        }
        if (event.type == kEvAbs && event.code == kAbsMtPositionY) {
            tap_state_.y = event.value;
            continue;
        }
        if (event.type == kEvAbs && event.code == kAbsMtTrackingId) {
            bool down = event.value >= 0;
            if (down != tap_state_.finger_down) {
                if (down) {
                    note_touch_down(&tap_state_);
                } else {
                    outcome = finish_tap(&tap_state_);
                }
            }
            continue;
        }
        if (event.type == kEvKey && event.code == kBtnTouch) {
            bool down = event.value != 0;
            if (down != tap_state_.finger_down) {
                if (down) {
                    note_touch_down(&tap_state_);
                } else {
                    outcome = finish_tap(&tap_state_);
                }
            }
            continue;
        }
        if (event.type == kEvKey && event.code == kKeyGoto && event.value == 1) {
            const uint64_t now = now_ms();
            ScreenState screen = read_screen_state();
            log_message(LogLevel::Debug,
                        "Kernel AOD single-tap KEY_GOTO at (" + std::to_string(tap_state_.x) +
                            ", " + std::to_string(tap_state_.y) + "), screen=" +
                            screen_state_name(screen));
            if (now >= tap_state_.last_preview_ms &&
                now - tap_state_.last_preview_ms >= kPreviewDebounceMs &&
                screen_is_off_or_doze(screen)) {
                ++tap_state_.sequence_id;
                tap_state_.tap_count = 0;
                tap_state_.second_tap_started = false;
                tap_state_.last_preview_ms = now;
                direct_preview = true;
                direct_x = tap_state_.x;
                direct_y = tap_state_.y;
            }
            continue;
        }
        if (event.type == kEvKey &&
            (event.code == kKeyWakeup || event.code == kKeyPower || event.code == kKeyF4) &&
            event.value == 1) {
            if (screen_is_off_or_doze(read_screen_state())) {
                kernel_reported_wake = true;
                outcome = TapOutcome::double_tap(tap_state_.x, tap_state_.y);
            }
        }
    }

    if (direct_preview) {
        (void)emit_gesture({GestureType::SingleTap, gesture_ids::SingleTap}, direct_x,
                           direct_y);
        return;
    }

    if (outcome.kind == TapOutcomeKind::Double) {
        if (kernel_reported_wake) {
            log_message(LogLevel::Debug,
                        "Touch driver reported a wake key; requesting an explicit power wake anyway");
        }
        (void)emit_gesture({GestureType::DoubleTap, gesture_ids::DoubleTap}, outcome.x,
                           outcome.y);
    } else if (outcome.kind == TapOutcomeKind::Single) {
        schedule_single_tap(outcome.sequence_id, outcome.x, outcome.y);
    }
}

void GestureHandler::note_touch_down(TapState* state) {
    const ScreenState screen = read_screen_state();
    if (screen == ScreenState::Unknown) {
        log_message(LogLevel::Debug,
                    "Screen state unavailable; suppressing userspace tap actions");
    }
    state->note_down(now_ms(), screen_is_off_or_doze(screen));
}

TapOutcome GestureHandler::finish_tap(TapState* state) {
    const bool dt2w_allowed =
        config_.double_tap.enabled && dt2w_allowed_ &&
        dt2w_allowed_->load(std::memory_order_acquire);
    return state->finish(now_ms(), dt2w_allowed, config_.single_tap.enabled);
}

void GestureHandler::schedule_single_tap(uint64_t sequence_id, int x, int y) {
    pending_single_.active = true;
    pending_single_.sequence_id = sequence_id;
    pending_single_.x = x;
    pending_single_.y = y;
    pending_single_.due_ms = now_ms() + kTapWindowMs;
}

void GestureHandler::finish_pending_single() {
    if (!pending_single_.active) {
        return;
    }
    const uint64_t now = now_ms();
    if (!running_.load(std::memory_order_acquire) ||
        tap_state_.sequence_id != pending_single_.sequence_id ||
        tap_state_.tap_count != 1 || tap_state_.finger_down ||
        tap_state_.second_tap_started ||
        (now >= tap_state_.last_preview_ms &&
         now - tap_state_.last_preview_ms < kPreviewDebounceMs)) {
        pending_single_.active = false;
        return;
    }
    if (!tap_state_.sequence_screen_off || gesture_screen_is_awake()) {
        pending_single_.active = false;
        return;
    }

    pending_single_.active = false;
    tap_state_.tap_count = 0;
    ++tap_state_.sequence_id;
    tap_state_.last_preview_ms = now;
    log_message(LogLevel::Info,
                "Userspace AOD single-tap at (" + std::to_string(pending_single_.x) + ", " +
                    std::to_string(pending_single_.y) +
                    "); requesting preview dispatch");
    (void)emit_gesture({GestureType::SingleTap, gesture_ids::SingleTap}, pending_single_.x,
                       pending_single_.y);
}

bool GestureHandler::emit_gesture(GestureIdentity identity, int x, int y) {
    const GestureEntry* entry = entry_for(identity);
    if (entry == nullptr || !entry->enabled) {
        return true;
    }

    const std::string action = entry->action;
    const bool wake_screen = entry->wake_screen;
    if (identity.type == GestureType::SingleTap && action == "aod_preview" &&
        !wake_screen) {
        if (!screen_is_off_or_doze(read_screen_state())) {
            log_message(LogLevel::Debug,
                        "AOD preview suppressed because the display is awake");
            return true;
        }
        inject_aod_preview();
    } else if (wake_screen) {
        request_dt2w_wake(identity);
    }

    GestureEvent event;
    event.identity = identity;
    event.timestamp = now_ms();
    event.x = x;
    event.y = y;
    event.wake_screen = wake_screen;
    event.action = action;
    {
        std::lock_guard<std::mutex> lock(callback_mutex_);
        if (callback_) {
            callback_(event);
        }
    }
    log_message(LogLevel::Debug, "Emitted gesture id=" + std::to_string(identity.raw_id));
    return true;
}

void GestureHandler::inject_aod_preview() {
    hold_wake_lock("oplus_hardware_touch_service_preview", 1500);
#ifdef OPLUS_GESTURE_BACKLIGHT
    const std::vector<std::string> command = {"/system/bin/input", "keyevent", "134"};
#else
    const std::vector<std::string> command = {"/system/bin/cmd", "input", "keyevent", "134"};
#endif
    CommandResult result = run_command(command, false, true);
    if (result.started && result.exit_code == 0) {
        log_message(LogLevel::Info, "Dispatched Android keycode 134 for AOD single-tap preview");
        return;
    }

    log_message(LogLevel::Warn,
                "AOD preview cmd failed; trying the uinput KEY_F4 fallback");
    if (wake_injector_ && wake_injector_->emit_aod_preview()) {
        log_message(LogLevel::Info, "Injected uinput KEY_F4 for AOD single-tap preview fallback");
    } else {
        log_message(LogLevel::Warn,
                    "AOD preview has no dispatch path: uinput injector is unavailable");
    }
}

void GestureHandler::request_dt2w_wake(GestureIdentity identity) {
    if (wake_retry_running_.exchange(true, std::memory_order_acq_rel)) {
        log_message(LogLevel::Debug,
                    "Wake gesture arrived while a wake request is already pending");
        return;
    }
    std::lock_guard<std::mutex> lock(wake_thread_mutex_);
    if (wake_thread_.joinable()) {
        wake_thread_.join();
    }
    wake_thread_ = std::thread(&GestureHandler::wake_worker, this, identity);
}

void GestureHandler::wake_worker(GestureIdentity identity) {
    (void)identity;
    for (int attempt = 1; attempt <= 2; ++attempt) {
        if (!screen_is_off_or_doze(read_screen_state())) {
            log_message(LogLevel::Debug,
                        "Wake gesture: display became awake before attempt " +
                            std::to_string(attempt));
            break;
        }

        hold_wake_lock("oplus_hardware_touch_service_dt2w", 2000);
        std::string error;
        if (request_power_wakeup(&error)) {
            log_message(LogLevel::Info,
                        "DT2W attempt " + std::to_string(attempt) +
                            ": requested Android power wake");
        } else {
            log_message(LogLevel::Warn,
                        "DT2W power wake attempt " + std::to_string(attempt) +
                            " failed: " + error);
            inject_uinput_wake();
        }

        sleep_ms(250);
        const ScreenState state = read_screen_state();
        if (!screen_is_off_or_doze(state)) {
            log_message(LogLevel::Info,
                        "DT2W wake confirmed after attempt " + std::to_string(attempt) +
                            " (state=" + screen_state_name(state) + ")");
            break;
        }
        if (attempt == 2) {
            log_message(LogLevel::Warn,
                        std::string("DT2W wake request completed but the display is still ") +
                            screen_state_name(state));
        } else {
            log_message(LogLevel::Debug,
                        "DT2W wake not visible after attempt " + std::to_string(attempt) +
                            "; retrying once");
        }
    }
    wake_retry_running_.store(false, std::memory_order_release);
}

void GestureHandler::inject_uinput_wake() {
    if (wake_injector_ && wake_injector_->emit_wake()) {
        log_message(LogLevel::Info, "Injected uinput KEY_WAKEUP fallback");
    } else {
        log_message(LogLevel::Warn, "No uinput wake injector is available");
    }
}

const GestureEntry* GestureHandler::entry_for(GestureIdentity identity) const {
    switch (identity.type) {
        case GestureType::DoubleTap:
            return &config_.double_tap;
        case GestureType::SingleTap:
            return &config_.single_tap;
        case GestureType::SwipeUp:
            return &config_.swipe_up;
        case GestureType::SwipeDown:
            return &config_.swipe_down;
        case GestureType::SwipeLeft:
            return &config_.swipe_left;
        case GestureType::SwipeRight:
            return &config_.swipe_right;
        case GestureType::Custom:
        default:
            for (const auto& entry : config_.custom) {
                if (entry.gesture_id == identity.raw_id) {
                    return &entry;
                }
            }
            return nullptr;
    }
}

bool GestureHandler::handle_kernel_gesture(uint8_t gesture_id) {
    return emit_gesture(gesture_from_id(gesture_id), 0, 0);
}

bool AodGestureHandler::handle_aod_double_tap(int x, int y) {
    std::lock_guard<std::mutex> lock(mutex_);
    expanded_ = !expanded_;
    if (expanded_) {
        log_message(LogLevel::Info,
                    "AOD: expanding music/activity at (" + std::to_string(x) + ", " +
                        std::to_string(y) + ")");
    } else {
        log_message(LogLevel::Info, "AOD: collapsing music/activity view");
    }
    return expanded_;
}

bool AodGestureHandler::is_expanded() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return expanded_;
}

void AodGestureHandler::collapse() {
    std::lock_guard<std::mutex> lock(mutex_);
    expanded_ = false;
}

bool panel_backlight_is_zero() {
    auto brightness =
        read_int_file("/sys/class/backlight/panel0-backlight/brightness");
    return brightness && *brightness == 0;
}

bool gesture_screen_is_off_or_doze() {
    const ScreenState state = read_screen_state();
#ifdef OPLUS_GESTURE_BACKLIGHT
    return screen_is_off_or_doze(state) || panel_backlight_is_zero();
#else
    return screen_is_off_or_doze(state);
#endif
}

bool gesture_screen_is_awake() {
    const ScreenState state = read_screen_state();
#ifdef OPLUS_GESTURE_BACKLIGHT
    return screen_is_on(state) && !panel_backlight_is_zero();
#else
    return screen_is_on(state);
#endif
}

}  // namespace oplus::touch
