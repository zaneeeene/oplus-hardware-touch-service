# C++20 touch HAL

This directory contains a standalone C++20 translation of
`oplus-hardware-touch-service`. It does not link to, wrap, or invoke the Rust
library. The installed executable keeps the required name:

```text
vendor.oplus.hardware.touch-V2-service
```

The C++ implementation follows the current Rust behavior and uses the same
init service, SELinux domain, Binder service name, and vendor ZIP layout.
Install either the Rust or C++ build, never both at once.

## Architecture

The daemon is one executable with long-lived workers:

- `main.cpp` parses options, installs shutdown handling, starts the panel
  Binder service, and owns the `TouchHal` lifecycle.
- `hal.cpp` coordinates the gesture, AOD, and raise-to-wake managers.
- `gesture.cpp` discovers the touchscreen evdev node, classifies taps,
  arms `/proc/tp_gesture`, dispatches AOD preview, and requests DT2W wake.
- `doze.cpp` owns the `mi_display` doze-level ioctl and MIPI DCS brightness
  stabilization.
- `aod_defaults.cpp` applies the one-time and every-boot OPlus AOD settings
  through `/system/bin/cmd settings`.
- `panel_feature.cpp` registers the stable AIDL
  `IDisplayPanelFeature` service through `libbinder_ndk.so`.
- `raise_to_wake.cpp` subscribes to the Android
  `xiaomi.sensor.pickup` sensor through `libandroid.so`.
- `xiaomi_touch.cpp`, `procfs.cpp`, `sysfs.cpp`, `evdev.cpp`, and
  `uinput.cpp` contain the kernel and input ABIs.

No runtime configuration file is used. Defaults are compiled from
`include/oplus_touch/config.hpp`.

## Requirements

- Android NDK r26 or r27
- CMake 3.20 or newer
- A host-executable Clang/Clang++ with the NDK sysroot
- `zip`, `unzip`, `file`, and `readelf`

The build script supports both AArch64 Linux and x86_64 Linux hosts. On an
AArch64 host, the NDK's x86_64 compiler driver may not execute; the script
automatically finds a host Clang instead.

## Build

From the repository root:

```bash
export ANDROID_NDK_HOME=/path/to/android-ndk-r26d
./cpp-build.sh --variant both
```

Build one gesture backend:

```bash
./cpp-build.sh --variant stable
./cpp-build.sh --variant backlight
```

Environment overrides:

```text
ANDROID_NDK_HOME or ANDROID_NDK_ROOT   NDK location
CPP_CLANG                              host C compiler
CPP_CLANGXX                            host C++ compiler
ANDROID_API_LEVEL                      target API, default 30
CPP_BUILD_DIR                          build root, default build/cpp
CPP_OUT_DIR                            output root, default build/cpp/out
```

The script performs a host build first, runs the C++ logic tests and the
executable self-test, cross-compiles the requested Android variants, checks
the ELF type, interpreter, load alignment, and shared-library dependencies,
then creates the vendor ZIP and SHA-256 checksums.

The C++ build does not modify `target/` or the Rust output under `build/out/`.

## Stable and backlight variants

The common behavior is identical. The variants differ only in touchscreen
discovery and AOD-preview dispatch:

| Behavior | `stable` | `backlight` |
| --- | --- | --- |
| Common touchscreen names | Yes | Yes |
| `fts`/FocalTech name discovery | No | Yes |
| `panel0-backlight` fallback when tracing says ON | No | Yes |
| AOD preview command | `cmd input keyevent 134` | `/system/bin/input keyevent 134` |

Use `stable` unless the target panel needs the FTS/backlight fallback.

## Outputs

Stable:

```text
build/cpp/out/vendor.oplus.hardware.touch-V2-service
build/cpp/out/oplus-hardware-touch-service-cpp-vendor.zip
build/cpp/out/oplus-hardware-touch-service-cpp-SHA256SUMS
```

Backlight:

```text
build/cpp/out/backlight-gesture/vendor.oplus.hardware.touch-V2-service
build/cpp/out/backlight-gesture/oplus-hardware-touch-service-cpp-backlight-gesture-vendor.zip
build/cpp/out/backlight-gesture/oplus-hardware-touch-service-cpp-backlight-gesture-SHA256SUMS
```

Each vendor ZIP contains exactly:

```text
README.md
vendor/bin/vendor.oplus.hardware.touch-V2-service
vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc
```

The executable is an ELF64 AArch64 PIE for `/system/bin/linker64`, targets
Android API 30, uses 64 KiB ELF load alignment, and statically links the C++
runtime. It does not require `libc++_shared.so`.

## Vendor installation

Merge one ZIP's `vendor/` directory into the vendor image:

```text
/vendor/bin/vendor.oplus.hardware.touch-V2-service        root:root 0755
/vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc root:root 0644
```

The init `.rc` must exist before Android boots. For a rooted test device with
a writable vendor partition:

```bash
adb root
adb remount
adb push vendor/bin/vendor.oplus.hardware.touch-V2-service /vendor/bin/
adb push vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc /vendor/etc/init/
adb shell chown root:root /vendor/bin/vendor.oplus.hardware.touch-V2-service
adb shell chmod 0755 /vendor/bin/vendor.oplus.hardware.touch-V2-service
adb shell chown root:root /vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc
adb shell chmod 0644 /vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc
adb reboot
```

The included init service uses:

```text
seclabel u:r:oplus_hardware_touch_service:s0
```

The complete SELinux policy, `file_contexts`, `service_contexts`, required
permissions, and service-manager rules are in the repository root
[`README.md`](../README.md#vendor-installation). Use those rules for both the
Rust and C++ builds.

## Runtime checks

```bash
getprop init.svc.oplus-touch-service
cat /proc/tp_gesture
service list | grep displaypanelfeature
logcat -s oplus-touch-service
```

Expected startup log messages include:

```text
Registered display panel capability service vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default
AOD doze keeper started
Raise-to-wake selected name='...' string='xiaomi.sensor.pickup' ...
All touch services started
```

Check the service process and SELinux denials:

```bash
ps -A | grep vendor.oplus.hardware.touch-V2-service
dmesg | grep -E 'avc:.*oplus_hardware_touch_service'
```

The service is controlled by `vendor.touch.hal.enable`:

```bash
setprop vendor.touch.hal.enable 1
setprop vendor.touch.hal.enable 0
```

## Tests and diagnostics

Host logic tests:

```bash
./build/cpp/host/oplus_touch_cpp_tests
./build/cpp/host/bin/vendor.oplus.hardware.touch-V2-service --test
```

On-device sensor probe:

```bash
su -c '/vendor/bin/vendor.oplus.hardware.touch-V2-service \
  --diagnose-sensors --sensor-seconds 30'
```

Use the repository-root `tools/capture-aod-dt2w-logs.sh` for AOD, DT2W,
preview, brightness, and raise-to-wake captures.

## API compatibility

The binary targets API 30 and uses stable NDK/libc ABIs. It is expected to run
on API 35+ vendor images, but panel drivers, SELinux policy, servicemanager,
sensor HALs, and OPlus SystemUI behavior remain device-specific. On-device
verification is still required before treating a ROM build as supported.

## Verification status

Host tests and both AArch64 cross-builds are automated by `cpp-build.sh`.
Hardware behavior cannot be verified by a host build. In particular, verify
DT2W, single-tap AOD preview, classic/static AOD brightness, panoramic
full-screen all-day AOD, panel feature discovery, and raise-to-wake on the
actual Redmi Note 12 4G vendor image.
