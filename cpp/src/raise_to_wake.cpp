#include "oplus_touch/raise_to_wake.hpp"

#include "oplus_touch/common.hpp"
#include "oplus_touch/screen_state.hpp"
#include "oplus_touch/uinput.hpp"
#include "oplus_touch/wake.hpp"

#include <algorithm>
#include <array>
#include <chrono>
#include <cstring>
#include <dlfcn.h>
#include <iostream>
#include <optional>
#include <sstream>
#include <vector>

namespace oplus::touch {
namespace {

constexpr int kALooperPrepareAllowNonCallbacks = 1;
constexpr int kALooperPollError = -4;
constexpr size_t kEventBatch = 8;
constexpr int kPollTimeoutMs = 500;
constexpr int kPickupTypeCode = 33171036;

struct ASensorEvent {
    int32_t version;
    int32_t sensor;
    int32_t type;
    int32_t reserved0;
    int64_t timestamp;
    float data[16];
    uint32_t flags;
    int32_t reserved1[3];
};

using ALooperCallbackFunc = int (*)(int, int, void*);

struct AndroidApi {
    using GetInstance = void* (*)(const char*);
    using GetSensorList = int (*)(void*, const void***);
    using GetDefaultSensor = const void* (*)(void*, int32_t, bool);
    using GetType = int32_t (*)(const void*);
    using GetName = const char* (*)(const void*);
    using GetStringType = const char* (*)(const void*);
    using GetHandle = int32_t (*)(const void*);
    using IsWakeup = int32_t (*)(const void*);
    using LooperPrepare = void* (*)(int32_t);
    using CreateEventQueue = void* (*)(void*, void*, int32_t, ALooperCallbackFunc, void*);
    using DestroyEventQueue = void (*)(void*, void*);
    using EnableSensor = int32_t (*)(void*, const void*);
    using DisableSensor = int32_t (*)(void*, const void*);
    using GetEvents = ssize_t (*)(void*, ASensorEvent*, size_t);
    using LooperPollAll = int32_t (*)(int32_t, int32_t*, int32_t*, void**);

    void* handle = nullptr;
    GetInstance get_instance = nullptr;
    GetSensorList get_sensor_list = nullptr;
    GetDefaultSensor get_default_sensor = nullptr;
    GetType get_type = nullptr;
    GetName get_name = nullptr;
    GetStringType get_string_type = nullptr;
    GetHandle get_handle = nullptr;
    IsWakeup is_wakeup_sensor = nullptr;
    LooperPrepare looper_prepare = nullptr;
    CreateEventQueue create_event_queue = nullptr;
    DestroyEventQueue destroy_event_queue = nullptr;
    EnableSensor enable_sensor = nullptr;
    DisableSensor disable_sensor = nullptr;
    GetEvents get_events = nullptr;
    LooperPollAll looper_poll_all = nullptr;

    ~AndroidApi() {
        if (handle != nullptr) {
            dlclose(handle);
        }
    }
};

struct SensorDescriptor {
    const void* sensor = nullptr;
    std::string name;
    std::string string_type;
    int32_t handle = 0;
    int32_t type_code = 0;
    bool wakeup = false;
};

struct PickupSensor {
    const void* sensor = nullptr;
    std::string name;
    std::string string_type;
    int32_t handle = 0;
    int32_t type_code = 0;
    bool wakeup = false;
    const char* source = "";
};

template <typename T>
T load_symbol(void* handle, const char* name) {
    dlerror();
    void* symbol = dlsym(handle, name);
    if (symbol == nullptr) {
        const char* error = dlerror();
        log_message(LogLevel::Warn,
                    std::string("Raise-to-wake: libandroid.so is missing ") + name +
                        (error ? std::string(": ") + error : ""));
        return nullptr;
    }
    return reinterpret_cast<T>(symbol);
}

std::optional<AndroidApi> load_android_api() {
    AndroidApi api;
    api.handle = dlopen("libandroid.so", RTLD_NOW | RTLD_LOCAL);
    if (api.handle == nullptr) {
        const char* error = dlerror();
        log_message(LogLevel::Warn,
                    std::string("Raise-to-wake: libandroid.so load failed: ") +
                        (error ? error : "unknown dlopen error"));
        return std::nullopt;
    }

    api.get_instance =
        load_symbol<AndroidApi::GetInstance>(api.handle,
                                             "ASensorManager_getInstanceForPackage");
    api.get_sensor_list =
        load_symbol<AndroidApi::GetSensorList>(api.handle, "ASensorManager_getSensorList");
    api.get_default_sensor =
        load_symbol<AndroidApi::GetDefaultSensor>(api.handle,
                                                   "ASensorManager_getDefaultSensorEx");
    api.get_type = load_symbol<AndroidApi::GetType>(api.handle, "ASensor_getType");
    api.get_name = load_symbol<AndroidApi::GetName>(api.handle, "ASensor_getName");
    api.get_string_type =
        load_symbol<AndroidApi::GetStringType>(api.handle, "ASensor_getStringType");
    api.get_handle = load_symbol<AndroidApi::GetHandle>(api.handle, "ASensor_getHandle");
    api.is_wakeup_sensor =
        load_symbol<AndroidApi::IsWakeup>(api.handle, "ASensor_isWakeUpSensor");
    api.looper_prepare =
        load_symbol<AndroidApi::LooperPrepare>(api.handle, "ALooper_prepare");
    api.create_event_queue =
        load_symbol<AndroidApi::CreateEventQueue>(api.handle,
                                                  "ASensorManager_createEventQueue");
    api.destroy_event_queue =
        load_symbol<AndroidApi::DestroyEventQueue>(api.handle,
                                                   "ASensorManager_destroyEventQueue");
    api.enable_sensor =
        load_symbol<AndroidApi::EnableSensor>(api.handle,
                                              "ASensorEventQueue_enableSensor");
    api.disable_sensor =
        load_symbol<AndroidApi::DisableSensor>(api.handle,
                                               "ASensorEventQueue_disableSensor");
    api.get_events =
        load_symbol<AndroidApi::GetEvents>(api.handle, "ASensorEventQueue_getEvents");
    api.looper_poll_all =
        load_symbol<AndroidApi::LooperPollAll>(api.handle, "ALooper_pollAll");

    if (api.get_instance == nullptr || api.get_sensor_list == nullptr ||
        api.get_default_sensor == nullptr || api.get_type == nullptr ||
        api.get_name == nullptr || api.get_string_type == nullptr ||
        api.get_handle == nullptr || api.is_wakeup_sensor == nullptr ||
        api.looper_prepare == nullptr || api.create_event_queue == nullptr ||
        api.destroy_event_queue == nullptr || api.enable_sensor == nullptr ||
        api.disable_sensor == nullptr || api.get_events == nullptr ||
        api.looper_poll_all == nullptr) {
        return std::nullopt;
    }
    return api;
}

std::string c_string(const char* value) {
    return value == nullptr ? std::string() : std::string(value);
}

std::vector<SensorDescriptor> enumerate_sensors(AndroidApi& api, void* manager) {
    const void** list = nullptr;
    int count = api.get_sensor_list(manager, &list);
    if (count <= 0 || list == nullptr) {
        return {};
    }

    std::vector<SensorDescriptor> result;
    result.reserve(static_cast<size_t>(count));
    for (int index = 0; index < count; ++index) {
        const void* sensor = list[index];
        if (sensor == nullptr) {
            continue;
        }
        SensorDescriptor descriptor;
        descriptor.sensor = sensor;
        descriptor.name = c_string(api.get_name(sensor));
        descriptor.string_type = c_string(api.get_string_type(sensor));
        descriptor.handle = api.get_handle(sensor);
        descriptor.type_code = api.get_type(sensor);
        descriptor.wakeup = api.is_wakeup_sensor(sensor) != 0;
        result.push_back(std::move(descriptor));
    }
    return result;
}

std::optional<PickupSensor> select_pickup_sensor(
    AndroidApi& api, void* manager, const RaiseToWakeConfig& config,
    const std::vector<SensorDescriptor>& descriptors) {
    std::optional<PickupSensor> exact_non_wakeup;
    std::optional<PickupSensor> exact_wakeup;
    std::optional<PickupSensor> typed_non_wakeup;
    std::optional<PickupSensor> typed_wakeup;

    auto from = [](const SensorDescriptor& descriptor, const char* source) {
        PickupSensor pickup;
        pickup.sensor = descriptor.sensor;
        pickup.name = descriptor.name;
        pickup.string_type = descriptor.string_type;
        pickup.handle = descriptor.handle;
        pickup.type_code = descriptor.type_code;
        pickup.wakeup = descriptor.wakeup;
        pickup.source = source;
        return pickup;
    };

    for (const auto& descriptor : descriptors) {
        const bool exact = descriptor.string_type == config.sensor_type;
        if (!exact && descriptor.type_code != kPickupTypeCode) {
            continue;
        }
        if (exact && descriptor.wakeup && !exact_wakeup) {
            exact_wakeup = from(descriptor, "string-type");
        } else if (exact && !descriptor.wakeup && !exact_non_wakeup) {
            exact_non_wakeup = from(descriptor, "string-type");
        } else if (!exact && descriptor.wakeup && !typed_wakeup) {
            typed_wakeup = from(descriptor, "vendor-type");
        } else if (!exact && !descriptor.wakeup && !typed_non_wakeup) {
            typed_non_wakeup = from(descriptor, "vendor-type");
        }
    }

    std::optional<PickupSensor> selected;
    if (config.prefer_wakeup_sensor) {
        selected = exact_wakeup ? exact_wakeup
                                : (exact_non_wakeup ? exact_non_wakeup
                                                    : (typed_wakeup ? typed_wakeup
                                                                    : typed_non_wakeup));
    } else {
        selected = exact_non_wakeup ? exact_non_wakeup
                                    : (exact_wakeup ? exact_wakeup
                                                    : (typed_non_wakeup ? typed_non_wakeup
                                                                        : typed_wakeup));
    }
    if (selected) {
        return selected;
    }

    const bool first_wakeup = config.prefer_wakeup_sensor;
    for (bool wakeup : {first_wakeup, !first_wakeup}) {
        const void* sensor = api.get_default_sensor(manager, kPickupTypeCode, wakeup);
        if (sensor != nullptr) {
            SensorDescriptor descriptor;
            descriptor.sensor = sensor;
            descriptor.name = c_string(api.get_name(sensor));
            descriptor.string_type = c_string(api.get_string_type(sensor));
            descriptor.handle = api.get_handle(sensor);
            descriptor.type_code = api.get_type(sensor);
            descriptor.wakeup = api.is_wakeup_sensor(sensor) != 0;
            return from(descriptor, "default-sensor");
        }
    }
    return std::nullopt;
}

std::string summarize_sensors(const std::vector<SensorDescriptor>& descriptors) {
    std::ostringstream summary;
    summary << descriptors.size() << " Android sensors";
    int count = 0;
    for (const auto& descriptor : descriptors) {
        const bool candidate =
            descriptor.type_code == kPickupTypeCode ||
            lower_ascii(descriptor.string_type).find("pickup") != std::string::npos ||
            lower_ascii(descriptor.name).find("pickup") != std::string::npos;
        if (!candidate) {
            continue;
        }
        summary << "; name='" << descriptor.name << "' string='" << descriptor.string_type
                << "' type=" << descriptor.type_code << " handle=" << descriptor.handle
                << " wakeup=" << (descriptor.wakeup ? "true" : "false");
        if (++count >= 8) {
            break;
        }
    }
    return summary.str();
}

const char* pickup_value_label(float value) {
    if (value >= 1.0F - 0.000001F && value <= 1.0F + 0.000001F) {
        return "picked up";
    }
    if (value >= 2.0F - 0.000001F && value <= 2.0F + 0.000001F) {
        return "put down";
    }
    return "unknown state";
}

bool setting_enabled(const RaiseToWakeConfig& config, std::string* error) {
    if (!config.require_setting) {
        return true;
    }
    CommandResult result = run_command(
        {"/system/bin/settings", "get", config.setting_namespace, config.setting_key},
        true, true);
    if (!result.started || result.exit_code != 0) {
        if (error != nullptr) {
            *error = result.started
                         ? "settings exited with " + std::to_string(result.exit_code) +
                               ": " + trim(result.error)
                         : "failed to run /system/bin/settings: " + result.error;
        }
        return false;
    }
    const std::string value = trim(result.output);
    if (value == "1" || value == "true") {
        return true;
    }
    if (value == "0" || value == "false" || value == "null" || value.empty()) {
        return false;
    }
    if (error != nullptr) {
        *error = "unexpected value \"" + value + "\"";
    }
    return false;
}

}  // namespace

RaiseToWakeManager::RaiseToWakeManager(RaiseToWakeConfig config,
                                       std::shared_ptr<SysfsManager> sysfs)
    : config_(std::move(config)), sysfs_(std::move(sysfs)) {}

RaiseToWakeManager::~RaiseToWakeManager() {
    stop();
}

bool RaiseToWakeManager::start() {
    if (!config_.enabled) {
        log_message(LogLevel::Info, "Raise to wake is disabled in config");
        return true;
    }
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
        return true;
    }
    (void)sysfs_->write_bool("raise_to_wake_enable", true);
    thread_ = std::thread(&RaiseToWakeManager::run_sensor_loop, this);
    log_message(LogLevel::Info,
                "Raise to wake started through Android sensor " + config_.sensor_type);
    return true;
}

void RaiseToWakeManager::stop() {
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }
    (void)sysfs_->write_bool("raise_to_wake_enable", false);
    if (thread_.joinable()) {
        thread_.join();
    }
    log_message(LogLevel::Info, "Raise to wake stopped");
}

void RaiseToWakeManager::set_event_callback(EventCallback callback) {
    std::lock_guard<std::mutex> lock(callback_mutex_);
    callback_ = std::move(callback);
}

void RaiseToWakeManager::emit_event(float value) {
    RaiseToWakeEvent event;
    event.timestamp = now_ms();
    event.value = value;
    std::lock_guard<std::mutex> lock(callback_mutex_);
    if (callback_) {
        callback_(event);
    }
}

std::optional<bool> RaiseToWakeManager::setting_enabled() const {
    std::string error;
    const bool enabled = oplus::touch::setting_enabled(config_, &error);
    if (!error.empty()) {
        log_message(LogLevel::Debug, "Raise-to-wake setting read failed: " + error);
    }
    return enabled;
}

void RaiseToWakeManager::run_sensor_loop() {
    std::unique_ptr<AndroidApi> api;
    while (running_.load(std::memory_order_acquire) && !api) {
        auto loaded = load_android_api();
        if (loaded) {
            api = std::make_unique<AndroidApi>(std::move(*loaded));
        } else {
            sleep_ms(1000);
        }
    }
    if (!api || !running_.load(std::memory_order_acquire)) {
        return;
    }

    uint64_t last_discovery_warning = now_ms() - 30000;
    while (running_.load(std::memory_order_acquire)) {
        void* manager = api->get_instance(config_.sensor_package.c_str());
        if (manager == nullptr) {
            if (now_ms() - last_discovery_warning >= 30000) {
                log_message(LogLevel::Warn,
                            "Raise-to-wake: waiting for the Android sensor manager");
                last_discovery_warning = now_ms();
            }
            sleep_ms(1000);
            continue;
        }

        const std::vector<SensorDescriptor> descriptors =
            enumerate_sensors(*api, manager);
        const std::optional<PickupSensor> pickup =
            select_pickup_sensor(*api, manager, config_, descriptors);
        if (!pickup) {
            if (now_ms() - last_discovery_warning >= 30000) {
                log_message(LogLevel::Warn,
                            "Raise-to-wake: sensor '" + config_.sensor_type +
                                "' / type " + std::to_string(kPickupTypeCode) +
                                " is not published yet; retrying (" +
                                summarize_sensors(descriptors) + ")");
                last_discovery_warning = now_ms();
            }
            sleep_ms(1000);
            continue;
        }

        log_message(LogLevel::Info,
                    "Raise-to-wake selected name='" + pickup->name + "' string='" +
                        pickup->string_type + "' type=" +
                        std::to_string(pickup->type_code) + " handle=" +
                        std::to_string(pickup->handle) + " wakeup=" +
                        (pickup->wakeup ? "true" : "false") + " source=" +
                        pickup->source);

        void* looper = api->looper_prepare(kALooperPrepareAllowNonCallbacks);
        if (looper == nullptr) {
            log_message(LogLevel::Warn,
                        "Raise-to-wake: ALooper_prepare failed; retrying");
            sleep_ms(1000);
            continue;
        }
        void* queue = api->create_event_queue(manager, looper, 1, nullptr, nullptr);
        if (queue == nullptr) {
            log_message(LogLevel::Warn,
                        "Raise-to-wake: sensor event queue creation failed; retrying");
            sleep_ms(1000);
            continue;
        }

        bool sensor_enabled_now = false;
        uint64_t enabled_at = now_ms();
        uint64_t last_setting_check = 0;
        bool setting_value = false;
        std::optional<bool> last_setting_state;
        uint64_t last_setting_error = now_ms() - 30000;
        std::optional<ScreenState> last_screen_state;
        std::optional<float> last_value;
        uint64_t last_trigger = now_ms() - 10000;
        uint64_t last_arm_attempt = now_ms() - 5000;
        std::optional<WakeInjector> fallback_injector;

        log_message(LogLevel::Info, "Raise-to-wake sensor HAL loop is ready");

        while (running_.load(std::memory_order_acquire)) {
            if (now_ms() - last_setting_check >= 2000) {
                last_setting_check = now_ms();
                auto enabled = setting_enabled();
                if (enabled) {
                    setting_value = *enabled;
                    if (last_setting_state != *enabled) {
                        if (*enabled || !config_.require_setting) {
                            log_message(
                                LogLevel::Info,
                                "Raise-to-wake setting " + config_.setting_namespace +
                                    "." + config_.setting_key + " is enabled");
                        } else {
                            log_message(
                                LogLevel::Warn,
                                "Raise-to-wake is disabled by secure setting " +
                                    config_.setting_namespace + "." +
                                    config_.setting_key + "; enable it in Settings");
                        }
                        last_setting_state = *enabled;
                    }
                } else {
                    setting_value = false;
                    if (now_ms() - last_setting_error >= 30000) {
                        log_message(
                            LogLevel::Warn,
                            "Raise-to-wake could not read secure setting " +
                                config_.setting_namespace + "." +
                                config_.setting_key);
                        last_setting_error = now_ms();
                    }
                }
            }

            const ScreenState screen = read_screen_state();
            if (last_screen_state != screen) {
                log_message(LogLevel::Debug,
                            std::string("Raise-to-wake screen state changed to ") +
                                screen_state_name(screen));
                last_screen_state = screen;
            }
            const bool should_enable =
                screen_is_off_or_doze(screen) &&
                (setting_value || !config_.require_setting);
            if (should_enable != sensor_enabled_now &&
                now_ms() - last_arm_attempt >= 5000) {
                const int32_t result =
                    should_enable ? api->enable_sensor(queue, pickup->sensor)
                                  : api->disable_sensor(queue, pickup->sensor);
                last_arm_attempt = now_ms();
                if (result == 0) {
                    sensor_enabled_now = should_enable;
                    enabled_at = now_ms();
                    last_value.reset();
                    log_message(LogLevel::Info,
                                sensor_enabled_now
                                    ? "Raise-to-wake sensor armed"
                                    : "Raise-to-wake sensor disarmed");
                } else {
                    log_message(LogLevel::Warn,
                                std::string("Raise-to-wake sensor ") +
                                    (should_enable ? "enable" : "disable") +
                                    " failed: " + std::to_string(result));
                }
            }

            const int32_t poll =
                api->looper_poll_all(kPollTimeoutMs, nullptr, nullptr, nullptr);
            if (poll == kALooperPollError) {
                log_message(LogLevel::Warn,
                            "Raise-to-wake sensor poll failed; rebuilding the queue");
                break;
            }
            if (!sensor_enabled_now) {
                continue;
            }
            if (poll >= 0) {
                hold_wake_lock("oplus_hardware_touch_service_raise", 1000);
            }

            std::array<ASensorEvent, kEventBatch> events{};
            ssize_t count = 0;
            while ((count = api->get_events(queue, events.data(), events.size())) > 0) {
                for (ssize_t index = 0; index < count; ++index) {
                    const ASensorEvent& event = events[static_cast<size_t>(index)];
                    if (event.type != pickup->type_code) {
                        continue;
                    }
                    const float value = event.data[0];
                    if (!last_value || *last_value != value) {
                        log_message(LogLevel::Info,
                                    "Raise-to-wake sensor value " +
                                        std::to_string(value) + " (" +
                                        pickup_value_label(value) + ")");
                        last_value = value;
                    }
                    if (value < 1.0F - 0.000001F ||
                        value > 1.0F + 0.000001F) {
                        continue;
                    }
                    if (now_ms() - enabled_at < config_.arm_grace_ms) {
                        log_message(
                            LogLevel::Debug,
                            "Ignoring pickup state during sensor arm grace");
                        continue;
                    }
                    if (now_ms() - last_trigger < config_.debounce_ms) {
                        continue;
                    }
                    last_trigger = now_ms();
                    emit_event(value);
                    log_message(LogLevel::Info,
                                "Raise to wake triggered by pickup sensor");

                    std::string error;
                    if (!request_power_wakeup(&error)) {
                        log_message(LogLevel::Warn,
                                    "Raise-to-wake power request failed: " +
                                        error);
                        if (!fallback_injector) {
                            auto injector =
                                WakeInjector::open("oplus-raise-to-wake");
                            if (injector.valid()) {
                                fallback_injector = std::move(injector);
                            }
                        }
                        if (fallback_injector &&
                            fallback_injector->emit_wake()) {
                            log_message(
                                LogLevel::Info,
                                "Raise-to-wake uinput fallback injected");
                        } else {
                            log_message(
                                LogLevel::Warn,
                                "Raise-to-wake could not run cmd power wakeup "
                                "and uinput is unavailable");
                        }
                    }
                }
            }
        }

        if (sensor_enabled_now) {
            (void)api->disable_sensor(queue, pickup->sensor);
        }
        api->destroy_event_queue(manager, queue);
        if (running_.load(std::memory_order_acquire)) {
            sleep_ms(500);
        }
    }
    log_message(LogLevel::Info, "Raise-to-wake sensor loop stopped");
}

int diagnose_sensors(uint64_t duration_seconds) {
    duration_seconds = std::clamp<uint64_t>(duration_seconds, 5, 600);
    auto api = load_android_api();
    if (!api) {
        std::cerr << "Android sensor NDK API is unavailable\n";
        return 1;
    }

    RaiseToWakeConfig config;
    void* manager = api->get_instance(config.sensor_package.c_str());
    if (manager == nullptr) {
        std::cerr << "ASensorManager_getInstanceForPackage returned null\n";
        return 1;
    }

    std::cout << "Sensor package: " << config.sensor_package
              << "; screen state: " << screen_state_name(read_screen_state()) << "\n";
    std::string setting_error;
    std::cout << "Setting " << config.setting_namespace << "." << config.setting_key
              << "=" << (setting_enabled(config, &setting_error) ? "enabled" : "disabled")
              << (setting_error.empty() ? "" : " (" + setting_error + ")") << "\n";

    auto descriptors = enumerate_sensors(*api, manager);
    std::cout << "Android sensors (" << descriptors.size() << "):\n";
    for (const auto& sensor : descriptors) {
        std::cout << "  name='" << sensor.name << "' string='" << sensor.string_type
                  << "' type=" << sensor.type_code << " handle=" << sensor.handle
                  << " wakeup=" << (sensor.wakeup ? "true" : "false") << "\n";
    }

    auto pickup = select_pickup_sensor(*api, manager, config, descriptors);
    if (!pickup) {
        std::cerr << "no pickup sensor matched '" << config.sensor_type << "' or type "
                  << kPickupTypeCode << " (" << summarize_sensors(descriptors) << ")\n";
        return 1;
    }
    std::cout << "Selected name='" << pickup->name << "' string='" << pickup->string_type
              << "' type=" << pickup->type_code << " handle=" << pickup->handle
              << " wakeup=" << (pickup->wakeup ? "true" : "false")
              << " source=" << pickup->source << "\n";

    void* looper = api->looper_prepare(kALooperPrepareAllowNonCallbacks);
    if (looper == nullptr) {
        std::cerr << "ALooper_prepare failed\n";
        return 1;
    }
    void* queue = api->create_event_queue(manager, looper, 1, nullptr, nullptr);
    if (queue == nullptr) {
        std::cerr << "sensor event queue creation failed\n";
        return 1;
    }
    if (api->enable_sensor(queue, pickup->sensor) != 0) {
        std::cerr << "ASensorEventQueue_enableSensor failed\n";
        api->destroy_event_queue(manager, queue);
        return 1;
    }

    hold_wake_lock("oplus_touch_sensor_probe", (duration_seconds + 5) * 1000);
    std::cout << "Listening for " << duration_seconds
              << "s. Keep the screen off, leave the phone still, then lift it.\n";
    const uint64_t started = now_ms();
    size_t event_count = 0;
    while (now_ms() - started < duration_seconds * 1000) {
        int32_t poll = api->looper_poll_all(kPollTimeoutMs, nullptr, nullptr, nullptr);
        if (poll == kALooperPollError) {
            std::cout << "Sensor poll error\n";
            break;
        }
        if (poll >= 0) {
            hold_wake_lock("oplus_touch_sensor_probe", 1000);
        }
        std::array<ASensorEvent, kEventBatch> events{};
        ssize_t count = 0;
        while ((count = api->get_events(queue, events.data(), events.size())) > 0) {
            for (ssize_t index = 0; index < count; ++index) {
                const auto& event = events[static_cast<size_t>(index)];
                ++event_count;
                std::cout << "event type=" << event.type << " value=" << event.data[0]
                          << " label=" << pickup_value_label(event.data[0]) << " raw=["
                          << event.data[0] << ", " << event.data[1] << ", " << event.data[2]
                          << ", " << event.data[3] << "]\n";
            }
        }
    }

    api->disable_sensor(queue, pickup->sensor);
    api->destroy_event_queue(manager, queue);
    release_wake_lock("oplus_touch_sensor_probe");
    std::cout << "Sensor probe complete: " << event_count << " event(s)\n";
    return event_count == 0 ? 1 : 0;
}

}  // namespace oplus::touch
