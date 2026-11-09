#!/bin/sh
# Checks the things a release APK needs in order to start.
#
# Everything here is a failure mode that produces "the app crashes on launch"
# or "the keyboard does not come up" on a device and nothing at all in the
# build log:
#
#   * an asset the keyboard reads on its first frame is missing, because the
#     resource pruning step deleted one directory too many;
#   * a class the manifest names by string was renamed or removed by R8, so
#     Android cannot instantiate the Application, the IME service or an
#     activity;
#   * the native library that registers the engine is not in the package.
#
# Usage: verify-apk-startup.sh <apk> [<apk> ...]
set -e

status=0
for apk in "$@"; do
    [ -f "$apk" ] || continue
    echo "== startup check: $apk"

    listing=$(unzip -l "$apk")

    # 1. Assets loaded while the keyboard is being laid out for the first time.
    for required in \
        assets/layouts/names.yaml \
        assets/layouts/mapping.yaml \
        assets/layouts/Special/error.yaml \
        assets/dict/main_en.dict.gzip
    do
        if ! echo "$listing" | grep -q "[ ]$required\$"; then
            echo "::error::$apk does not contain $required"
            status=1
        fi
    done
    if ! echo "$listing" | grep -q "assets/layouts/Default/.*[.]yaml\$"; then
        echo "::error::$apk has no layout in assets/layouts/Default"
        status=1
    fi
    if ! echo "$listing" | grep -q "assets/themes/"; then
        echo "::warning::$apk ships no themes; the theme picker will be empty"
    fi
    if ! echo "$listing" | grep -q "lib/arm64-v8a/libkeyboard_core.so"; then
        echo "::error::$apk does not contain lib/arm64-v8a/libkeyboard_core.so"
        status=1
    fi

    # 2. Every class the manifest names has to be in the dex, under that exact
    #    name: this is what breaks when a shrinker is allowed to repackage the
    #    application.
    aapt2_bin=$(ls "${ANDROID_HOME:-/usr/local/lib/android/sdk}"/build-tools/*/aapt2 2>/dev/null | tail -1)
    if [ -z "$aapt2_bin" ]; then
        echo "::warning::no aapt2 found, skipping the manifest class check"
        continue
    fi

    manifest=$("$aapt2_bin" dump xmltree --file AndroidManifest.xml "$apk" 2>/dev/null || true)
    if [ -z "$manifest" ]; then
        echo "::warning::aapt2 could not dump the manifest of $apk"
        continue
    fi

    package=$(echo "$manifest" | sed -n 's/^ *package="\([^"]*\)".*/\1/p' | head -1)
    [ -n "$package" ] || package=$(echo "$manifest" | grep -o 'package=[^ ]*' | head -1 | cut -d= -f2 | tr -d '"')
    echo "   package: $package"

    # Class descriptors are plain ASCII in the dex string table, so a binary
    # safe grep finds them without needing dexdump on the runner.
    dex=$(unzip -p "$apk" 'classes*.dex' | grep -a -o -E 'L[a-zA-Z0-9/$_]+;' | sort -u)

    # Component and application names only: dotted, at least one upper case
    # letter in the last segment, and either fully qualified or package relative.
    classes=$(echo "$manifest" \
        | sed -n 's/.*android:name[^=]*="\([^"]*\)".*/\1/p' \
        | grep -E '^[.a-zA-Z][a-zA-Z0-9_.$]*$' \
        | grep -E '[.][A-Z]|^[.]' \
        | grep -vE '^(android[.]|androidx[.]|com[.]google[.]|kotlin[.]|dalvik[.])' \
        | grep -vE '[.](permission|action|category|scheme|authorities)[.]' \
        | sort -u)

    missing=0
    for name in $classes; do
        case "$name" in
            .*) binary="${package}${name}" ;;
            "$package".*) binary="$name" ;;
            *) continue ;;
        esac
        binary=$(echo "$binary" | tr '.' '/')
        if ! echo "$dex" | grep -qx "L$binary;"; then
            echo "::error::$apk names $binary in its manifest but the class is not in the dex"
            missing=$((missing + 1))
        fi
    done
    if [ "$missing" -gt 0 ]; then
        status=1
    else
        echo "   every manifest class is in the dex"
    fi

    # 3. The strings the app only reaches through getIdentifier() (they are
    #    listed in res/raw/keep.xml) have to survive the resource shrinker: the
    #    system asks for them by name when it renders the keyboard picker.
    subtype_strings=$("$aapt2_bin" dump resources "$apk" 2>/dev/null \
        | grep -c "string/subtype_" || true)
    case "$subtype_strings" in
        ''|*[!0-9]*) echo "::warning::could not read the resource table of $apk" ;;
        0)
            echo "::error::$apk has no subtype_* strings; the language list would be empty"
            status=1
            ;;
        *) echo "   subtype strings in the package: $subtype_strings" ;;
    esac
done

exit "$status"
