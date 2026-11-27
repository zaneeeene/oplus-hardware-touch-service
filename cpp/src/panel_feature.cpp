#include "oplus_touch/panel_feature.hpp"

#include "oplus_touch/common.hpp"

#include <atomic>
#include <cerrno>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <dlfcn.h>
#include <new>
#include <thread>
#include <vector>

namespace oplus::touch {
namespace {

#ifdef __ANDROID__
constexpr const char* kPanelDescriptor =
    "vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature";
constexpr int32_t kStatusOk = 0;
constexpr int32_t kStatusAlreadyExists = -EEXIST;
constexpr int32_t kStatusInvalidOperation = -ENOSYS;
constexpr int32_t kStatusUnknownTransaction = -EBADMSG;
constexpr int32_t kMaxPanelValues = 64;

using AIBinder = void;
using AIBinderClass = void;
using AStatus = void;
using AParcel = void;

using OnCreate = void* (*)(void*);
using OnDestroy = void (*)(void*);
using OnTransact = int32_t (*)(AIBinder*, uint32_t, const AParcel*, AParcel*);
using ClassDefine = AIBinderClass* (*)(const char*, OnCreate, OnDestroy, OnTransact);
using BinderNew = AIBinder* (*)(const AIBinderClass*, void*);
using AddService = int32_t (*)(AIBinder*, const char*);
using SetThreadCount = void (*)(uint32_t);
using StartThreadPool = void (*)();
using ParcelReadInt32 = int32_t (*)(const AParcel*, int32_t*);
using ParcelWriteInt32 = int32_t (*)(AParcel*, int32_t);
using ParcelWriteInt32Array = int32_t (*)(AParcel*, const int32_t*, int32_t);
using ParcelWriteString = int32_t (*)(AParcel*, const char*, int32_t);
using WriteStatusHeader = int32_t (*)(AParcel*, const AStatus*);
using NewOkStatus = AStatus* (*)();
using DeleteStatus = void (*)(AStatus*);

struct BinderApi {
    void* handle = nullptr;
    ClassDefine class_define = nullptr;
    BinderNew binder_new = nullptr;
    AddService add_service = nullptr;
    SetThreadCount set_thread_count = nullptr;
    StartThreadPool start_thread_pool = nullptr;
    ParcelReadInt32 parcel_read_int32 = nullptr;
    ParcelWriteInt32 parcel_write_int32 = nullptr;
    ParcelWriteInt32Array parcel_write_int32_array = nullptr;
    ParcelWriteString parcel_write_string = nullptr;
    WriteStatusHeader write_status_header = nullptr;
    NewOkStatus new_ok_status = nullptr;
    DeleteStatus delete_status = nullptr;
};

BinderApi g_api;
std::atomic<AIBinderClass*> g_panel_class{nullptr};
std::atomic<AIBinder*> g_panel_binder{nullptr};

template <typename T>
T load_binder_symbol(void* handle, const char* name) {
    dlerror();
    void* symbol = dlsym(handle, name);
    if (symbol == nullptr) {
        const char* error = dlerror();
        log_message(LogLevel::Warn,
                    std::string("Display panel service requires missing binder symbol ") + name +
                        (error == nullptr ? std::string() : std::string(": ") + error));
        return nullptr;
    }
    return reinterpret_cast<T>(symbol);
}

bool load_binder_api() {
    if (g_api.handle != nullptr) {
        return true;
    }

    void* handle = dlopen("libbinder_ndk.so", RTLD_NOW | RTLD_LOCAL);
    if (handle == nullptr) {
        const char* error = dlerror();
        log_message(LogLevel::Warn,
                    std::string("Display panel service could not load libbinder_ndk.so") +
                        (error == nullptr ? std::string() : std::string(": ") + error));
        return false;
    }

    BinderApi api;
    api.handle = handle;
    api.class_define =
        load_binder_symbol<ClassDefine>(handle, "AIBinder_Class_define");
    api.binder_new = load_binder_symbol<BinderNew>(handle, "AIBinder_new");
    api.add_service =
        load_binder_symbol<AddService>(handle, "AServiceManager_addService");
    api.set_thread_count = load_binder_symbol<SetThreadCount>(
        handle, "ABinderProcess_setThreadPoolMaxThreadCount");
    api.start_thread_pool =
        load_binder_symbol<StartThreadPool>(handle, "ABinderProcess_startThreadPool");
    api.parcel_read_int32 =
        load_binder_symbol<ParcelReadInt32>(handle, "AParcel_readInt32");
    api.parcel_write_int32 =
        load_binder_symbol<ParcelWriteInt32>(handle, "AParcel_writeInt32");
    api.parcel_write_int32_array = load_binder_symbol<ParcelWriteInt32Array>(
        handle, "AParcel_writeInt32Array");
    api.parcel_write_string =
        load_binder_symbol<ParcelWriteString>(handle, "AParcel_writeString");
    api.write_status_header =
        load_binder_symbol<WriteStatusHeader>(handle, "AParcel_writeStatusHeader");
    api.new_ok_status = load_binder_symbol<NewOkStatus>(handle, "AStatus_newOk");
    api.delete_status = load_binder_symbol<DeleteStatus>(handle, "AStatus_delete");

    if (api.class_define == nullptr || api.binder_new == nullptr ||
        api.add_service == nullptr || api.set_thread_count == nullptr ||
        api.start_thread_pool == nullptr || api.parcel_read_int32 == nullptr ||
        api.parcel_write_int32 == nullptr ||
        api.parcel_write_int32_array == nullptr ||
        api.parcel_write_string == nullptr || api.write_status_header == nullptr ||
        api.new_ok_status == nullptr || api.delete_status == nullptr) {
        dlclose(handle);
        return false;
    }

    // Registration owns these facilities for the process lifetime.
    g_api = api;
    return true;
}

int32_t write_ok_header(AParcel* output) {
    AStatus* status = g_api.new_ok_status();
    if (status == nullptr) {
        return kStatusInvalidOperation;
    }
    const int32_t result = g_api.write_status_header(output, status);
    g_api.delete_status(status);
    return result == kStatusOk ? kStatusOk : result;
}

int32_t write_status_and_int(AParcel* output, int32_t value) {
    const int32_t status = write_ok_header(output);
    if (status != kStatusOk) {
        return status;
    }
    return g_api.parcel_write_int32(output, value);
}

int32_t write_feature_value_response(AParcel* output,
                                     const std::vector<int32_t>& values) {
    const int32_t status = write_ok_header(output);
    if (status != kStatusOk) {
        return status;
    }
    // Stable AIDL writes the method return value before the out array.
    const int32_t result = g_api.parcel_write_int32(output, 0);
    if (result != kStatusOk) {
        return result;
    }
    return g_api.parcel_write_int32_array(output, values.data(),
                                          static_cast<int32_t>(values.size()));
}

int32_t get_feature_value(const AParcel* input, AParcel* output) {
    int32_t feature = 0;
    if (g_api.parcel_read_int32(input, &feature) != kStatusOk) {
        return kStatusUnknownTransaction;
    }
    const auto value = panel_feature_value(feature);
    if (!value) {
        return kStatusUnknownTransaction;
    }

    int32_t length = 0;
    if (g_api.parcel_read_int32(input, &length) != kStatusOk || length < 1 ||
        length > kMaxPanelValues) {
        return kStatusUnknownTransaction;
    }

    std::vector<int32_t> values(static_cast<size_t>(length), 0);
    values[0] = *value;
    return write_feature_value_response(output, values);
}

int32_t set_feature_value(const AParcel* input, AParcel* output) {
    int32_t feature = 0;
    if (g_api.parcel_read_int32(input, &feature) != kStatusOk ||
        feature != kPanelFeatureAodCaps) {
        return kStatusUnknownTransaction;
    }
    return write_status_and_int(output, 0);
}

void* panel_on_create(void*) {
    return new (std::nothrow) uint8_t{0};
}

void panel_on_destroy(void* user_data) {
    delete static_cast<uint8_t*>(user_data);
}

int32_t panel_on_transact(AIBinder*, uint32_t code, const AParcel* input,
                          AParcel* output) {
    if (g_api.class_define == nullptr) {
        return kStatusUnknownTransaction;
    }

    switch (code) {
        case kPanelTransactionGetFeatureValue:
            return get_feature_value(input, output);
        case kPanelTransactionSetFeatureValue:
            return set_feature_value(input, output);
        case kPanelTransactionGetInfo:
            return kStatusUnknownTransaction;
        case kPanelTransactionGetInterfaceVersion:
            return write_status_and_int(output, kPanelAidlVersion);
        case kPanelTransactionGetInterfaceHash: {
            const int32_t status = write_ok_header(output);
            if (status != kStatusOk) {
                return status;
            }
            return g_api.parcel_write_string(output, kPanelAidlHash,
                                             static_cast<int32_t>(
                                                 std::strlen(kPanelAidlHash)));
        }
        default:
            return kStatusUnknownTransaction;
    }
}

void register_panel_service(AodPanelServiceConfig config) {
    if (!load_binder_api()) {
        return;
    }

    AIBinderClass* binder_class =
        g_api.class_define(kPanelDescriptor, panel_on_create, panel_on_destroy,
                           panel_on_transact);
    if (binder_class == nullptr) {
        log_message(LogLevel::Warn,
                    "Could not define the display panel AIDL binder class");
        return;
    }
    AIBinder* binder = g_api.binder_new(binder_class, nullptr);
    if (binder == nullptr) {
        log_message(LogLevel::Warn,
                    "Could not create the display panel binder service");
        return;
    }

    // Keep the class and binder alive for the daemon lifetime. This service is
    // deliberately not marked VINTF-stable, matching the reference build.
    g_panel_class.store(binder_class, std::memory_order_release);
    g_panel_binder.store(binder, std::memory_order_release);

    g_api.set_thread_count(2);
    g_api.start_thread_pool();

    for (int attempt = 0; attempt < 30; ++attempt) {
        const int32_t status = g_api.add_service(binder, config.service_name.c_str());
        if (status == kStatusOk) {
            log_message(LogLevel::Info,
                        "Registered display panel capability service " +
                            config.service_name);
            return;
        }
        if (status == kStatusAlreadyExists) {
            log_message(LogLevel::Info,
                        "Display panel capability service " + config.service_name +
                            " is already registered");
            return;
        }
        if (attempt == 29) {
            log_message(LogLevel::Warn,
                        "Could not register display panel service " +
                            config.service_name + ": status " +
                            std::to_string(status));
            return;
        }
        std::this_thread::sleep_for(std::chrono::seconds(1));
    }
}

#endif
}  // namespace

std::optional<int32_t> panel_feature_value(int32_t feature) {
    switch (feature) {
        case kPanelFeatureAodCaps:
            return kPanelAodCapabilities;
        case kPanelFeatureUdfpsType:
            return kPanelUdfpsLocalHbm;
        default:
            return std::nullopt;
    }
}

bool start_panel_feature_service(const AodPanelServiceConfig& config) {
    if (!config.enabled) {
        log_message(LogLevel::Info,
                    "Display panel capability service is disabled in config");
        return true;
    }
#ifdef __ANDROID__
    std::thread(register_panel_service, config).detach();
    return true;
#else
    log_message(LogLevel::Warn,
                "Display panel binder service is Android-only");
    return false;
#endif
}

}  // namespace oplus::touch
