# Android

Apps and examples are NativeActivity apps built with `cargo-apk`, aarch64
only, min SDK 26, target SDK 35.

## Setup

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
# Android SDK with platform-tools, build-tools, platform android-35 and an NDK
export ANDROID_HOME=/opt/android-sdk
export NDK_HOME=$ANDROID_HOME/ndk/<version>
# Release builds need a keystore; the debug one is fine for development.
export CARGO_APK_RELEASE_KEYSTORE=$HOME/.android/debug.keystore
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
```

`cargo-apk` is capped at platform 35, so `android-35` must be installed even
if newer platforms are present.

## The demo

Run on a device (USB debugging on, visible in `adb devices`):

```sh
cargo apk run -p dfui-demo --lib --release
adb logcat | grep -E 'dfui|fps'   # logs; frame stats every 2 s
```

`--lib` is needed because the demo crate also has a desktop binary, which
makes `cargo-apk` panic ("Bin is not compatible with Cdylib") after building.
The APK lands in `target/release/apk/dfui-demo.apk`; `cargo apk build`
instead of `run` only builds it. For real distribution, point the keystore
variables at a release keystore. Frame stats (`fps … cpu … gpu …`) are how the
baselines in `TODO.md` are measured; use the demo's Bench screen.

## The map and video examples

Each embeds its data at build time:

```sh
DFUI_MAP_OSM=/path/to/amsterdam.json cargo apk run -p dfui-map \
    --example osm_android --release
DFUI_MEDIA_CLIP=movie.mp4 cargo apk run -p dfui-media --example play_android
```

Map tiles stream in over the first frames rather than all in the first one.
The clip must be fragmented MP4, as dfui-media's README says.

## In an app

`android_main` calls `dfui::run_android`, which returns when the
activity is destroyed. If the process lives on (a service, a download),
Android calls `android_main` again for the next activity, and it can run
again. `eprintln!` output is lost on Android, so log with `log` and
`android_logger` (see `android_main` in `demo/src/lib.rs`) and read the logs
with `adb logcat`. `dfui::android::with_activity` runs JNI calls with the
activity from any thread.
