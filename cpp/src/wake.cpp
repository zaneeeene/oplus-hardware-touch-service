#include "oplus_touch/wake.hpp"

#include "oplus_touch/common.hpp"

namespace oplus::touch {

void hold_wake_lock(const std::string& name, uint64_t duration_ms) {
    const std::string request = name + " " + std::to_string(duration_ms * 1000000ULL);
    (void)write_file("/sys/power/wake_lock", request);
}

void release_wake_lock(const std::string& name) {
    (void)write_file("/sys/power/wake_unlock", name);
}

bool request_power_wakeup(std::string* error) {
    CommandResult result = run_command({"/system/bin/cmd", "power", "wakeup"}, false, true);
    if (result.started && result.exit_code == 0) {
        return true;
    }
    if (error != nullptr) {
        if (!result.started) {
            *error = "cmd power wakeup could not start: " + result.error;
        } else {
            *error = "cmd power wakeup exited with " + std::to_string(result.exit_code) +
                     (result.error.empty() ? "" : ": " + result.error);
        }
    }
    return false;
}

}  // namespace oplus::touch
