#pragma once

#include <optional>
#include <string>
#include <vector>

namespace oplus::touch {

std::optional<std::string> discover_input_device(const std::vector<std::string>& keywords);
std::optional<std::string> configured_or_discover(const std::string& configured,
                                                  const std::vector<std::string>& keywords);
std::optional<std::string> find_touchscreen();

}  // namespace oplus::touch
