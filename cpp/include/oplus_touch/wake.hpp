#pragma once

#include <cstdint>
#include <string>

namespace oplus::touch {

void hold_wake_lock(const std::string& name, uint64_t duration_ms);
void release_wake_lock(const std::string& name);
bool request_power_wakeup(std::string* error = nullptr);

}  // namespace oplus::touch
