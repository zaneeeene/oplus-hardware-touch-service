#pragma once

#include "oplus_touch/config.hpp"

#include <cstdint>
#include <optional>
#include <string>

namespace oplus::touch {

class SysfsManager {
  public:
    explicit SysfsManager(SysfsConfig config) : config_(std::move(config)) {}

    bool write_bool(const std::string& name, bool value);
    bool write_u8(const std::string& name, uint8_t value);
    bool write_u32(const std::string& name, uint32_t value);
    bool write_string(const std::string& name, const std::string& value);

    std::optional<bool> read_bool(const std::string& name) const;
    std::optional<uint8_t> read_u8(const std::string& name) const;
    std::optional<uint32_t> read_u32(const std::string& name) const;
    std::optional<std::string> read_string(const std::string& name) const;

    bool exists(const std::string& name) const;
    void initialize();
    bool set_touch_mode(const std::string& mode);
    bool set_report_rate(uint32_t rate);
    bool set_game_mode(bool enable);
    bool wake_screen();

    const SysfsConfig& config() const { return config_; }

  private:
    std::optional<std::string> get_path(const std::string& name) const;

    SysfsConfig config_;
};

}  // namespace oplus::touch
