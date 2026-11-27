cmake_minimum_required(VERSION 3.20)

project(oplus_hardware_touch_service_cpp
    VERSION 1.0.0
    DESCRIPTION "C++20 OPlus touch HAL for topaz/SM6225"
    LANGUAGES CXX)

option(OPLUS_TOUCH_BACKLIGHT
    "Enable the FTS/backlight gesture backend" OFF)
option(OPLUS_TOUCH_BUILD_TESTS
    "Build host-side logic tests" ON)
option(OPLUS_TOUCH_ANDROID
    "Cross-compile for Android/bionic" OFF)

if(DEFINED ANDROID_API_LEVEL)
    set(OPLUS_TOUCH_ANDROID_API "${ANDROID_API_LEVEL}")
else()
    set(OPLUS_TOUCH_ANDROID_API "30")
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT OPLUS_TOUCH_ANDROID)
    find_package(Threads REQUIRED)
endif()

set(OPLUS_TOUCH_CORE_SOURCES
    src/aod.cpp
    src/aod_defaults.cpp
    src/common.cpp
    src/config.cpp
    src/doze.cpp
    src/evdev.cpp
    src/gesture.cpp
    src/hal.cpp
    src/panel_feature.cpp
    src/procfs.cpp
    src/raise_to_wake.cpp
    src/screen_state.cpp
    src/sysfs.cpp
    src/uinput.cpp
    src/wake.cpp
    src/xiaomi_touch.cpp)

add_library(oplus_touch_core STATIC ${OPLUS_TOUCH_CORE_SOURCES})
target_include_directories(oplus_touch_core PUBLIC include)
target_link_libraries(oplus_touch_core PUBLIC ${CMAKE_DL_LIBS})
if(NOT OPLUS_TOUCH_ANDROID)
    target_link_libraries(oplus_touch_core PUBLIC Threads::Threads)
endif()
target_compile_options(oplus_touch_core PRIVATE
    -Wall
    -Wextra
    -Wpedantic)

if(OPLUS_TOUCH_BACKLIGHT)
    target_compile_definitions(oplus_touch_core PUBLIC OPLUS_GESTURE_BACKLIGHT=1)
endif()

if(OPLUS_TOUCH_ANDROID)
    target_link_libraries(oplus_touch_core PUBLIC log)
    target_compile_options(oplus_touch_core PUBLIC
        --target=aarch64-linux-android${OPLUS_TOUCH_ANDROID_API}
        -fPIE)
    target_link_options(oplus_touch_core PUBLIC
        --target=aarch64-linux-android${OPLUS_TOUCH_ANDROID_API}
        -fPIE
        -pie
        -static-libstdc++
        -Wl,-z,max-page-size=65536)
endif()

add_executable(oplus_hardware_touch_service src/main.cpp)
set_target_properties(oplus_hardware_touch_service PROPERTIES
    OUTPUT_NAME "vendor.oplus.hardware.touch-V2-service"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin")
target_link_libraries(oplus_hardware_touch_service PRIVATE oplus_touch_core)

if(OPLUS_TOUCH_BUILD_TESTS AND NOT OPLUS_TOUCH_ANDROID)
    enable_testing()
    add_executable(oplus_touch_cpp_tests tests/test_logic.cpp)
    target_link_libraries(oplus_touch_cpp_tests PRIVATE oplus_touch_core)
    add_test(NAME oplus_touch_cpp_logic COMMAND oplus_touch_cpp_tests)
endif()

install(TARGETS oplus_hardware_touch_service
    RUNTIME DESTINATION bin)
