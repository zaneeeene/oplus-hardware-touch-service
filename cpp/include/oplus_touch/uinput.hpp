#pragma once

#include <cstdint>
#include <string>

namespace oplus::touch {

class WakeInjector {
  public:
    static WakeInjector open(const std::string& name);
    WakeInjector() = default;
    explicit WakeInjector(int fd) : fd_(fd) {}
    ~WakeInjector();

    WakeInjector(const WakeInjector&) = delete;
    WakeInjector& operator=(const WakeInjector&) = delete;
    WakeInjector(WakeInjector&& other) noexcept;
    WakeInjector& operator=(WakeInjector&& other) noexcept;

    bool valid() const { return fd_ >= 0; }
    bool emit_wake() const;
    bool emit_aod_preview() const;

  private:
    bool emit_key(uint16_t code) const;
    void destroy();

    int fd_ = -1;
};

}  // namespace oplus::touch
