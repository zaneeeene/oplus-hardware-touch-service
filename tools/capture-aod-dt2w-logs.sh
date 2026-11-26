#!/system/bin/sh
# Capture the device state needed to diagnose AOD, DT2W, and panel flicker.
#
# Run entirely on the Android device:
#
#   su -c 'sh /sdcard/Download/capture-aod-dt2w-logs.sh'
#
# The script relaunches itself through su when started as a normal app/shell
# user. It prompts for each interaction so the logs stay aligned with the
# failing sequence.
set -eu
umask 022

shell_quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

if [ "$(id -u 2>/dev/null || printf '1')" -ne 0 ]; then
    SCRIPT="$0"
    case "$SCRIPT" in
        /*) ;;
        *) SCRIPT="$PWD/$SCRIPT" ;;
    esac

    COMMAND="sh $(shell_quote "$SCRIPT")"
    for ARG in "$@"; do
        COMMAND="$COMMAND $(shell_quote "$ARG")"
    done

    printf '%s\n' "Root access is required; requesting su..."
    exec su -c "$COMMAND"
fi

STAMP="$(date -u +%Y%m%dT%H%M%SZ 2>/dev/null || date +%Y%m%dT%H%M%S)"
DEFAULT_OUT="/sdcard/Download/aod-dt2w-logs-$STAMP"
OUT="${1:-$DEFAULT_OUT}"
case "$OUT" in
    /*) ;;
    *) OUT="$PWD/$OUT" ;;
esac

if ! mkdir -p "$OUT" 2>/dev/null; then
    if [ "$OUT" = "$DEFAULT_OUT" ]; then
        OUT="/data/local/tmp/aod-dt2w-logs-$STAMP"
        mkdir -p "$OUT"
        printf '%s\n' "Could not write to /sdcard/Download; using $OUT instead."
    else
        printf 'Could not create output directory: %s\n' "$OUT" >&2
        exit 1
    fi
fi

SESSION_PIDS=""
WATCHDOG_PIDS=""

cleanup() {
    trap - 0 HUP INT TERM 2>/dev/null || true
    for PID in ${SESSION_PIDS:-}; do
        kill "$PID" 2>/dev/null || true
    done
    for PID in ${WATCHDOG_PIDS:-}; do
        kill "$PID" 2>/dev/null || true
    done
    for PID in ${SESSION_PIDS:-} ${WATCHDOG_PIDS:-}; do
        wait "$PID" 2>/dev/null || true
    done
    SESSION_PIDS=""
    WATCHDOG_PIDS=""
}
trap cleanup EXIT HUP INT TERM

capture_shell() {
    NAME="$1"
    shift
    {
        printf '$'
        printf ' %s' "$@"
        printf '\n'
        "$@" 2>&1 || true
    } >"$OUT/$NAME"
}

capture_root_file() {
    SOURCE="$1"
    NAME="$2"
    ERROR_FILE="$OUT/$NAME.error"
    if cat "$SOURCE" >"$OUT/$NAME" 2>"$ERROR_FILE"; then
        rm -f "$ERROR_FILE"
    else
        {
            printf 'Could not read %s\n\n' "$SOURCE"
            cat "$ERROR_FILE" 2>/dev/null || true
        } >"$OUT/$NAME"
        rm -f "$ERROR_FILE"
    fi
}

capture_setting() {
    NAMESPACE="$1"
    KEY="$2"
    printf '%s.%s=' "$NAMESPACE" "$KEY"
    settings get "$NAMESPACE" "$KEY" 2>/dev/null || true
}

start_bounded_capture() {
    NAME="$1"
    shift
    "$@" >"$OUT/$NAME" 2>&1 &
    CAPTURE_PID=$!
    SESSION_PIDS="$SESSION_PIDS $CAPTURE_PID"

    # The interaction prompts normally stop this first. The watchdog puts a
    # hard 15-minute cap on verbose streams if the terminal is left unattended.
    (
        sleep 900
        kill "$CAPTURE_PID" 2>/dev/null || true
    ) &
    WATCHDOG_PIDS="$WATCHDOG_PIDS $!"
}

printf 'Collecting static device state into %s\n' "$OUT"
{
    getprop
} >"$OUT/properties.txt" 2>&1 || true

capture_shell service-status.txt sh -c '
    printf "init.svc.oplus-touch-service="
    getprop init.svc.oplus-touch-service
    printf "vendor.touch.hal.enable="
    getprop vendor.touch.hal.enable
    pidof vendor.oplus.hardware.touch-V2-service || true
    ps -A | grep -E "vendor\.oplus\.hardware\.touch-V2-service|oplus-touch-service|system_server" || true
'
capture_shell input-devices.txt cat /proc/bus/input/devices
capture_shell input-capabilities.txt getevent -lp
capture_shell tp-gesture.txt cat /proc/tp_gesture
capture_shell xiaomi-touch-info.txt sh -c '
    ls -lZ /dev/xiaomi-touch /proc/xiaomi-touch 2>&1
    stat /dev/xiaomi-touch 2>&1 || true
    cat /proc/xiaomi-touch 2>&1 || true
'
capture_shell display-nodes.txt sh -c '
    for f in \
        /sys/class/backlight/panel0-backlight/brightness \
        /sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/doze_brightness \
        /sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/mipi_rw \
        /sys/class/display/panel0/panel_state
    do
        echo "== $f"
        cat "$f" 2>&1 || true
    done
'

capture_root_file /data/system/users/0/settings_system.xml settings_system.xml
capture_root_file /data/system/users/0/settings_secure.xml settings_secure.xml
capture_root_file /data/system/users/0/settings_global.xml settings_global.xml

{
    printf '%s\n\n' "Settings values relevant to DT2W, AOD, and raise-to-wake"
    for NAMESPACE in system secure global; do
        for KEY in \
            oplus_screen_off_gesture_state \
            oplus_customize_gesture_double_touch_on \
            oplus_customize_gesture_double_touch \
            oplus_customize_gesture_double_tap_on \
            oplus_customize_gesture_double_tap \
            oplus_customize_gesture_double_touch_off \
            oplus_customize_double_tap_wake_enable \
            oplus_double_tap_wake_enable \
            double_tap_wake_enable \
            dt2w_enable \
            oplus_customize_gesture_wake_up_arouse \
            disable_gestures \
            Setting_AodEnable \
            Setting_AodSwitchEnable \
            Setting_AodUserSetTime \
            Setting_Sub_AodFullScreen \
            oplus_keyguard_panoramic_aod_support \
            oplus_keyguard_panoramic_aod_all_day_support \
            oplus_keyguard_workshop_aod_support \
            aod_media_support \
            full_screen_aod \
            panoramic_aod_enable \
            panoramic_aod_rate
        do
            capture_setting "$NAMESPACE" "$KEY"
        done
    done
} >"$OUT/settings-values.txt" 2>&1

printf '%s\n' "Starting bounded logcat, evdev, and panel captures (15 minutes)."
logcat -c >/dev/null 2>&1 || true
start_bounded_capture session-logcat.txt logcat -v threadtime
start_bounded_capture session-evdev.txt getevent -lt
start_bounded_capture session-panel-samples.txt sh -c '
    while true; do
        printf "%s screen=" "$(date +%s)"
        getprop debug.tracing.screen_state
        printf " backlight="
        cat /sys/class/backlight/panel0-backlight/brightness 2>/dev/null || true
        printf " doze="
        cat /sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/doze_brightness 2>/dev/null || true
        printf " tp="
        cat /proc/tp_gesture 2>/dev/null || true
        sleep 0.2
    done
'

step() {
    printf '\n=== %s ===\n' "$1"
    printf '%s' "Press Enter after completing this step... "
    IFS= read -r _ || true
}

cat <<'EOF'

Follow these steps on the phone. Do not reboot during the capture.
The most useful sequence is exactly the failing sequence you see.
EOF

step "1. Wake the phone, unlock it, and leave it on the home screen"
step "2. Press power to lock it and wait until the AOD/clock is visible"
step "3. Single-tap the AOD twice (the one-tap preview test)"
step "4. Double-tap immediately after the first AOD frame appears"
step "5. Turn the screen back off, then spam double-tap several times"
step "6. Leave full-screen AOD visible for at least several minutes"

printf '\n%s\n' "Stopping background capture..."
cleanup

capture_shell dmesg-tail.txt sh -c 'dmesg | tail -n 4000'
capture_shell screen-state.txt sh -c '
    printf "screen_state="
    getprop debug.tracing.screen_state
    printf "backlight="
    cat /sys/class/backlight/panel0-backlight/brightness 2>/dev/null || true
    printf "doze_brightness="
    cat /sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/doze_brightness 2>/dev/null || true
'
capture_shell touch-node.txt sh -c '
    printf "tp_gesture="
    cat /proc/tp_gesture 2>/dev/null || true
    printf "xiaomi_touch="
    ls -l /dev/xiaomi-touch /proc/xiaomi-touch 2>/dev/null || true
'

grep -E 'avc: denied|oplus[-_]hardware[-_]touch|vendor\.oplus\.hardware\.touch|oplus-touch-service|DT2W|Settings|AOD|doze|backlight' \
    "$OUT/session-logcat.txt" >"$OUT/session-highlights.txt" 2>/dev/null || true
grep -E 'avc: denied|oplus[-_]hardware[-_]touch|vendor\.oplus\.hardware\.touch|oplus-touch-service|DT2W|AOD|doze|backlight|panel' \
    "$OUT/dmesg-tail.txt" >"$OUT/dmesg-highlights.txt" 2>/dev/null || true

DEVICE="$(getprop ro.product.device 2>/dev/null | tr -d '\r')"
API="$(getprop ro.build.version.sdk 2>/dev/null | tr -d '\r')"
cat >"$OUT/README.txt" <<EOF
Captured: $STAMP
Device: $DEVICE
API: $API
Output: $OUT

Files:
  properties.txt             Full getprop output
  service-status.txt         Init/service and process state
  settings_*.xml             SettingsProvider XML snapshots
  settings-values.txt        Relevant values in system/secure/global
  tp-gesture.txt             Current /proc/tp_gesture value
  xiaomi-touch-info.txt      Device metadata and proc contents
  display-nodes.txt          Backlight/doze/MIPI readback
  session-logcat.txt         Bounded logcat during all six interaction steps
  session-evdev.txt          Bounded getevent input stream
  session-panel-samples.txt  5 Hz screen/backlight/doze/tp_gesture samples
  session-highlights.txt     HAL/AOD/DT2W/AVC lines from logcat
  dmesg-highlights.txt       Relevant kernel lines from the final dmesg tail
  dmesg-tail.txt             Last 4000 kernel messages

If the daemon is not running, include service-status.txt and dmesg-tail.txt.
If DT2W selection looks wrong, include settings_*.xml and session-logcat.txt.
If this was written under /data/local/tmp, copy it to shared storage as root.
EOF

printf '\n%s\n' "Capture complete: $OUT"
ls -lh "$OUT"
