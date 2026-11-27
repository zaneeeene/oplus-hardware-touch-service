#pragma once

#include "oplus_touch/config.hpp"
#include "oplus_touch/xiaomi_touch.hpp"

#include <atomic>
#include <cstdint>
#include <functional>
#include <linux/input.h>
#include <memory>
#include <mutex>
#include <string>
#include <thread>

namespace oplus::touch {

class WakeInjector;

enum class GestureType : uint8_t {
    DoubleTap = 0x22,
    SingleTap = 0x23,
    SwipeUp = 0x24,
    SwipeDown = 0x25,
    SwipeLeft = 0x26,
    SwipeRight = 0x27,
    Custom = 0xff,
};

struct GestureIdentity {
    GestureType type = GestureType::Custom;
    uint8_t raw_id = 0xff;
};

GestureIdentity gesture_from_id(uint8_t id);

struct GestureEvent {
    GestureIdentity identity{};
    uint64_t timestamp = 0;
    int x = 0;
    int y = 0;
    bool wake_screen = false;
    std::string action;
};

enum class TapOutcomeKind {
    None,
    Single,
    Double,
};

struct TapOutcome {
    TapOutcomeKind kind = TapOutcomeKind::None;
    uint64_t sequence_id = 0;
    int x = 0;
    int y = 0;

    static TapOutcome none();
    static TapOutcome single(uint64_t sequence_id, int x, int y);
    static TapOutcome double_tap(int x, int y);
};

// Pure tap classifier. It is public so host tests can exercise the same state
// machine used by the Android daemon.
struct TapState {
    int x = 0;
    int y = 0;
    bool finger_down = false;
    uint32_t tap_count = 0;
    uint64_t last_tap_ms = 0;
    uint64_t sequence_id = 0;
    bool second_tap_started = false;
    bool sequence_screen_off = false;
    uint64_t last_preview_ms = 0;

    void note_down(uint64_t now, bool screen_off);
    TapOutcome finish(uint64_t now, bool double_enabled, bool single_enabled);
};

class GestureHandler {
  public:
    using EventCallback = std::function<void(const GestureEvent&)>;

    GestureHandler(GestureConfig config, std::shared_ptr<XiaomiTouch> xiaomi_touch,
                   std::string touchscreen_path, std::string tp_gesture_path,
                   bool enable_uinput_wake, std::shared_ptr<std::atomic_bool> dt2w_allowed);
    ~GestureHandler();

    GestureHandler(const GestureHandler&) = delete;
    GestureHandler& operator=(const GestureHandler&) = delete;

    bool start();
    void stop();
    void set_event_callback(EventCallback callback);
    bool handle_kernel_gesture(uint8_t gesture_id);

    bool running() const { return running_.load(std::memory_order_acquire); }
    const GestureConfig& config() const { return config_; }

  private:
    struct PendingSingle {
        bool active = false;
        uint64_t sequence_id = 0;
        int x = 0;
        int y = 0;
        uint64_t due_ms = 0;
    };

    void enable_kernel_gestures();
    void event_loop();
    void process_events(const struct input_event* events, size_t count);
    void note_touch_down(TapState* state);
    TapOutcome finish_tap(TapState* state);
    void schedule_single_tap(uint64_t sequence_id, int x, int y);
    void finish_pending_single();
    bool emit_gesture(GestureIdentity identity, int x, int y);
    void inject_aod_preview();
    void request_dt2w_wake(GestureIdentity identity);
    void wake_worker(GestureIdentity identity);
    void inject_uinput_wake();
    const GestureEntry* entry_for(GestureIdentity identity) const;

    GestureConfig config_;
    std::shared_ptr<XiaomiTouch> xiaomi_touch_;
    std::string tp_gesture_path_;
    int touchscreen_fd_ = -1;
    std::unique_ptr<WakeInjector> wake_injector_;
    std::shared_ptr<std::atomic_bool> dt2w_allowed_;

    std::atomic_bool running_{false};
    std::atomic_bool wake_retry_running_{false};
    std::thread event_thread_;
    std::thread wake_thread_;
    std::mutex callback_mutex_;
    EventCallback callback_;
    std::mutex wake_thread_mutex_;

    TapState tap_state_;
    PendingSingle pending_single_;
};

class AodGestureHandler {
  public:
    bool handle_aod_double_tap(int x, int y);
    bool is_expanded() const;
    void collapse();

  private:
    mutable std::mutex mutex_;
    bool expanded_ = false;
};

bool panel_backlight_is_zero();
bool gesture_screen_is_off_or_doze();
bool gesture_screen_is_awake();

}  // namespace oplus::touch
