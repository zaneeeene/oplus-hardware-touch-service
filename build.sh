#!/usr/bin/env bash
# Build, verify, and package the Android/bionic AArch64 touch service.
set -euo pipefail
umask 022

ROOT="$(cd "$(dirname "$0")" && pwd)"
TARGET="aarch64-linux-android"
API="${ANDROID_API_LEVEL:-30}"
PROJECT_NAME="oplus-hardware-touch-service"
HAL_NAME="vendor.oplus.hardware.touch-V2-service"
GESTURE_VARIANT="${GESTURE_VARIANT:-stable}"

usage() {
    cat <<'EOF'
Usage: ./build.sh [--gesture-variant stable|backlight]

Gesture backends:
  stable     Default touchscreen discovery and AOD preview path.
  backlight  Shipped root-gesture variant: adds FTS discovery, panel0
             backlight screen-off fallback, and direct input key injection.

The GESTURE_VARIANT environment variable is also supported.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --gesture-variant)
            if [[ $# -lt 2 ]]; then
                echo "--gesture-variant requires a value" >&2
                usage >&2
                exit 2
            fi
            GESTURE_VARIANT="$2"
            shift 2
            ;;
        --gesture-variant=*)
            GESTURE_VARIANT="${1#*=}"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$GESTURE_VARIANT" in
    stable)
        FEATURE_ARGS=()
        ARTIFACT_SUFFIX=""
        OUTPUT_SUBDIR=""
        ;;
    backlight)
        FEATURE_ARGS=(--features gesture-backlight)
        ARTIFACT_SUFFIX="-backlight-gesture"
        OUTPUT_SUBDIR="/backlight-gesture"
        ;;
    *)
        echo "Unsupported gesture variant: $GESTURE_VARIANT" >&2
        usage >&2
        exit 2
        ;;
esac

BUILD_DIR="${BUILD_DIR:-$ROOT/build}"
DEFAULT_OUT_DIR="${BUILD_DIR}/out${OUTPUT_SUBDIR}"
OUT_DIR="${OUT_DIR:-${DIST_DIR:-$DEFAULT_OUT_DIR}}"
ZIP_OUT="${ZIP_OUT:-$OUT_DIR/$PROJECT_NAME$ARTIFACT_SUFFIX-vendor.zip}"
BIN_OUT="${BIN_OUT:-$OUT_DIR/$HAL_NAME}"
SUMS_OUT="${SUMS_OUT:-$OUT_DIR/$PROJECT_NAME$ARTIFACT_SUFFIX-SHA256SUMS}"
INIT_RC="$ROOT/etc/init/$HAL_NAME.rc"

STAGE=""
cleanup() {
    [[ -z "$STAGE" ]] || rm -rf "$STAGE"
}
trap cleanup EXIT HUP INT TERM

find_ndk() {
    local candidate
    for candidate in \
        "${ANDROID_NDK_HOME:-}" \
        "${NDK_HOME:-}" \
        "$HOME/android-ndk-r26d" \
        "$HOME/android-ndk-r27c" \
        "/opt/android-ndk" \
        "$HOME/Android/Sdk/ndk/26.3.11579264"
    do
        if [[ -n "$candidate" && -d "$candidate/toolchains/llvm/prebuilt/linux-x86_64/sysroot" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

NDK="$(find_ndk)" || {
    echo "Android NDK not found. Set ANDROID_NDK_HOME to NDK r26/r27." >&2
    exit 1
}

export ANDROID_NDK_HOME="$NDK"
export ANDROID_API_LEVEL="$API"

if ! rustup target list --installed | grep -qx "$TARGET"; then
    rustup target add "$TARGET"
fi

cd "$ROOT"

echo "==> Checking formatting and tests (gesture variant: $GESTURE_VARIANT)"
cargo fmt --all -- --check
cargo test --locked --all-targets "${FEATURE_ARGS[@]}"

echo "==> Building $TARGET (API $API, gesture variant: $GESTURE_VARIANT)"
cargo build --locked --release --target "$TARGET" --bin "$PROJECT_NAME" "${FEATURE_ARGS[@]}"

BIN="$ROOT/target/$TARGET/release/$PROJECT_NAME"
file "$BIN" | grep -Eq 'ARM aarch64|AArch64'
readelf -W -l "$BIN" | grep -q "/system/bin/linker64"
readelf -W -l "$BIN" | grep '^  LOAD' | awk '{ if ($NF != "0x10000") exit 1 }'

echo "==> Staging release files"
STAGE="$(mktemp -d)"
mkdir -p "$STAGE/vendor/bin" "$STAGE/vendor/etc/init"

install -Dm0755 "$BIN" "$STAGE/vendor/bin/$HAL_NAME"
install -Dm0644 "$INIT_RC" "$STAGE/vendor/etc/init/$HAL_NAME.rc"
install -Dm0644 "$ROOT/README.md" "$STAGE/README.md"

mkdir -p "$(dirname "$ZIP_OUT")" "$(dirname "$BIN_OUT")" "$(dirname "$SUMS_OUT")"
rm -f "$ZIP_OUT" "$BIN_OUT" "$SUMS_OUT"
(
    cd "$STAGE"
    zip -X -qr "$ZIP_OUT" .
)
install -Dm0755 "$BIN" "$BIN_OUT"

echo "==> Verifying ZIP"
unzip -tq "$ZIP_OUT" >/dev/null
ZIP_LISTING="$(unzip -Z1 "$ZIP_OUT")"
for required in \
    README.md \
    "vendor/bin/$HAL_NAME" \
    "vendor/etc/init/$HAL_NAME.rc"
do
    if ! grep -Fxq "$required" <<<"$ZIP_LISTING"; then
        echo "ZIP is missing $required" >&2
        exit 1
    fi
done

ACTUAL_FILES="$(grep -Ev '/$' <<<"$ZIP_LISTING" | sort)"
EXPECTED_FILES="$(
    printf '%s\n' \
        README.md \
        "vendor/bin/$HAL_NAME" \
        "vendor/etc/init/$HAL_NAME.rc" |
        sort
)"
if [[ "$ACTUAL_FILES" != "$EXPECTED_FILES" ]]; then
    echo "ZIP contains unexpected files:" >&2
    diff -u <(printf '%s\n' "$EXPECTED_FILES") <(printf '%s\n' "$ACTUAL_FILES") >&2 || true
    exit 1
fi

(
    cd "$(dirname "$BIN_OUT")"
    sha256sum "$(basename "$BIN_OUT")"
    cd "$(dirname "$ZIP_OUT")"
    sha256sum "$(basename "$ZIP_OUT")"
) > "$SUMS_OUT"

echo
echo "Gesture variant: $GESTURE_VARIANT"
echo "Android binary: $BIN"
echo "Release binary: $BIN_OUT"
echo "Vendor ZIP:     $ZIP_OUT"
echo "Checksums:      $SUMS_OUT"
cat "$SUMS_OUT"
