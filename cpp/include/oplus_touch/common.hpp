#pragma once

#include <atomic>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace oplus::touch {

enum class LogLevel : int {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
};

void set_log_level(LogLevel level);
LogLevel log_level();
void log_message(LogLevel level, std::string_view message);

std::string trim(std::string_view value);
std::string lower_ascii(std::string_view value);
bool starts_with(std::string_view value, std::string_view prefix);
bool ends_with(std::string_view value, std::string_view suffix);

std::optional<std::string> read_file(const std::string& path);
bool write_file(const std::string& path, std::string_view content);
bool path_exists(const std::string& path);
std::optional<int> read_int_file(const std::string& path);

uint64_t now_ms();
void sleep_ms(uint64_t milliseconds);

std::optional<std::string> get_property(const std::string& name);
bool property_equals(const std::string& name, std::string_view expected);
bool wait_for_property(const std::string& name, std::string_view expected,
                       const std::atomic_bool& running, uint64_t timeout_ms,
                       uint64_t poll_ms = 250);

// Runs an executable without a shell. It redirects stdin/stdout/stderr to
// /dev/null unless capture_output is requested.
bool run_command_status(const std::vector<std::string>& args);
std::optional<std::string> run_command_capture(const std::vector<std::string>& args);

struct CommandResult {
    int exit_code = -1;
    bool started = false;
    std::string output;
    std::string error;
};

CommandResult run_command(const std::vector<std::string>& args, bool capture_stdout = false,
                          bool capture_stderr = false);

std::string errno_string(int error_number);
std::string join_strings(const std::vector<std::string>& values, std::string_view separator);

}  // namespace oplus::touch
