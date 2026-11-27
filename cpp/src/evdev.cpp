#include "oplus_touch/evdev.hpp"

#include "oplus_touch/common.hpp"

#include <sstream>

namespace oplus::touch {

std::optional<std::string> discover_input_device(const std::vector<std::string>& keywords) {
    auto table = read_file("/proc/bus/input/devices");
    if (!table) {
        return std::nullopt;
    }

    std::vector<std::string> lowered;
    lowered.reserve(keywords.size());
    for (const auto& keyword : keywords) {
        lowered.push_back(lower_ascii(keyword));
    }

    size_t position = 0;
    while (position <= table->size()) {
        size_t end = table->find("\n\n", position);
        if (end == std::string::npos) {
            end = table->size();
        }
        const std::string block = table->substr(position, end - position);
        const std::string lower_block = lower_ascii(block);

        bool matched = false;
        for (const auto& keyword : lowered) {
            if (lower_block.find(keyword) != std::string::npos) {
                matched = true;
                break;
            }
        }
        if (matched) {
            std::istringstream lines(block);
            std::string line;
            while (std::getline(lines, line)) {
                constexpr const char* prefix = "H: Handlers=";
                if (!starts_with(line, prefix)) {
                    continue;
                }
                std::istringstream handlers(line.substr(std::char_traits<char>::length(prefix)));
                std::string handler;
                while (handlers >> handler) {
                    if (!starts_with(handler, "event")) {
                        continue;
                    }
                    const std::string path = "/dev/input/" + handler;
                    if (path_exists(path)) {
                        return path;
                    }
                }
            }
        }
        if (end == table->size()) {
            break;
        }
        position = end + 2;
    }
    return std::nullopt;
}

std::optional<std::string> configured_or_discover(const std::string& configured,
                                                  const std::vector<std::string>& keywords) {
    if (!configured.empty() && path_exists(configured)) {
        return configured;
    }
    return discover_input_device(keywords);
}

std::optional<std::string> find_touchscreen() {
    return discover_input_device({"touchscreen", "touch screen", "touch_dev", "nt36",
                                  "focaltech", "ft3", "ft5", "goodix", "synaptics",
                                  "sec_ts", "ili2", "ili9"});
}

}  // namespace oplus::touch
