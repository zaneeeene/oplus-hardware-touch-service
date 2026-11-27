#include "oplus_touch/uinput.hpp"

#include "oplus_touch/common.hpp"

#include <algorithm>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <linux/input.h>
#include <linux/uinput.h>
#include <sys/ioctl.h>
#include <unistd.h>

namespace oplus::touch {
namespace {

constexpr uint16_t kEvSyn = 0x00;
constexpr uint16_t kEvKey = 0x01;
constexpr uint16_t kSynReport = 0x00;
constexpr uint16_t kKeyWakeup = 143;
constexpr uint16_t kKeyF4 = 62;

bool set_bit(int fd, unsigned long request, uint16_t bit) {
    return ioctl(fd, request, static_cast<unsigned long>(bit)) >= 0;
}

bool write_all(int fd, const void* data, size_t size) {
    const auto* bytes = static_cast<const uint8_t*>(data);
    size_t offset = 0;
    while (offset < size) {
        ssize_t count = write(fd, bytes + offset, size - offset);
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            return false;
        }
        if (count == 0) {
            return false;
        }
        offset += static_cast<size_t>(count);
    }
    return true;
}

struct input_event event(uint16_t type, uint16_t code, int32_t value) {
    struct input_event result{};
    result.type = type;
    result.code = code;
    result.value = value;
    return result;
}

}  // namespace

WakeInjector WakeInjector::open(const std::string& name) {
    int fd = ::open("/dev/uinput", O_WRONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) {
        log_message(LogLevel::Debug, std::string("open /dev/uinput failed: ") +
                                         std::strerror(errno));
        return WakeInjector();
    }

    if (!set_bit(fd, UI_SET_EVBIT, kEvKey) || !set_bit(fd, UI_SET_KEYBIT, kKeyWakeup) ||
        !set_bit(fd, UI_SET_KEYBIT, kKeyF4)) {
        close(fd);
        return WakeInjector();
    }

    struct uinput_user_dev device{};
    const size_t name_length = std::min(name.size(), sizeof(device.name) - 1);
    std::memcpy(device.name, name.data(), name_length);
    device.id.bustype = 0x06;
    device.id.vendor = 0x18d1;
    device.id.product = 0x0001;
    device.id.version = 1;

    if (!write_all(fd, &device, sizeof(device)) || ioctl(fd, UI_DEV_CREATE, 0) < 0) {
        close(fd);
        return WakeInjector();
    }
    sleep_ms(150);
    return WakeInjector(fd);
}

WakeInjector::WakeInjector(WakeInjector&& other) noexcept : fd_(other.fd_) {
    other.fd_ = -1;
}

WakeInjector& WakeInjector::operator=(WakeInjector&& other) noexcept {
    if (this != &other) {
        destroy();
        fd_ = other.fd_;
        other.fd_ = -1;
    }
    return *this;
}

WakeInjector::~WakeInjector() {
    destroy();
}

void WakeInjector::destroy() {
    if (fd_ >= 0) {
        ioctl(fd_, UI_DEV_DESTROY, 0);
        close(fd_);
        fd_ = -1;
    }
}

bool WakeInjector::emit_key(uint16_t code) const {
    if (fd_ < 0) {
        return false;
    }
    const struct input_event events[] = {
        event(kEvKey, code, 1),
        event(kEvSyn, kSynReport, 0),
        event(kEvKey, code, 0),
        event(kEvSyn, kSynReport, 0),
    };
    for (const auto& value : events) {
        if (!write_all(fd_, &value, sizeof(value))) {
            return false;
        }
    }
    return true;
}

bool WakeInjector::emit_wake() const {
    return emit_key(kKeyWakeup);
}

bool WakeInjector::emit_aod_preview() const {
    return emit_key(kKeyF4);
}

}  // namespace oplus::touch
