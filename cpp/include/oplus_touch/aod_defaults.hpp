#pragma once

#include "oplus_touch/config.hpp"

#include <atomic>
#include <optional>
#include <thread>

namespace oplus::touch {

class AodDefaultsWorker {
  public:
    explicit AodDefaultsWorker(AodDefaultsConfig config);
    ~AodDefaultsWorker();

    AodDefaultsWorker(const AodDefaultsWorker&) = delete;
    AodDefaultsWorker& operator=(const AodDefaultsWorker&) = delete;

    bool start();
    void stop();
    bool running() const { return running_.load(std::memory_order_acquire); }

  private:
    void run();
    void apply_every_boot();
    std::optional<std::string> read_secure_setting(const std::string& key) const;
    bool put_secure_setting(const std::string& key, const std::string& value) const;

    AodDefaultsConfig config_;
    std::atomic_bool running_{false};
    std::thread thread_;
};

}  // namespace oplus::touch
