#pragma once

#include "oplus_touch/config.hpp"
#include "oplus_touch/sysfs.hpp"

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace oplus::touch {

struct RaiseToWakeEvent {
    uint64_t timestamp = 0;
    float value = 0.0F;
};

class RaiseToWakeManager {
  public:
    using EventCallback = std::function<void(const RaiseToWakeEvent&)>;

    RaiseToWakeManager(RaiseToWakeConfig config, std::shared_ptr<SysfsManager> sysfs);
    ~RaiseToWakeManager();

    RaiseToWakeManager(const RaiseToWakeManager&) = delete;
    RaiseToWakeManager& operator=(const RaiseToWakeManager&) = delete;

    bool start();
    void stop();
    bool running() const { return running_.load(std::memory_order_acquire); }
    void set_event_callback(EventCallback callback);

  private:
    void run_sensor_loop();
    std::optional<bool> setting_enabled() const;
    void emit_event(float value);

    RaiseToWakeConfig config_;
    std::shared_ptr<SysfsManager> sysfs_;
    std::atomic_bool running_{false};
    std::thread thread_;
    std::mutex callback_mutex_;
    EventCallback callback_;
};

int diagnose_sensors(uint64_t duration_seconds);

}  // namespace oplus::touch
