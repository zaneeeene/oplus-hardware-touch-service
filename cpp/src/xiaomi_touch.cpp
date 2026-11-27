#include "oplus_touch/xiaomi_touch.hpp"

#include "oplus_touch/common.hpp"

#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <unistd.h>

namespace oplus::touch {
namespace {

constexpr size_t kMaxBufferSize = 256;
constexpr uint32_t kXiaomiTouchMagic = static_cast<uint32_t>('T');
constexpr uint32_t kIoctlTypeShift = 8;

}  // namespace

XiaomiTouch XiaomiTouch::open_optional(const std::string& path) {
    XiaomiTouch device(path);
    if (!device.is_available()) {
        log_message(LogLevel::Warn,
                    "Cannot open " + path + ": continuing with procfs/sysfs");
    }
    return device;
}

XiaomiTouch::XiaomiTouch(std::string path) : path_(std::move(path)) {
    fd_ = open(path_.c_str(), O_RDWR | O_CLOEXEC);
    if (fd_ >= 0) {
        log_message(LogLevel::Info, "Opened xiaomi-touch device at " + path_);
        initialize();
    }
}

XiaomiTouch::~XiaomiTouch() {
    close_device();
}

XiaomiTouch::XiaomiTouch(XiaomiTouch&& other) noexcept {
    std::lock_guard<std::mutex> lock(other.mutex_);
    fd_ = other.fd_;
    path_ = std::move(other.path_);
    mode_cache_ = other.mode_cache_;
    other.fd_ = -1;
}

XiaomiTouch& XiaomiTouch::operator=(XiaomiTouch&& other) noexcept {
    if (this == &other) {
        return *this;
    }
    close_device();
    std::lock_guard<std::mutex> lock(other.mutex_);
    fd_ = other.fd_;
    path_ = std::move(other.path_);
    mode_cache_ = other.mode_cache_;
    other.fd_ = -1;
    return *this;
}

void XiaomiTouch::close_device() {
    if (fd_ >= 0) {
        close(fd_);
        fd_ = -1;
        log_message(LogLevel::Debug, "Closed xiaomi-touch device");
    }
}

XiaomiModeTable XiaomiTouch::initial_mode_table() {
    XiaomiModeTable table{};
    for (auto& values : table) {
        values[0] = 1;
        values[1] = 1;
        values[2] = 0;
        values[3] = 1;
    }
    table[static_cast<size_t>(XiaomiTouchMode::AodEnable)] = {1, 1, 0, 1, 0, 0};
    table[static_cast<size_t>(XiaomiTouchMode::DoubletapMode)] = {1, 1, 0, 1, 0, 0};
    table[static_cast<size_t>(XiaomiTouchMode::FodEnable)] = {1, 1, 0, 1, 0, 0};
    table[static_cast<size_t>(XiaomiTouchMode::ReportRate)] = {60, 60, 30, 120, 0, 0};
    return table;
}

void XiaomiTouch::initialize() {
    mode_cache_ = initial_mode_table();
}

unsigned long XiaomiTouch::ioctl_request(XiaomiTouchCmd command) {
    return static_cast<unsigned long>((kXiaomiTouchMagic << kIoctlTypeShift) |
                                      static_cast<uint32_t>(command));
}

std::optional<int32_t> XiaomiTouch::ioctl(XiaomiTouchCmd command, XiaomiTouchMode mode,
                                          int32_t value) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ < 0) {
        return std::nullopt;
    }

    std::array<int32_t, kMaxBufferSize> buffer{};
    buffer[0] = static_cast<int32_t>(mode);
    buffer[1] = value;
    int result = ::ioctl(fd_, ioctl_request(command), buffer.data());
    if (result < 0) {
        log_message(LogLevel::Debug, std::string("xiaomi-touch ioctl failed: ") +
                                         std::strerror(errno));
        return std::nullopt;
    }

    if (command == XiaomiTouchCmd::SetCurValue || command == XiaomiTouchCmd::ResetMode) {
        mode_cache_[static_cast<size_t>(mode)][0] = buffer[1];
    }
    log_message(LogLevel::Debug,
                "xiaomi-touch ioctl cmd=" + std::to_string(static_cast<uint32_t>(command)) +
                    " mode=" + std::to_string(static_cast<uint32_t>(mode)) +
                    " value=" + std::to_string(value) +
                    " result=" + std::to_string(buffer[1]));
    return buffer[1];
}

std::optional<int32_t> XiaomiTouch::get_mode(XiaomiTouchMode mode) {
    return ioctl(XiaomiTouchCmd::GetCurValue, mode, 0);
}

std::optional<int32_t> XiaomiTouch::set_mode(XiaomiTouchMode mode, int32_t value) {
    return ioctl(XiaomiTouchCmd::SetCurValue, mode, value);
}

bool XiaomiTouch::enable_aod(bool enable) {
    const bool success = set_mode(XiaomiTouchMode::AodEnable, enable ? 1 : 0).has_value();
    log_message(LogLevel::Info, enable ? "AOD enabled" : "AOD disabled");
    return success;
}

bool XiaomiTouch::enable_dt2w(bool enable) {
    const bool success =
        set_mode(XiaomiTouchMode::DoubletapMode, enable ? 1 : 0).has_value();
    log_message(LogLevel::Info, enable ? "DT2W enabled" : "DT2W disabled");
    return success;
}

bool XiaomiTouch::enable_fod(bool enable) {
    const bool success = set_mode(XiaomiTouchMode::FodEnable, enable ? 1 : 0).has_value();
    log_message(LogLevel::Info, enable ? "FOD enabled" : "FOD disabled");
    return success;
}

bool XiaomiTouch::set_report_rate(int32_t rate) {
    const bool success = set_mode(XiaomiTouchMode::ReportRate, rate).has_value();
    if (success) {
        log_message(LogLevel::Info, "Report rate set to " + std::to_string(rate));
    }
    return success;
}

bool XiaomiTouch::set_game_mode(bool enable) {
    const bool success = set_mode(XiaomiTouchMode::GameMode, enable ? 1 : 0).has_value();
    log_message(LogLevel::Info, enable ? "Game mode enabled" : "Game mode disabled");
    return success;
}

XiaomiModeTable XiaomiTouch::all_modes() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return mode_cache_;
}

}  // namespace oplus::touch
