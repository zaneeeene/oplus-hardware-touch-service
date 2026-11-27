#include "oplus_touch/common.hpp"
#include "oplus_touch/config.hpp"
#include "oplus_touch/hal.hpp"
#include "oplus_touch/panel_feature.hpp"
#include "oplus_touch/procfs.hpp"
#include "oplus_touch/raise_to_wake.hpp"
#include "oplus_touch/xiaomi_touch.hpp"

#include <cerrno>
#include <csignal>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <pthread.h>
#include <string>
#include <unistd.h>
#include <vector>

namespace {

struct Options {
    oplus::touch::LogLevel log_level = oplus::touch::LogLevel::Info;
    bool foreground = false;
    bool test = false;
    bool diagnose_sensors = false;
    uint64_t sensor_seconds = 30;
};

void usage(std::ostream& output) {
    output
        << "Usage: vendor.oplus.hardware.touch-V2-service [options]\n"
        << "  -l, --log-level off|error|warn|info|debug  Log verbosity\n"
        << "  -f, --foreground                         Run in foreground\n"
        << "      --test                               Run built-in self-tests\n"
        << "      --diagnose-sensors                   List sensors and pickup events\n"
        << "      --sensor-seconds N                   Sensor probe duration (default 30)\n"
        << "  -h, --help                               Show this help\n";
}

std::optional<oplus::touch::LogLevel> parse_log_level(const std::string& value) {
    if (value == "off") {
        return oplus::touch::LogLevel::Off;
    }
    if (value == "error") {
        return oplus::touch::LogLevel::Error;
    }
    if (value == "warn" || value == "warning") {
        return oplus::touch::LogLevel::Warn;
    }
    if (value == "info") {
        return oplus::touch::LogLevel::Info;
    }
    if (value == "debug") {
        return oplus::touch::LogLevel::Debug;
    }
    return std::nullopt;
}

std::optional<Options> parse_options(int argc, char** argv) {
    Options options;
    for (int index = 1; index < argc; ++index) {
        const std::string argument = argv[index];
        auto require_value = [&](const char* option) -> const char* {
            if (index + 1 >= argc) {
                std::cerr << option << " requires a value\n";
                return nullptr;
            }
            return argv[++index];
        };

        if (argument == "-h" || argument == "--help") {
            usage(std::cout);
            std::exit(0);
        }
        if (argument == "-f" || argument == "--foreground") {
            options.foreground = true;
            continue;
        }
        if (argument == "--test") {
            options.test = true;
            continue;
        }
        if (argument == "--diagnose-sensors") {
            options.diagnose_sensors = true;
            continue;
        }
        if (argument == "-l" || argument == "--log-level") {
            const char* value = require_value(argument.c_str());
            if (value == nullptr) {
                return std::nullopt;
            }
            const auto level = parse_log_level(value);
            if (!level) {
                std::cerr << "Unsupported log level: " << value << "\n";
                return std::nullopt;
            }
            options.log_level = *level;
            continue;
        }
        if (argument.rfind("--log-level=", 0) == 0) {
            const auto level = parse_log_level(argument.substr(12));
            if (!level) {
                std::cerr << "Unsupported log level: " << argument.substr(12) << "\n";
                return std::nullopt;
            }
            options.log_level = *level;
            continue;
        }
        if (argument == "--sensor-seconds") {
            const char* value = require_value(argument.c_str());
            if (value == nullptr) {
                return std::nullopt;
            }
            char* end = nullptr;
            errno = 0;
            const unsigned long long parsed = std::strtoull(value, &end, 10);
            if (errno != 0 || end == value || *end != '\0') {
                std::cerr << "Invalid sensor duration: " << value << "\n";
                return std::nullopt;
            }
            options.sensor_seconds = static_cast<uint64_t>(parsed);
            continue;
        }
        if (argument.rfind("--sensor-seconds=", 0) == 0) {
            const std::string value = argument.substr(17);
            char* end = nullptr;
            errno = 0;
            const unsigned long long parsed = std::strtoull(value.c_str(), &end, 10);
            if (errno != 0 || end == value.c_str() || *end != '\0') {
                std::cerr << "Invalid sensor duration: " << value << "\n";
                return std::nullopt;
            }
            options.sensor_seconds = static_cast<uint64_t>(parsed);
            continue;
        }

        std::cerr << "Unknown argument: " << argument << "\n";
        usage(std::cerr);
        return std::nullopt;
    }
    return options;
}

int run_self_tests() {
    using namespace oplus::touch;

    const TouchConfig config;
    if (!config.enable_aod || !config.enable_dt2w ||
        !config.enable_gestures || !config.enable_raise_to_wake ||
        !config.aod_keepalive.enabled || !config.aod_defaults.enabled ||
        !config.aod_panel_service.enabled) {
        std::cerr << "config defaults test failed\n";
        return 1;
    }
    if (static_cast<uint32_t>(XiaomiTouchMode::AodEnable) != 11 ||
        static_cast<uint32_t>(XiaomiTouchMode::DoubletapMode) != 14 ||
        static_cast<uint32_t>(XiaomiTouchMode::FodEnable) != 10) {
        std::cerr << "xiaomi-touch mode test failed\n";
        return 1;
    }
    if (gesture_ids::DoubleTap != 0x22 || gesture_ids::SwipeUp != 0x24 ||
        panel_feature_value(kPanelFeatureAodCaps) != kPanelAodCapabilities ||
        panel_feature_value(kPanelFeatureUdfpsType) != kPanelUdfpsLocalHbm) {
        std::cerr << "protocol constant test failed\n";
        return 1;
    }
    if (config.aod_defaults.marker != "oplus_touch_hal_aod_defaults_v3") {
        std::cerr << "AOD defaults marker test failed\n";
        return 1;
    }

    const std::filesystem::path node =
        std::filesystem::temp_directory_path() /
        ("oplus-touch-cpp-selftest-" + std::to_string(getpid()));
    {
        std::ofstream output(node, std::ios::trunc);
        output << "0\n";
    }
    ProcfsTouch procfs(node.string(), "/nonexistent");
    if (!procfs.set_gesture_mask(UINT64_MAX) ||
        procfs.read_gesture().value_or("") != "1" ||
        !procfs.set_gesture_mask(0) ||
        procfs.read_gesture().value_or("") != "0") {
        std::filesystem::remove(node);
        std::cerr << "tp_gesture boolean arming test failed\n";
        return 1;
    }
    std::filesystem::remove(node);

    std::cout << "All C++ self-tests passed\n";
    return 0;
}

int run_service(const oplus::touch::TouchConfig& config) {
    using namespace oplus::touch;

    signal(SIGPIPE, SIG_IGN);

    sigset_t shutdown_signals;
    sigemptyset(&shutdown_signals);
    sigaddset(&shutdown_signals, SIGINT);
    sigaddset(&shutdown_signals, SIGTERM);
    if (pthread_sigmask(SIG_BLOCK, &shutdown_signals, nullptr) != 0) {
        log_message(LogLevel::Error,
                    "Could not block shutdown signals for the service");
        return 1;
    }

    if (config.enable_aod && config.aod_panel_service.enabled) {
        if (!start_panel_feature_service(config.aod_panel_service)) {
            log_message(LogLevel::Warn,
                        "Display panel capability service unavailable");
        }
    }

    TouchHal hal(config);
    if (!hal.start_all()) {
        log_message(LogLevel::Error, "Touch HAL startup failed");
        return 1;
    }

    log_message(LogLevel::Info,
                "OPlus hardware touch service is running");
    int signal_number = 0;
    const int result = sigwait(&shutdown_signals, &signal_number);
    if (result != 0) {
        log_message(LogLevel::Warn,
                    "Shutdown signal wait failed: " + errno_string(result));
    } else {
        log_message(LogLevel::Info,
                    "Shutdown signal " + std::to_string(signal_number) +
                        " received, stopping services");
    }

    hal.stop_all();
    log_message(LogLevel::Info, "Hardware touch service stopped gracefully");
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    const auto options = parse_options(argc, argv);
    if (!options) {
        return 2;
    }

    oplus::touch::set_log_level(options->log_level);

    if (options->test) {
        return run_self_tests();
    }
    if (options->diagnose_sensors) {
        return oplus::touch::diagnose_sensors(options->sensor_seconds);
    }

    oplus::touch::log_message(oplus::touch::LogLevel::Info,
                              "OPlus hardware touch service starting");
    oplus::touch::log_message(oplus::touch::LogLevel::Info,
                              "Device: Redmi Note 12 4G (topaz/SM6225)");
    if (!options->foreground) {
        oplus::touch::log_message(
            oplus::touch::LogLevel::Info,
            "Running as a foreground init-managed service");
    }

    return run_service(oplus::touch::default_config());
}
