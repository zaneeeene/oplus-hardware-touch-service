LOCAL_PATH := $(call my-dir)

include $(CLEAR_VARS)
LOCAL_MODULE := oplus-hardware-touch-service
LOCAL_MODULE_OWNER := oplus
LOCAL_SRC_FILES := vendor/bin/vendor.oplus.hardware.touch-V2-service
LOCAL_MODULE_CLASS := EXECUTABLES
LOCAL_MODULE_PATH := $(TARGET_OUT_VENDOR)/bin
LOCAL_MODULE_STEM := vendor.oplus.hardware.touch-V2-service
LOCAL_STRIP_MODULE := false
LOCAL_CHECK_ELF_FILES := false
LOCAL_MODULE_TAGS := optional
include $(BUILD_PREBUILT)

include $(CLEAR_VARS)
LOCAL_MODULE := oplus-hardware-touch-service-rc
LOCAL_MODULE_OWNER := oplus
LOCAL_SRC_FILES := vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc
LOCAL_MODULE_CLASS := ETC
LOCAL_MODULE_PATH := $(TARGET_OUT_VENDOR)/etc/init
LOCAL_MODULE_STEM := vendor.oplus.hardware.touch-V2-service.rc
LOCAL_MODULE_TAGS := optional
include $(BUILD_PREBUILT)
