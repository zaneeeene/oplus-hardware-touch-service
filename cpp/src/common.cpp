#include "oplus_touch/common.hpp"

#include <algorithm>
#include <cerrno>
#include <chrono>
#include <climits>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <mutex>
#include <sys/wait.h>
#include <thread>
#include <unistd.h>

#ifdef __ANDROID__
#include <android/log.h>
#include <sys/system_properties.h>
#endif

namespace oplus::touch {
namespace {

std::atomic<int> g_log_level{static_cast<int>(LogLevel::Info)};
std::mutex g_log_mutex;

#ifndef __ANDROID__
const char* level_name(LogLevel level) {
    switch (level) {
        case LogLevel::Error:
            return "E";
        case LogLevel::Warn:
            return "W";
        case LogLevel::Info:
            return "I";
        case LogLevel::Debug:
            return "D";
        case LogLevel::Off:
        default:
            return "?";
    }
}
#endif

int redirect_to_null() {
    return open("/dev/null", O_RDWR | O_CLOEXEC);
}

void close_if_valid(int fd) {
    if (fd >= 0) {
        close(fd);
    }
}

}  // namespace

void set_log_level(LogLevel level) {
    g_log_level.store(static_cast<int>(level), std::memory_order_release);
}

LogLevel log_level() {
    return static_cast<LogLevel>(g_log_level.load(std::memory_order_acquire));
}

void log_message(LogLevel level, std::string_view message) {
    if (static_cast<int>(level) > g_log_level.load(std::memory_order_acquire) ||
        level == LogLevel::Off) {
        return;
    }
#ifdef __ANDROID__
    int priority = ANDROID_LOG_INFO;
    switch (level) {
        case LogLevel::Error:
            priority = ANDROID_LOG_ERROR;
            break;
        case LogLevel::Warn:
            priority = ANDROID_LOG_WARN;
            break;
        case LogLevel::Info:
            priority = ANDROID_LOG_INFO;
            break;
        case LogLevel::Debug:
            priority = ANDROID_LOG_DEBUG;
            break;
        default:
            break;
    }
    __android_log_print(priority, "oplus-touch-service", "%.*s",
                        static_cast<int>(message.size()), message.data());
#else
    std::lock_guard<std::mutex> lock(g_log_mutex);
    std::fprintf(stderr, "%s/oplus-touch-service: %.*s\n", level_name(level),
                 static_cast<int>(message.size()), message.data());
#endif
}

std::string trim(std::string_view value) {
    size_t begin = 0;
    while (begin < value.size() &&
           (value[begin] == ' ' || value[begin] == '\t' || value[begin] == '\r' ||
            value[begin] == '\n')) {
        ++begin;
    }
    size_t end = value.size();
    while (end > begin &&
           (value[end - 1] == ' ' || value[end - 1] == '\t' || value[end - 1] == '\r' ||
            value[end - 1] == '\n')) {
        --end;
    }
    return std::string(value.substr(begin, end - begin));
}

std::string lower_ascii(std::string_view value) {
    std::string result;
    result.reserve(value.size());
    for (char character : value) {
        if (character >= 'A' && character <= 'Z') {
            result.push_back(static_cast<char>(character - 'A' + 'a'));
        } else {
            result.push_back(character);
        }
    }
    return result;
}

bool starts_with(std::string_view value, std::string_view prefix) {
    return value.size() >= prefix.size() && value.substr(0, prefix.size()) == prefix;
}

bool ends_with(std::string_view value, std::string_view suffix) {
    return value.size() >= suffix.size() &&
           value.substr(value.size() - suffix.size()) == suffix;
}

std::optional<std::string> read_file(const std::string& path) {
    int fd = open(path.c_str(), O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        return std::nullopt;
    }
    std::string result;
    char buffer[4096];
    for (;;) {
        ssize_t count = read(fd, buffer, sizeof(buffer));
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            close(fd);
            return std::nullopt;
        }
        if (count == 0) {
            break;
        }
        result.append(buffer, static_cast<size_t>(count));
    }
    close(fd);
    return result;
}

bool write_file(const std::string& path, std::string_view content) {
    int fd = open(path.c_str(), O_WRONLY | O_CLOEXEC);
    if (fd < 0) {
        return false;
    }
    size_t offset = 0;
    while (offset < content.size()) {
        ssize_t count = write(fd, content.data() + offset, content.size() - offset);
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            close(fd);
            return false;
        }
        if (count == 0) {
            close(fd);
            return false;
        }
        offset += static_cast<size_t>(count);
    }
    close(fd);
    return true;
}

bool path_exists(const std::string& path) {
    return access(path.c_str(), F_OK) == 0;
}

std::optional<int> read_int_file(const std::string& path) {
    auto content = read_file(path);
    if (!content) {
        return std::nullopt;
    }
    const std::string trimmed = trim(*content);
    if (trimmed.empty()) {
        return std::nullopt;
    }
    char* end = nullptr;
    errno = 0;
    long value = std::strtol(trimmed.c_str(), &end, 10);
    if (errno != 0 || end == trimmed.c_str()) {
        return std::nullopt;
    }
    if (value < INT32_MIN || value > INT32_MAX) {
        return std::nullopt;
    }
    return static_cast<int>(value);
}

uint64_t now_ms() {
    const auto now = std::chrono::system_clock::now().time_since_epoch();
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(now).count());
}

void sleep_ms(uint64_t milliseconds) {
    std::this_thread::sleep_for(std::chrono::milliseconds(milliseconds));
}

std::optional<std::string> get_property(const std::string& name) {
#ifdef __ANDROID__
    char value[PROP_VALUE_MAX] = {};
    int length = __system_property_get(name.c_str(), value);
    if (length <= 0) {
        return std::nullopt;
    }
    return std::string(value, static_cast<size_t>(length));
#else
    const std::string env_name = "OPLUS_PROP_" + name;
    const char* value = std::getenv(env_name.c_str());
    if (value == nullptr || *value == '\0') {
        return std::nullopt;
    }
    return std::string(value);
#endif
}

bool property_equals(const std::string& name, std::string_view expected) {
    auto value = get_property(name);
    return value && trim(*value) == expected;
}

bool wait_for_property(const std::string& name, std::string_view expected,
                       const std::atomic_bool& running, uint64_t timeout_ms,
                       uint64_t poll_ms) {
    const uint64_t deadline = now_ms() + timeout_ms;
    while (running.load(std::memory_order_acquire)) {
        if (property_equals(name, expected)) {
            return true;
        }
        if (now_ms() >= deadline) {
            return false;
        }
        sleep_ms(poll_ms);
    }
    return false;
}

CommandResult run_command(const std::vector<std::string>& args, bool capture_stdout,
                          bool capture_stderr) {
    CommandResult result;
    if (args.empty()) {
        return result;
    }

    int stdout_pipe[2] = {-1, -1};
    int stderr_pipe[2] = {-1, -1};
    if (capture_stdout && pipe2(stdout_pipe, O_CLOEXEC) != 0) {
        result.error = errno_string(errno);
        return result;
    }
    if (capture_stderr && pipe2(stderr_pipe, O_CLOEXEC) != 0) {
        close_if_valid(stdout_pipe[0]);
        close_if_valid(stdout_pipe[1]);
        result.error = errno_string(errno);
        return result;
    }

    pid_t child = fork();
    if (child < 0) {
        result.error = errno_string(errno);
        close_if_valid(stdout_pipe[0]);
        close_if_valid(stdout_pipe[1]);
        close_if_valid(stderr_pipe[0]);
        close_if_valid(stderr_pipe[1]);
        return result;
    }
    if (child == 0) {
        int null_fd = redirect_to_null();
        if (null_fd >= 0) {
            dup2(null_fd, STDIN_FILENO);
            if (!capture_stdout) {
                dup2(null_fd, STDOUT_FILENO);
            }
            if (!capture_stderr) {
                dup2(null_fd, STDERR_FILENO);
            }
            if (null_fd > STDERR_FILENO) {
                close(null_fd);
            }
        }
        if (capture_stdout) {
            dup2(stdout_pipe[1], STDOUT_FILENO);
            close_if_valid(stdout_pipe[0]);
            close_if_valid(stdout_pipe[1]);
        }
        if (capture_stderr) {
            dup2(stderr_pipe[1], STDERR_FILENO);
            close_if_valid(stderr_pipe[0]);
            close_if_valid(stderr_pipe[1]);
        }

        std::vector<char*> argv;
        argv.reserve(args.size() + 1);
        for (const auto& arg : args) {
            argv.push_back(const_cast<char*>(arg.c_str()));
        }
        argv.push_back(nullptr);
        execv(argv[0], argv.data());
        _exit(127);
    }

    close_if_valid(stdout_pipe[1]);
    close_if_valid(stderr_pipe[1]);
    result.started = true;

    auto read_all = [](int fd, std::string* output) {
        char buffer[1024];
        for (;;) {
            ssize_t count = read(fd, buffer, sizeof(buffer));
            if (count < 0) {
                if (errno == EINTR) {
                    continue;
                }
                break;
            }
            if (count == 0) {
                break;
            }
            output->append(buffer, static_cast<size_t>(count));
        }
    };

    if (capture_stdout) {
        read_all(stdout_pipe[0], &result.output);
    }
    if (capture_stderr) {
        read_all(stderr_pipe[0], &result.error);
    }
    close_if_valid(stdout_pipe[0]);
    close_if_valid(stderr_pipe[0]);

    int status = 0;
    pid_t waited = -1;
    do {
        waited = waitpid(child, &status, 0);
    } while (waited < 0 && errno == EINTR);
    if (waited < 0) {
        result.error = errno_string(errno);
        result.exit_code = -1;
        return result;
    }
    if (WIFEXITED(status)) {
        result.exit_code = WEXITSTATUS(status);
    } else if (WIFSIGNALED(status)) {
        result.exit_code = 128 + WTERMSIG(status);
    }
    return result;
}

bool run_command_status(const std::vector<std::string>& args) {
    CommandResult result = run_command(args, false, false);
    return result.started && result.exit_code == 0;
}

std::optional<std::string> run_command_capture(const std::vector<std::string>& args) {
    CommandResult result = run_command(args, true, true);
    if (!result.started || result.exit_code != 0) {
        return std::nullopt;
    }
    return result.output;
}

std::string errno_string(int error_number) {
    return std::string(std::strerror(error_number));
}

std::string join_strings(const std::vector<std::string>& values, std::string_view separator) {
    std::string result;
    for (size_t index = 0; index < values.size(); ++index) {
        if (index != 0) {
            result.append(separator);
        }
        result.append(values[index]);
    }
    return result;
}

}  // namespace oplus::touch
