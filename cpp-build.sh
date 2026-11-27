#!/usr/bin/env bash
# Build, test, verify, and package the standalone C++ touch HAL.
set -euo pipefail
umask 022

ROOT="$(cd "$(dirname "$0")" && pwd)"
PROJECT_NAME="oplus-hardware-touch-service"
HAL_NAME="vendor.oplus.hardware.touch-V2-service"
API="${ANDROID_API_LEVEL:-30}"
VARIANT="${CPP_GESTURE_VARIANT:-stable}"
BUILD_ROOT="${CPP_BUILD_DIR:-$ROOT/build/cpp}"
OUT_ROOT="${CPP_OUT_DIR:-$BUILD_ROOT/out}"
INIT_RC="$ROOT/etc/init/$HAL_NAME.rc"

usage() {
    cat <<'EOF'
Usage: ./cpp-build.sh [--variant stable|backlight|both]

C++ gesture backends:
  stable     Standard touchscreen discovery and cmd input preview path.
  backlight  Adds FTS discovery, panel0-backlight screen-off fallback, and
             the direct /system/bin/input keyevent 134 preview path.
  both       Build stable and backlight into separate output directories.

Environment:
  ANDROID_NDK_HOME / ANDROID_NDK_ROOT  Android NDK r26/r27 location
  CPP_CLANG / CPP_CLANGXX              Host-executable Clang compilers
  ANDROID_API_LEVEL                    Target API (default: 30)
  CPP_GESTURE_VARIANT                  Default variant when no option is given
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --variant|--gesture-variant)
            if [[ $# -lt 2 ]]; then
                echo "$1 requires a value" >&2
                usage >&2
                exit 2
            fi
            VARIANT="$2"
            shift 2
            ;;
        --variant=*|--gesture-variant=*)
            VARIANT="${1#*=}"
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

case "$VARIANT" in
    stable|backlight|both) ;;
    *)
        echo "Unsupported C++ gesture variant: $VARIANT" >&2
        usage >&2
        exit 2
        ;;
esac

find_ndk() {
    local candidate
    for candidate in \
        "${ANDROID_NDK_ROOT:-}" \
        "${ANDROID_NDK_HOME:-}" \
        "${NDK_HOME:-}" \
        "$HOME/android-ndk-r26d" \
        "$HOME/android-ndk-r27c" \
        "/opt/android-ndk"
    do
        if [[ -n "$candidate" &&
              -d "$candidate/toolchains/llvm/prebuilt/linux-x86_64/sysroot" ]];
        then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

compiler_runs() {
    local compiler="$1"
    local lib_dir
    lib_dir="$(cd "$(dirname "$compiler")/.." && pwd)/lib"
    if [[ -d "$lib_dir" ]]; then
        LD_LIBRARY_PATH="$lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
            "$compiler" --version >/dev/null 2>&1
    else
        "$compiler" --version >/dev/null 2>&1
    fi
}

find_clangxx() {
    local ndk="$1"
    local candidate
    for candidate in \
        "${CPP_CLANGXX:-}" \
        "$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/clang++" \
        "$(command -v clang++ 2>/dev/null || true)" \
        "$HOME/toolchains/clang23/usr/bin/clang++" \
        "/usr/bin/clang++"
    do
        if [[ -n "$candidate" && -x "$candidate" ]] && compiler_runs "$candidate"; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

NDK="$(find_ndk)" || {
    echo "Android NDK not found. Set ANDROID_NDK_HOME or ANDROID_NDK_ROOT." >&2
    exit 1
}
CLANGXX="$(find_clangxx "$NDK")" || {
    echo "No host-executable clang++ was found. Set CPP_CLANGXX." >&2
    exit 1
}
CLANG="${CPP_CLANG:-$(dirname "$CLANGXX")/clang}"
if [[ ! -x "$CLANG" ]] || ! compiler_runs "$CLANG"; then
    CLANG="$CLANGXX"
fi

CLANG_LIB_DIR="$(cd "$(dirname "$CLANGXX")/.." && pwd)/lib"
if [[ -d "$CLANG_LIB_DIR" ]]; then
    export LD_LIBRARY_PATH="$CLANG_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
export ANDROID_NDK_ROOT="$NDK"
export ANDROID_NDK_HOME="$NDK"
export ANDROID_API_LEVEL="$API"
export CPP_CLANG="$CLANG"
export CPP_CLANGXX="$CLANGXX"

CMAKE_BIN="${CMAKE:-cmake}"
if ! command -v "$CMAKE_BIN" >/dev/null 2>&1; then
    echo "cmake is required" >&2
    exit 1
fi

echo "==> Building and running host C++ tests"
HOST_BUILD="$BUILD_ROOT/host"
"$CMAKE_BIN" -S "$ROOT/cpp" -B "$HOST_BUILD" \
    -DCMAKE_BUILD_TYPE=Release \
    -DOPLUS_TOUCH_BUILD_TESTS=ON \
    -DOPLUS_TOUCH_ANDROID=OFF
"$CMAKE_BIN" --build "$HOST_BUILD" --parallel
"$HOST_BUILD/oplus_touch_cpp_tests"
"$HOST_BUILD/bin/$HAL_NAME" --test

build_variant() {
    local variant="$1"
    local feature="OFF"
    local output_subdir=""
    local artifact_suffix="-cpp"
    local build_dir="$BUILD_ROOT/build-$variant"
    local out_dir="$OUT_ROOT"
    local zip_out
    local bin_out
    local sums_out
    local stage

    if [[ "$variant" == "backlight" ]]; then
        feature="ON"
        output_subdir="/backlight-gesture"
        artifact_suffix="-cpp-backlight-gesture"
        out_dir="$OUT_ROOT/backlight-gesture"
    fi

    zip_out="$out_dir/${PROJECT_NAME}${artifact_suffix}-vendor.zip"
    bin_out="$out_dir/$HAL_NAME"
    sums_out="$out_dir/${PROJECT_NAME}${artifact_suffix}-SHA256SUMS"

    echo "==> Building C++ $variant HAL for Android API $API"
    "$CMAKE_BIN" -S "$ROOT/cpp" -B "$build_dir" \
        -DCMAKE_TOOLCHAIN_FILE="$ROOT/cpp/cmake/android-aarch64-clang.cmake" \
        -DANDROID_NDK_ROOT="$NDK" \
        -DCPP_CLANG="$CLANG" \
        -DCPP_CLANGXX="$CLANGXX" \
        -DANDROID_API_LEVEL="$API" \
        -DOPLUS_TOUCH_ANDROID=ON \
        -DOPLUS_TOUCH_BUILD_TESTS=OFF \
        -DOPLUS_TOUCH_BACKLIGHT="$feature" \
        -DCMAKE_BUILD_TYPE=Release
    "$CMAKE_BIN" --build "$build_dir" --parallel

    local binary="$build_dir/bin/$HAL_NAME"
    file "$binary" | grep -Eq 'ELF 64-bit.*ARM aarch64'
    readelf -W -l "$binary" | grep -q '/system/bin/linker64'
    if readelf -W -l "$binary" |
        awk '$1 == "LOAD" && $NF != "0x10000" { exit 1 }'; then
        :
    else
        echo "$variant binary does not use 64 KiB ELF load alignment" >&2
        return 1
    fi
    if readelf -d "$binary" | grep -q 'libc++_shared.so'; then
        echo "$variant binary unexpectedly depends on libc++_shared.so" >&2
        return 1
    fi

    stage="$(mktemp -d)"
    mkdir -p "$stage/vendor/bin" "$stage/vendor/etc/init"
    install -Dm0755 "$binary" "$stage/vendor/bin/$HAL_NAME"
    install -Dm0644 "$INIT_RC" "$stage/vendor/etc/init/$HAL_NAME.rc"
    install -Dm0644 "$ROOT/README.md" "$stage/README.md"

    mkdir -p "$out_dir"
    rm -f "$zip_out" "$bin_out" "$sums_out"
    (
        cd "$stage"
        zip -X -qr "$zip_out" .
    )
    install -Dm0755 "$binary" "$bin_out"
    rm -rf "$stage"

    unzip -tq "$zip_out" >/dev/null
    local listing
    local actual
    local expected
    listing="$(unzip -Z1 "$zip_out")"
    actual="$(grep -Ev '/$' <<<"$listing" | sort)"
    expected="$(
        printf '%s\n' \
            README.md \
            "vendor/bin/$HAL_NAME" \
            "vendor/etc/init/$HAL_NAME.rc" |
            sort
    )"
    if [[ "$actual" != "$expected" ]]; then
        echo "$variant ZIP contains unexpected or missing files:" >&2
        diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") >&2 || true
        return 1
    fi

    (
        cd "$(dirname "$bin_out")"
        sha256sum "$(basename "$bin_out")"
        cd "$(dirname "$zip_out")"
        sha256sum "$(basename "$zip_out")"
    ) > "$sums_out"

    echo
    echo "C++ gesture variant: $variant"
    echo "Android binary:     $binary"
    echo "Vendor binary:      $bin_out"
    echo "Vendor ZIP:         $zip_out"
    echo "Checksums:          $sums_out"
    cat "$sums_out"
    echo
}

if [[ "$VARIANT" == "both" ]]; then
    build_variant stable
    build_variant backlight
else
    build_variant "$VARIANT"
fi
