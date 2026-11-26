#!/bin/sh
# Link Rust's aarch64-linux-android target against bionic without running an
# x86_64 NDK clang binary. This is required on native aarch64 build hosts.
set -eu

if [ -n "${ANDROID_NDK_HOME:-}" ]; then
    NDK="$ANDROID_NDK_HOME"
elif [ -n "${NDK_HOME:-}" ]; then
    NDK="$NDK_HOME"
else
    NDK=""
    for candidate in \
        "$HOME/android-ndk-r26d" \
        "$HOME/android-ndk-r27c" \
        "/opt/android-ndk" \
        "$HOME/Android/Sdk/ndk/26.3.11579264"
    do
        if [ -d "$candidate/toolchains/llvm/prebuilt/linux-x86_64/sysroot" ]; then
            NDK="$candidate"
            break
        fi
    done
fi

if [ -z "$NDK" ]; then
    echo "Android NDK not found; set ANDROID_NDK_HOME" >&2
    exit 1
fi

SYSROOT="$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
API="${ANDROID_API_LEVEL:-30}"
ARCH_LIB="$SYSROOT/usr/lib/aarch64-linux-android/$API"
CLANG_RT="$NDK/toolchains/llvm/prebuilt/linux-x86_64/lib/clang/17/lib/linux/aarch64"

if [ ! -d "$ARCH_LIB" ]; then
    echo "Android NDK sysroot not found: $ARCH_LIB" >&2
    exit 1
fi

RUST_SYSROOT="$(rustc --print sysroot)"
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
NDK_LLD="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/ld.lld"

if [ -n "${RUST_LLD:-}" ]; then
    LLD="$RUST_LLD"
elif [ "$(uname -m)" = "x86_64" ] && [ -x "$NDK_LLD" ]; then
    # The NDK linker is an x86_64 host binary and is the simplest option on
    # conventional CI runners. It cannot run on native AArch64 hosts.
    LLD="$NDK_LLD"
else
    LLD=""
    # Prefer the ld.lld entry point: newer Rust releases make rust-lld a
    # generic driver that requires an ld.lld/ld64.lld style argv[0].
    for candidate in \
        "$RUST_SYSROOT/lib/rustlib/$HOST_TRIPLE/bin/gcc-ld/ld.lld" \
        "$RUST_SYSROOT/lib/rustlib/$HOST_TRIPLE/bin/rust-lld" \
        "$RUST_SYSROOT/lib/rustlib/aarch64-unknown-linux-gnu/bin/gcc-ld/ld.lld" \
        "$RUST_SYSROOT/lib/rustlib/aarch64-unknown-linux-gnu/bin/rust-lld"
    do
        if [ -x "$candidate" ]; then
            LLD="$candidate"
            break
        fi
    done
fi

if [ -z "$LLD" ] || [ ! -x "$LLD" ]; then
    echo "No usable LLD found; set RUST_LLD to an AArch64-compatible linker" >&2
    exit 1
fi

# rustc's Android target uses the GCC linker flavor and therefore passes
# driver-style options such as -Wl,-z,now and -nodefaultlibs. rust-lld is a
# linker, not a compiler driver, so expand those options before invoking it.
ARGS_FILE="$(mktemp "${TMPDIR:-/tmp}/oplus-hardware-touch-service-link.XXXXXX")"
trap 'rm -f "$ARGS_FILE"' EXIT HUP INT TERM

emit_arg() {
    printf '%s\0' "$1" >> "$ARGS_FILE"
}

emit_arg "$ARCH_LIB/crtbegin_dynamic.o"
for arg in "$@"; do
    case "$arg" in
        -nodefaultlibs|-nostartfiles|-nostdlib)
            # These flags control the compiler driver's default libraries.
            # rustc already supplies the exact bionic libraries we need.
            ;;
        -Wl,*)
            old_ifs="$IFS"
            IFS=','
            for option in ${arg#-Wl,}; do
                [ -n "$option" ] && emit_arg "$option"
            done
            IFS="$old_ifs"
            ;;
        *)
            emit_arg "$arg"
            ;;
    esac
done
emit_arg "$ARCH_LIB/crtend_android.o"

xargs -0 "$LLD" \
    --sysroot="$SYSROOT" \
    --dynamic-linker=/system/bin/linker64 \
    -L"$ARCH_LIB" \
    -L"$SYSROOT/usr/lib/aarch64-linux-android" \
    -L"$CLANG_RT" \
    < "$ARGS_FILE"
