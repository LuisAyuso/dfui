#!/bin/sh
# Rebuilds android/classes.dex, which dfui embeds; run after changing android/**/*.java.
set -eu
cd "$(dirname "$0")/.."
jar=$ANDROID_HOME/platforms/android-35/android.jar
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
javac --release 8 -Xlint:-options -cp "$jar" -d "$out" $(find android -name '*.java')
"$ANDROID_HOME"/cmdline-tools/latest/bin/d8 --min-api 26 --lib "$jar" --output "$out" $(find "$out" -name '*.class')
cp "$out/classes.dex" android/classes.dex
