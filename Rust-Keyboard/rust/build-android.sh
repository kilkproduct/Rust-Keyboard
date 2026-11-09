#!/bin/sh
# Builds the Rust half of the native layer and drops it where Gradle looks for
# JNI libraries.
#
#   ./rust/build-android.sh
#
# Requires a Rust toolchain (rustup) and an Android NDK. When either is absent
# the script exits successfully without doing anything: the app then simply
# runs on the C++ engine, exactly like before this port started.
#
# ABIs match `abiFilters` in build.gradle. Set KB_ABIS to build a subset, e.g.
#   KB_ABIS="arm64-v8a" ./rust/build-android.sh

set -e

cd "$(dirname "$0")"
WORKSPACE_DIR="$(pwd)"
PROJECT_DIR="$(cd .. && pwd)"
OUTPUT_DIR="$PROJECT_DIR/java/jniLibs"

if ! command -v cargo >/dev/null 2>&1; then
    echo "rust: cargo not found, skipping (the C++ engine stays in charge)"
    exit 0
fi

ALL_ABIS="${KB_ABIS:-arm64-v8a}"

if command -v cargo-ndk >/dev/null 2>&1 || cargo ndk --version >/dev/null 2>&1; then
    FLAGS=""
    for abi in $ALL_ABIS; do
        FLAGS="$FLAGS -t $abi"
    done
    # shellcheck disable=SC2086
    cargo ndk $FLAGS --platform 24 -o "$OUTPUT_DIR" build --release -p kb-jni
    echo "rust: libkeyboard_core.so written to java/jniLibs"
    exit 0
fi

# Fallback: drive cargo directly with the NDK's clang as the linker.
find_ndk() {
    if [ -n "$ANDROID_NDK_HOME" ] && [ -d "$ANDROID_NDK_HOME" ]; then
        echo "$ANDROID_NDK_HOME"
        return 0
    fi
    if [ -n "$ANDROID_NDK_ROOT" ] && [ -d "$ANDROID_NDK_ROOT" ]; then
        echo "$ANDROID_NDK_ROOT"
        return 0
    fi
    if [ -n "$ANDROID_HOME" ] && [ -d "$ANDROID_HOME/ndk" ]; then
        ls -d "$ANDROID_HOME/ndk"/* 2>/dev/null | sort -V | tail -n 1
        return 0
    fi
    if [ -d "$HOME/Android/Sdk/ndk" ]; then
        ls -d "$HOME/Android/Sdk/ndk"/* 2>/dev/null | sort -V | tail -n 1
        return 0
    fi
    return 1
}

NDK="$(find_ndk || true)"
if [ -z "$NDK" ]; then
    echo "rust: no Android NDK found, skipping (set ANDROID_NDK_HOME to build)"
    exit 0
fi

HOST_TAG="linux-x86_64"
case "$(uname -s)" in
    Darwin) HOST_TAG="darwin-x86_64" ;;
esac
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin"

if [ ! -d "$TOOLCHAIN" ]; then
    echo "rust: unexpected NDK layout at $TOOLCHAIN, skipping"
    exit 0
fi

API=24

build_for() {
    abi="$1"
    target="$2"
    linker="$3"
    if [ ! -x "$TOOLCHAIN/$linker" ]; then
        echo "rust: $linker missing, skipping $abi"
        return 0
    fi
    if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        echo "rust: target $target not installed (rustup target add $target), skipping $abi"
        return 0
    fi
    env_var="$(echo "$target" | tr 'a-z-' 'A-Z_')"
    echo "rust: building $target"
    env "CARGO_TARGET_${env_var}_LINKER=$TOOLCHAIN/$linker" \
        "CC_${target}=$TOOLCHAIN/$linker" \
        "AR_${target}=$TOOLCHAIN/llvm-ar" \
        cargo build --release -p kb-jni --target "$target"
    mkdir -p "$OUTPUT_DIR/$abi"
    cp "target/$target/release/libkeyboard_core.so" "$OUTPUT_DIR/$abi/libkeyboard_core.so"
    echo "rust: java/jniLibs/$abi/libkeyboard_core.so"
}

for abi in $ALL_ABIS; do
    case "$abi" in
        arm64-v8a)   build_for "$abi" aarch64-linux-android      "aarch64-linux-android$API-clang" ;;
        armeabi-v7a) build_for "$abi" armv7-linux-androideabi    "armv7a-linux-androideabi$API-clang" ;;
        x86)         build_for "$abi" i686-linux-android         "i686-linux-android$API-clang" ;;
        x86_64)      build_for "$abi" x86_64-linux-android       "x86_64-linux-android$API-clang" ;;
        *) echo "rust: unknown ABI $abi" ;;
    esac
done

echo "rust: done"
