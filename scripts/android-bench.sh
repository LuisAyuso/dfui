#!/bin/sh
# Cross-compiles a criterion bench and runs it on the connected device.
# Usage: scripts/android-bench.sh gpu|cpu [criterion filter]
set -eu
bench=${1:?usage: $0 gpu|cpu [filter]}
shift
bin=$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$bin/aarch64-linux-android26-clang
export CC_aarch64_linux_android=$bin/aarch64-linux-android26-clang
export AR_aarch64_linux_android=$bin/llvm-ar
exe=$(cargo bench --target aarch64-linux-android --bench "$bench" --no-run --message-format=json 2>/dev/null |
    sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -1)
adb push "$exe" /data/local/tmp/dfui-bench >/dev/null
adb shell "cd /data/local/tmp && ./dfui-bench --bench $*"
