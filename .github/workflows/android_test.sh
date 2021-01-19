#!/bin/sh

set -ex

cd examples

rustup target install x86_64-linux-android
cargo install cargo-apk
cargo apk run --example quad_android --target x86_64-linux-android --features gl

sleep 30s

adb shell /system/bin/screencap -p /sdcard/screenshot.png
adb pull /sdcard/screenshot.png ~/screenshot.png
adb logcat *:S RustStdoutStderr:V -d > ~/logcat.log

if grep -e 'thread.*panicked at' ~/logcat.log;
then
    exit 1
else
    exit 0
fi
