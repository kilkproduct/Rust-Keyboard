#!/bin/sh
# Builds the Rust engine and drops it where Gradle looks for JNI libraries.
#
#   ./rust/build-android.sh
#
# This library *is* the engine: the C++ engine it replaced is not in the build
# any more, so there is nothing to fall back to. Every missing piece is
# therefore a hard failure - an APK without libkeyboard_core.so installs fine
# and then crashes on the first keystroke.
#
# ABIs match `abiFilters` in build.gradle. Set KB_ABIS to build a subset, e.g.
#   KB_ABIS="arm64-v8a" ./rust/build-android.sh

set -eu

cd "$(dirname "$0")"
PROJECT_DIR="$(cd .. && pwd)"
OUTPUT_DIR="$PROJECT_DIR/java/jniLibs"
ALL_ABIS="${KB_ABIS:-arm64-v8a}"

fail() {
    echo "rust: $1" >&2
    exit 1
}

if ! command -v cargo >/dev/null 2>&1; then
    fail "cargo not found. Install Rust (see rust/README.md); the keyboard has no other engine."
fi

find_ndk() {
    if [ -n "${ANDROID_NDK_HOME:-}" ] && [ -d "$ANDROID_NDK_HOME" ]; then
        echo "$ANDROID_NDK_HOME"
        return 0
    fi
    if [ -n "${ANDROID_NDK_ROOT:-}" ] && [ -d "$ANDROID_NDK_ROOT" ]; then
        echo "$ANDROID_NDK_ROOT"
        return 0
    fi
    if [ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME/ndk" ]; then
        ls -d "$ANDROID_HOME/ndk"/* 2>/dev/null | sort -V | tail -n 1
        return 0
    fi
    if [ -d "$HOME/Android/Sdk/ndk" ]; then
        ls -d "$HOME/Android/Sdk/ndk"/* 2>/dev/null | sort -V | tail -n 1
        return 0
    fi
    return 1
}

check_output() {
    abi="$1"
    lib="$OUTPUT_DIR/$abi/libkeyboard_core.so"
    [ -f "$lib" ] || fail "no library at $lib after building $abi"
    size=$(wc -c < "$lib" | tr -d ' ')
    [ "$size" -gt 100000 ] || fail "$lib is only $size bytes, which cannot be the engine"
    echo "rust: $lib ($size bytes)"
}

if command -v cargo-ndk >/dev/null 2>&1 || cargo ndk --version >/dev/null 2>&1; then
    flags=""
    for abi in $ALL_ABIS; do
        flags="$flags -t $abi"
    done
    # shellcheck disable=SC2086
    cargo ndk $flags --platform 24 -o "$OUTPUT_DIR" build --release -p kb-jni \
        || fail "cargo ndk failed"
    for abi in $ALL_ABIS; do
        check_output "$abi"
    done
    echo "rust: done"
    exit 0
fi

# No cargo-ndk: drive cargo directly with the NDK's clang as the linker.
NDK="$(find_ndk || true)"
[ -n "$NDK" ] || fail "no Android NDK found; set ANDROID_NDK_HOME (see rust/README.md)"

HOST_TAG="linux-x86_64"
case "$(uname -s)" in
    Darwin) HOST_TAG="darwin-x86_64" ;;
esac
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin"
[ -d "$TOOLCHAIN" ] || fail "unexpected NDK layout at $TOOLCHAIN"

API=24

build_for() {
    abi="$1"
    target="$2"
    linker="$3"
    [ -x "$TOOLCHAIN/$linker" ] || fail "$linker is missing from $TOOLCHAIN"
    rustup target list --installed 2>/dev/null | grep -qx "$target" \
        || fail "target $target is not installed (rustup target add $target)"
    env_var="$(echo "$target" | tr 'a-z-' 'A-Z_')"
    echo "rust: building $target"
    env "CARGO_TARGET_${env_var}_LINKER=$TOOLCHAIN/$linker" \
        "CC_${target}=$TOOLCHAIN/$linker" \
        "AR_${target}=$TOOLCHAIN/llvm-ar" \
        cargo build --release -p kb-jni --target "$target" \
        || fail "cargo build failed for $target"
    mkdir -p "$OUTPUT_DIR/$abi"
    cp "target/$target/release/libkeyboard_core.so" "$OUTPUT_DIR/$abi/libkeyboard_core.so"
    check_output "$abi"
}

for abi in $ALL_ABIS; do
    case "$abi" in
        arm64-v8a)   build_for "$abi" aarch64-linux-android      "aarch64-linux-android$API-clang" ;;
        armeabi-v7a) build_for "$abi" armv7-linux-androideabi    "armv7a-linux-androideabi$API-clang" ;;
        x86)         build_for "$abi" i686-linux-android         "i686-linux-android$API-clang" ;;
        x86_64)      build_for "$abi" x86_64-linux-android       "x86_64-linux-android$API-clang" ;;
        *) fail "unknown ABI $abi" ;;
    esac
done

echo "rust: done"
