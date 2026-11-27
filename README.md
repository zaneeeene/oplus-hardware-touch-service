# OPlus Hardware Touch Service for topaz

`oplus-hardware-touch-service` is a userspace HAL for the Redmi Note 12 4G
(`topaz`, SM6225). It replaces the broken userspace `xiaomi-touch` helper with
one Android/bionic executable and keeps all runtime behavior in the daemon and
init service. No companion shell script or user-installed helper is required.

The repository contains two independent implementations:

- the original Rust implementation in `src/`;
- a standalone C++20 translation in `cpp/` with no Rust FFI or wrapper.

Both implement the same touch, gesture, AOD, display-panel, and
raise-to-wake behavior. They are built separately, produce the same installed
executable name, and must not be installed at the same time.

The installed HAL executable uses the required name
`vendor.oplus.hardware.touch-V2-service`. Its init service is
`oplus-touch-service`; the shorter service name keeps the generated
`init.svc.*` property within Android's property-name limit.

The daemon does not modify or build a kernel. MiCode `topaz-t-oss` was used
only as an ABI reference; no other device kernel tree is touched.

## Features

- Arms kernel DT2W through the boolean `/proc/tp_gesture` interface.
- Uses the `/dev/xiaomi-touch` `_IO('T', command)` misc ABI when present.
- Wakes screen-off gestures through `/system/bin/cmd power wakeup`, matching
  the working raise-to-wake injection instead of relying on a raw
  `KEY_WAKEUP`. The request is checked after 250 ms and retried once if the
  panel is still off/dozing; uinput remains a command-failure fallback.
- Provides a selectable `backlight` gesture backend for panels that need FTS
  touchscreen discovery, a direct `panel0-backlight` screen-off check, and the
  `/system/bin/input keyevent 134` AOD-preview path. The default `stable`
  backend is unchanged.
- Suppresses the DT2S immediate-rewake race: a tap sequence that starts while
  the display is awake cannot inject a wake key after the framework turns the
  display off.
- Supports single-tap AOD preview through Linux `KEY_F4` (Android keycode
  `134`), plus the touch driver's native `KEY_GOTO` signal.
- Keeps classic AOD/static-image/analog-clock styles visible through the
  `mi_display` doze level and MIPI DCS brightness commands.
- Follows the full-screen AOD fade when the panel exposes it, falls back to the
  classic AOD path as soon as the fade collapses, and performs at most one
  settled DBV fallback when DCS readback is unavailable.
- Serves `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature`
  feature `217` as `15` (panoramic, all-day, smooth transition, doze-direct)
  and feature `211` as `16` (local HBM UDFPS). This is what lets OPlus SystemUI
  keep full-screen/panoramic AOD enabled instead of using its temporary
  energy-saving path.
- Applies the OPlus full-screen/all-day AOD secure defaults once, then
  re-applies the every-boot AOD media capability flag. The current one-time
  marker is `oplus_touch_hal_aod_defaults_v3`; older builds with a stale marker
  automatically apply the corrected profile once.
- Implements raise-to-wake through Android sensor HAL type
  `xiaomi.sensor.pickup`, matching the working
  `oplus-hardware-raise2wake` binary. It uses `/system/bin/cmd power wakeup`
  with a uinput fallback and does not read an accelerometer evdev node.

## Build outputs

The build produces:

```text
target/aarch64-linux-android/release/oplus-hardware-touch-service
build/out/vendor.oplus.hardware.touch-V2-service
build/out/oplus-hardware-touch-service-vendor.zip
build/out/oplus-hardware-touch-service-SHA256SUMS
```

The optional backlight gesture backend is written to a separate directory so
it cannot overwrite the default build:

```text
build/out/backlight-gesture/vendor.oplus.hardware.touch-V2-service
build/out/backlight-gesture/oplus-hardware-touch-service-backlight-gesture-vendor.zip
build/out/backlight-gesture/oplus-hardware-touch-service-backlight-gesture-SHA256SUMS
```

Inside the release ZIP, files are arranged as:

```text
README.md
vendor/bin/vendor.oplus.hardware.touch-V2-service
vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc
```

The runtime configuration is compiled into the executable. The ZIP contains
only the installable vendor payload and this README; it has no config file,
installer metadata, helper script, or separate policy directory.

The executable is ELF64 AArch64 PIE, uses `/system/bin/linker64`, targets
Android API 30, and uses 64 KiB ELF load alignment. It uses only stable NDK
ABI symbols and has no dependency on the NDK clang compiler driver at runtime.

The target is an AArch64 build. The host may be AArch64 Linux; the supplied
`tools/android-linker.sh` uses Rust's native `rust-lld` because the NDK's
x86_64 clang driver cannot execute on that host.

The standalone C++ port builds into separate directories and does not replace
or remove any Rust artifact:

```text
build/cpp/out/vendor.oplus.hardware.touch-V2-service
build/cpp/out/oplus-hardware-touch-service-cpp-vendor.zip
build/cpp/out/oplus-hardware-touch-service-cpp-SHA256SUMS

build/cpp/out/backlight-gesture/vendor.oplus.hardware.touch-V2-service
build/cpp/out/backlight-gesture/oplus-hardware-touch-service-cpp-backlight-gesture-vendor.zip
build/cpp/out/backlight-gesture/oplus-hardware-touch-service-cpp-backlight-gesture-SHA256SUMS
```

The C++ source, architecture, and build details are documented in
[`cpp/README.md`](cpp/README.md).

## Build

```bash
cd oplus-hardware-touch-service
export ANDROID_NDK_HOME=/path/to/android-ndk-r26d
./build.sh
```

Build the optional backlight gesture backend instead:

```bash
./build.sh --gesture-variant backlight
```

The equivalent environment form is:

```bash
GESTURE_VARIANT=backlight ./build.sh
```

`./build.sh --help` lists both variants. The choice maps to the
`gesture-backlight` Cargo feature, which changes only the touchscreen name
list, screen-off fallback, and AOD-preview command path.

The script checks formatting, runs all tests, cross-builds the Android target,
verifies the ELF interpreter and page alignment, packages the HAL binary,
init service, and README, tests the ZIP, and writes fresh checksums. Generated
files stay under `build/` and `target/`.

GitHub Actions runs the host checks and the full Android build. Both AArch64
Linux hosts and conventional x86_64 GitHub runners are supported.

### C++ build

```bash
cd oplus-hardware-touch-service
export ANDROID_NDK_HOME=/path/to/android-ndk-r26d
./cpp-build.sh --variant both
```

Build only one backend when required:

```bash
./cpp-build.sh --variant stable
./cpp-build.sh --variant backlight
```

`cpp-build.sh` locates the NDK and a host-executable Clang automatically, or
accepts `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, `CPP_CLANG`, and
`CPP_CLANGXX`. It supports an AArch64 Linux host as well as a conventional
x86_64 host. The script runs the host logic tests and executable self-test,
cross-builds both requested variants, verifies the Android ELF properties,
packages the three-file vendor ZIP, and writes SHA-256 checksums.

The C++ stable and backlight backends intentionally use separate output
directories. Selecting `both` does not overwrite the stable binary or ZIP.

## Repository layout

```text
etc/init/               Android init service
kernel/                 Optional kernel-interface reference stubs
sepolicy/               SELinux policy and file/service contexts
src/                    Rust daemon, compiled-in defaults, and HAL implementation
cpp/                    Standalone C++20 HAL translation and host tests
tools/                  Linker and on-device diagnostic tools
build.sh                Format, test, cross-build, package, and checksum
cpp-build.sh            C++ host-test, cross-build, verify, and package script
```

Compiled binaries, ZIP archives, `target/`, `build/`, and the generated
vendor tree are ignored by Git.

## Vendor installation

The ZIP is an install-only vendor payload. Its complete layout is:

```text
README.md
vendor/bin/vendor.oplus.hardware.touch-V2-service
vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc
```

Install the Rust ZIP or one C++ ZIP, never both. Every build uses the same
installed binary and init-service names, so merging two payloads would replace
one implementation with the other.

Merge the ZIP's `vendor/` directory into the vendor image. The final device
paths must be:

```text
/vendor/bin/vendor.oplus.hardware.touch-V2-service        root:root 0755
/vendor/etc/init/vendor.oplus.hardware.touch-V2-service.rc root:root 0644
```

For a quick test on a rooted device with a writable vendor partition:

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

The init `.rc` must be present before boot so Android init parses the service.
Pushing the files into a running system is not enough by itself.

The AOSP prebuilt recipe is included in the source repository as `Android.mk`.

### Required init label

The included init service already contains:

```text
service oplus-touch-service /vendor/bin/vendor.oplus.hardware.touch-V2-service --foreground
    class main
    user root
    group root system input
    capabilities SYS_RESOURCE
    seclabel u:r:oplus_hardware_touch_service:s0
    disabled
```

The `seclabel` line requires the `oplus_hardware_touch_service` SELinux
domain. Do not change the service to `u:r:su:s0`; use the vendor domain below.

### Required file_contexts

Add this entry to the device's vendor file contexts:

```text
/vendor/bin/vendor.oplus.hardware.touch-V2-service u:object_r:oplus_hardware_touch_service_exec:s0
```

In a ROM source tree, put it in the device vendor policy directory, normally
as a file such as `sepolicy/vendor/file_contexts`. After flashing the rebuilt
vendor image, the executable should report:

```bash
ls -Z /vendor/bin/vendor.oplus.hardware.touch-V2-service
# u:object_r:oplus_hardware_touch_service_exec:s0
```

### Required service_contexts

The display-panel AIDL service must be visible to SystemUI and
`system_server`, otherwise panoramic/full-screen AOD falls back to the
temporary energy-saving path. Add:

```text
vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default u:object_r:oplus_hardware_touch_service_panel_service:s0
```

This is a vendor service context, normally placed in
`vendor/etc/selinux/vendor_service_contexts` in a ROM build.

### Required SELinux policy

Merge this policy into the device vendor policy. It assumes the standard
topaz labels listed below; adapt the device-label rules if your ROM uses
different existing labels.

```te
type oplus_hardware_touch_service, domain;
type oplus_hardware_touch_service_exec, exec_type, vendor_file_type, file_type;
type oplus_hardware_touch_service_panel_service, service_manager_type, hal_service_type, protected_service;

init_daemon_domain(oplus_hardware_touch_service)

# /dev/xiaomi-touch and /proc/tp_gesture
allow oplus_hardware_touch_service touchfeature_device:chr_file rw_file_perms;
allow oplus_hardware_touch_service proc_touchpad:file rw_file_perms;

# Touchscreen evdev and uinput wake/preview injection. AOSP labels
# /dev/uinput as uhid_device; uinput_device is not a topaz policy type.
allow oplus_hardware_touch_service input_device:chr_file rw_file_perms;
allow oplus_hardware_touch_service uhid_device:chr_file rw_file_perms;

# Xiaomi mi_display doze and DCS brightness interfaces.
allow oplus_hardware_touch_service graphics_device:chr_file rw_file_perms;
allow oplus_hardware_touch_service vendor_sysfs_graphics:dir r_dir_perms;
allow oplus_hardware_touch_service vendor_sysfs_graphics:file rw_file_perms;
allow oplus_hardware_touch_service vendor_sysfs_graphics:lnk_file r_file_perms;

# Optional /sys/class/touch compatibility nodes.
allow oplus_hardware_touch_service vendor_sysfs_touch:dir r_dir_perms;
allow oplus_hardware_touch_service vendor_sysfs_touch:file rw_file_perms;
allow oplus_hardware_touch_service vendor_sysfs_touch:lnk_file r_file_perms;

# Raise-to-wake wake lock and logging.
allow oplus_hardware_touch_service sysfs_wake_lock:file rw_file_perms;
allow oplus_hardware_touch_service self:unix_stream_socket create_stream_socket_perms;
allow oplus_hardware_touch_service logd:unix_stream_socket connectto;
allow oplus_hardware_touch_service logd_socket:sock_file write;

# Screen state, boot state and vendor properties.
get_prop(oplus_hardware_touch_service, { debug_prop system_prop vendor_default_prop })

# Binder service registration and discovery by SystemUI/system_server.
allow oplus_hardware_touch_service servicemanager:binder call;
allow oplus_hardware_touch_service oplus_hardware_touch_service_panel_service:service_manager { add find };
allow platform_app oplus_hardware_touch_service_panel_service:service_manager find;
allow system_app oplus_hardware_touch_service_panel_service:service_manager find;
allow priv_app oplus_hardware_touch_service_panel_service:service_manager find;
allow system_server oplus_hardware_touch_service_panel_service:service_manager find;
allow platform_app oplus_hardware_touch_service:binder { call transfer };
allow system_app oplus_hardware_touch_service:binder { call transfer };
allow priv_app oplus_hardware_touch_service:binder { call transfer };
allow system_server oplus_hardware_touch_service:binder { call transfer };
allow oplus_hardware_touch_service platform_app:binder transfer;
allow oplus_hardware_touch_service system_app:binder transfer;
allow oplus_hardware_touch_service priv_app:binder transfer;
allow oplus_hardware_touch_service system_server:binder transfer;
allow platform_app oplus_hardware_touch_service:fd use;
allow system_app oplus_hardware_touch_service:fd use;
allow priv_app oplus_hardware_touch_service:fd use;
allow system_server oplus_hardware_touch_service:fd use;

# Android sensor-manager client for xiaomi.sensor.pickup.
allow oplus_hardware_touch_service sensorservice_service:service_manager find;
allow oplus_hardware_touch_service sensorservice_service:binder { call transfer };
allow oplus_hardware_touch_service system_server:binder { call transfer };
allow oplus_hardware_touch_service system_server:fd use;
allow system_server oplus_hardware_touch_service:binder transfer;
allow system_server oplus_hardware_touch_service:fd use;

# /system/bin/cmd for power wakeup, settings defaults and input injection.
allow oplus_hardware_touch_service system_file:file { execute execute_no_trans getattr map open read };
allow oplus_hardware_touch_service settings_service:service_manager find;
allow oplus_hardware_touch_service settings_service:binder { call transfer };
allow oplus_hardware_touch_service input_service:service_manager find;
allow oplus_hardware_touch_service input_service:binder { call transfer };
allow oplus_hardware_touch_service power_service:service_manager find;
allow oplus_hardware_touch_service power_service:binder { call transfer };
allow oplus_hardware_touch_service activity_service:service_manager find;
allow oplus_hardware_touch_service activity_service:binder { call transfer };
```

Add the vendor policy directory containing these files to
`BOARD_VENDOR_SEPOLICY_DIRS` in the device tree, then rebuild the vendor image
and SELinux policy. If your ROM does not use split vendor policy directories,
append these rules to the equivalent existing vendor policy files instead.

The policy intentionally reuses existing device labels:

```text
/dev/xiaomi-touch       -> touchfeature_device
input event nodes       -> input_device
/dev/uinput             -> uhid_device
/proc/tp_gesture        -> proc_touchpad
mi_display nodes        -> graphics_device / vendor_sysfs_graphics
touch sysfs nodes       -> vendor_sysfs_touch
```

Do not relabel these shared devices globally. If a label is missing on another
ROM, add a narrow rule for that existing type rather than changing the device
node's label.

### Permissions

The HAL must run as `root` with groups `root system input` and the
`SYS_RESOURCE` capability, as already declared in the included `.rc` file. It
needs access to:

```text
/dev/xiaomi-touch
/proc/tp_gesture
/dev/input/touchscreen0 or the discovered touchscreen event node
/dev/uinput
/dev/mi_display/disp_feature
/sys/devices/virtual/mi_display/.../doze_brightness
/sys/devices/virtual/mi_display/.../mipi_rw
/sys/class/backlight/panel0-backlight/brightness
/sys/class/touch/* (optional compatibility nodes)
/sys/power/wake_lock
/system/bin/cmd
```

The service registers
`vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default` on
Binder and uses `cmd power wakeup`, `cmd input keyevent`, and
`cmd settings put` for gesture, AOD-preview, and AOD-default behavior.

### Verify installation

```bash
getprop init.svc.oplus-touch-service
ps -A | grep vendor.oplus.hardware.touch-V2-service
logcat -s oplus-touch-service
dmesg | grep -E 'avc:.*oplus_hardware_touch_service'
```

The service is controlled with the property documented under
[Configuration](#configuration).

## Configuration

There is no runtime `config.json`. Rust defaults are defined in
`src/config.rs`; C++ defaults are defined in
`cpp/include/oplus_touch/config.hpp`. Both are compiled directly into the HAL
executable, so the service does not create or read anything under
`/vendor/etc/oplus-hardware-touch-service` and works with a read-only vendor
image. Change the relevant defaults and rebuild to produce a different
profile.

The init service is deliberately long-lived because it owns evdev and uinput
fds. It starts on `late-fs` and can be controlled with:

```bash
setprop vendor.touch.hal.enable 1
setprop vendor.touch.hal.enable 0
```

## Touch interfaces

### `/proc/tp_gesture`

On the Novatek/Xiaomi touch driver this is a boolean arming node:

```text
write 1 -> arm gesture engine (including DT2W)
write 0 -> disarm gesture engine
```

It is not a 64-bit gesture bitmap. The daemon preserves the boolean ABI and
the self-test checks it.

### `/dev/xiaomi-touch`

The misc device ABI is:

```text
_IO('T', 0) = SetCurValue
_IO('T', 7) = SetLongValue
```

The argument is an `int[256]`; `buf[0]` is the mode and `buf[1]` is the value.
If the device is absent, startup continues and procfs/evdev/sysfs paths remain
usable.

### Input, DT2W, and single-tap preview

The daemon discovers the touchscreen from `/proc/bus/input/devices` with a
config override. A userspace double tap is emitted only after two valid
releases within 350 ms and only when the tap sequence began while the display
was off or dozing.

Kernel `KEY_WAKEUP`, `KEY_POWER`, and `KEY_F4` reports trigger the gesture, but
are not treated as proof that Android resumed. OPlus power policy can accept
the input event while leaving the display off, so the daemon explicitly asks
the power service to wake. A second request is made after 250 ms only if the
display is still off or dozing.

Single-tap AOD preview defaults to gesture ID `35` (`action: aod_preview`,
`wake_screen: false`). It injects Android keycode `134` without waking the
device and is debounced to avoid duplicate previews.

### DT2W arming

DT2W is armed directly from `enable_dt2w` and
`gestures.double_tap.enabled`; the daemon does not depend on an OPlus
SettingsProvider key. At startup it programs xiaomi-touch and writes the
boolean `/proc/tp_gesture` node to `1`. Setting `enable_dt2w` to `false`
in the compiled-in defaults removes the double-tap gesture and disarms the
proc node.

The previous live Settings/XML selector was removed because ports expose
different or missing gesture keys, and an unmatched key silently forced DT2W
off even when the gesture itself was enabled. The current HAL does not read
any Settings/XML DT2W key.

## AOD behavior

The daemon owns the `mi_display` doze keeper. When `debug.tracing.screen_state`
enters doze or doze-suspend:

- classic clocks/static images use doze level `1` and DBV `808`;
- full-screen AOD follows the panel fade where possible;
- if DCS readback is unavailable, the keeper still writes the desired AOD DBV
  instead of leaving the panel nearly black, with at most one settled fallback
  write per AOD entry;
- if the low framework fallback makes DCS read back the same value as the
  normal backlight node, the settled classic-AOD path still restores DBV
  `808`; this is the case which previously left AOD dim indefinitely;
- a full-screen fade that collapses below backlight register `40` switches to
  the classic path at the next dense DBV check rather than waiting for 300 ms,
  avoiding the black frame during the ON -> AOD transition;
- classic AOD checks DBV once at 300 ms, avoiding repeated brightness
  corrections while entering or leaving AOD;
- the panel doze level is re-applied if it is cleared during an extended AOD
  session;
- waking to the lockscreen sends `DOZE_OFF` immediately and rechecks it at
  300/800 ms, reapplying only if the AOD level survived or its node is absent.

The panel capability service is registered before the touch worker starts.
It returns a stable AIDL response in this order:

```text
status header + return 0 + int[] values
```

Unknown features and methods return an unknown transaction, matching the
reference service's fallback behavior. The service intentionally does not call
`AIBinder_markVintfStability`: the reference build does not define
`BINDER_STABILITY_SUPPORT`, and marking a local service VINTF-stable would
require a matching VINTF manifest entry.

### AOD defaults

The one-time secure settings are applied after `sys.boot_completed=1` and a
short delay. They are guarded by
`oplus_touch_hal_aod_defaults_v3`, so settings changed later in the UI are not
overwritten. The every-boot `aod_media_support=1` flag is applied again after
SystemUI starts.

The defaults are written with `/system/bin/cmd settings` and require the
service domain to be allowed to execute that command. If the ROM's policy
rejects the command, inspect `logcat -s oplus-touch-service` and merge the
corresponding `system_file` and service-manager permissions from the supplied
policy.

## Raise to wake

Raise-to-wake is enabled only when the OPlus setting is enabled:

```bash
settings put secure oplus_customize_gesture_wake_up_arouse 1
```

The daemon subscribes to `xiaomi.sensor.pickup`, arms the sensor while the
screen is off/dozing, waits for the pickup state `1.0`, and requests wake with
`/system/bin/cmd power wakeup`. A one-second wake lock is held only after a
real sensor event so the AP cannot suspend before the wake request is
delivered. The sensor queue is rebuilt after poll errors.

To disable the setting again:

```bash
settings put secure oplus_customize_gesture_wake_up_arouse 0
```

## Diagnostic capture

`tools/capture-aod-dt2w-logs.sh` collects the properties, Settings XML,
DT2W/raise-to-wake values, `/proc/tp_gesture`, input capabilities, panel
backlight/doze nodes, SELinux denials, bounded logcat, and bounded evdev
events. It then prompts for the screen-on, AOD, preview, immediate DT2W,
spammed DT2W, long full-screen AOD, and raise-to-wake probe steps:

```bash
su -c 'sh /sdcard/Download/capture-aod-dt2w-logs.sh /sdcard/Download/aod-dt2w-logs'
```

The script runs entirely on the device and needs root only through `su`. If it
is started as a normal shell or app user, it relaunches itself through `su`
automatically. Root is required for the Settings XML snapshots, panel nodes,
and kernel log. The default output directory is
`/sdcard/Download/aod-dt2w-logs-<UTC timestamp>`; if that location cannot be
created, it falls back to `/data/local/tmp`. Both logcat and evdev capture stop
when the seven-step sequence finishes, with a hard 15-minute limit.

The sensor-manager package identity is `vendor.oplus.hardware.raise2wake`,
matching the working reference binary's `ASensorManager_getInstanceForPackage`
path. The non-wakeup pickup sensor is selected first by default.

If raise-to-wake does not arm, run the on-device sensor probe as root:

```bash
su -c '/vendor/bin/vendor.oplus.hardware.touch-V2-service \
  --diagnose-sensors --sensor-seconds 30'
```

It prints every Android sensor with its name, string type, numeric type,
handle, and wake-up flag, then subscribes to the selected pickup sensor for 30
seconds. Keep the screen off, leave the phone still, then lift it. The probe
temporarily holds a wake lock so AP suspend cannot hide the event stream; the
normal HAL arms only while the screen is off/dozing and does not hold a
continuous lock. The HAL also falls back from the exact
`xiaomi.sensor.pickup` string type to vendor type `33171036` and
`ASensorManager_getDefaultSensorEx` for ported ROMs that rename the sensor.

## API 35+

Both binaries target API 30 and use only stable NDK/libc ABI symbols, so they
are expected to run on API 35+ vendor images. API 35 compatibility is not a
guarantee until verified on the target device with its actual SELinux policy,
service-manager behavior, panel driver, and OPlus SystemUI. Both ZIPs are
built with 64 KiB ELF load alignment to remain compatible with newer page
sizes.

## On-device verification

```bash
getprop init.svc.oplus-touch-service
logcat -s oplus-touch-service
cat /proc/tp_gesture
ls -lZ /dev/xiaomi-touch /dev/uinput /dev/mi_display/disp_feature
cat /proc/bus/input/devices
service list | grep displaypanelfeature
```

For a host-side self-test:

```bash
cargo run -- --test
./build/cpp/host/bin/vendor.oplus.hardware.touch-V2-service --test
```

The Rust self-test and the C++ self-test both check boolean
`/proc/tp_gesture` arming and do not claim that the node accepts a gesture
bitmap.

Hardware verdicts still require the target device. In particular, verify:

1. the panel service is listed after boot;
2. SystemUI keeps panoramic/full-screen AOD active beyond its old timeout;
3. classic AOD/static clocks are visible at DBV 808;
4. a DT2S tap sequence does not immediately re-wake;
5. single-tap preview works while the screen is off;
6. pickup wake works with the secure setting enabled.

## Sources and acknowledgements

This project was developed using the following public references and
device-behavior observations:

- [hoshikv/MiOplusFingerprint-rs commit `fa36acc`](https://github.com/hoshikv/MiOplusFingerprint-rs/commit/fa36acc074b1f29df5d2a6d3aa9cb7d031ffde3a)
  and [commit `6d15f27`](https://github.com/hoshikv/MiOplusFingerprint-rs/commit/6d15f27cb0a43f635c9f392b093fd53911d2a343)
  for AOD, gesture, and wake behavior references.
- [Xia-s-Projekt/oplus-touch-hal](https://github.com/Xia-s-Projekt/oplus-touch-hal)
  for OPlus touch-service structure, AIDL behavior, and gesture-watcher design
  references.
- [nischay12089/oplus-sensor-bridge](https://github.com/nischay12089/oplus-sensor-bridge)
  for sensor bridge, screen transition, flicker, and one-tap-preview behavior
  references.
- [MiCode/Xiaomi_Kernel_OpenSource branch `topaz-t-oss`](https://github.com/MiCode/Xiaomi_Kernel_OpenSource/tree/topaz-t-oss)
  for read-only topaz kernel ABI references covering `xiaomi-touch`,
  `mi_display`, touch input, and gesture interfaces.
- [oppo-source/android_kernel_oppo_sm6225](https://github.com/oppo-source/android_kernel_oppo_sm6225)
  for read-only SM6225-compatible touch and display interface references.
- [kquieter-debug/dt2w_rust](https://github.com/kquieter-debug/dt2w_rust)
  for investigating dynamic DT2W Settings/XML behavior. The live Settings
  selector was later removed from this implementation.
- The locally supplied `oplus-hardware-raise2wake` binary was used as a
  behavioral reference for pickup-sensor selection and Android power-wake
  injection. It is not redistributed in this repository.

No unrelated device-kernel source or configuration was used or modified.
Kernel trees were consulted only for the SM6225/topaz interfaces relevant to
this userspace service.

## License

GPL-2.0-only. See [LICENSE](LICENSE).
