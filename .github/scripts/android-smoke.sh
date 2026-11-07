#!/usr/bin/env bash
# Boots the app on the emulator, seeds a vault, then drives the UI over CDP.
# Phase A checks the storage-permission gate, phase B checks the note flow.
set -u

echo "=== adb root ==="
adb root || true
adb wait-for-device

echo "=== install ==="
adb uninstall com.helixnotes.app || true
adb install -r helixnotes-x86_64.apk

echo "=== apk manifest permissions ==="
aapt2_bin=$(ls "$ANDROID_HOME"/build-tools/*/aapt2 2>/dev/null | sort -V | tail -n 1)
if [ -n "$aapt2_bin" ]; then
  "$aapt2_bin" dump permissions helixnotes-x86_64.apk || true
  "$aapt2_bin" dump badging helixnotes-x86_64.apk | grep -E "^package:|targetSdkVersion" || true
else
  echo "aapt2 not found"
fi

echo "=== seed vault ==="
adb shell mkdir -p /sdcard/Documents/HelixNotes
printf '# Sample Note One\n\nHello from the seeded vault.\n' > sample-one.md
printf '# Sample Note Two\n\nSecond seeded note.\n' > sample-two.md
adb push sample-one.md "/sdcard/Documents/HelixNotes/Sample Note One.md"
adb push sample-two.md "/sdcard/Documents/HelixNotes/Sample Note Two.md"

echo "=== narrow screen ==="
adb shell wm size 720x1600
adb shell wm density 320

echo "=== installed package permissions ==="
adb shell dumpsys package com.helixnotes.app | grep -iE "MANAGE_EXTERNAL_STORAGE|READ_EXTERNAL|WRITE_EXTERNAL|INTERNET" | head -n 20 || true

echo "=== smoke test ==="
node .github/scripts/android-smoke.mjs > smoke.txt 2>&1
status=$?
cat smoke.txt

echo "=== capture ==="
adb shell screencap -p /sdcard/screen.png
adb pull /sdcard/screen.png screen.png
adb shell uiautomator dump /sdcard/window.xml || true
adb pull /sdcard/window.xml window.xml || true
adb shell dumpsys activity activities > activities.txt
adb logcat -d > logcat.txt
adb logcat -d -s chromium:V Tauri:V RustWebView:V AndroidRuntime:E > webview.txt || true
echo '--- LOGCAT ERRORS ---'
grep -iE 'error|exception|fatal|crash|tauri|chromium' logcat.txt | tail -n 200 || true

exit "$status"

