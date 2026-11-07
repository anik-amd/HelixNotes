#!/usr/bin/env bash
# Boots the app on the emulator, seeds a vault, then drives the UI over CDP.
set -u

echo "=== adb root ==="
adb root || true
adb wait-for-device

echo "=== install ==="
adb install -r helixnotes-x86_64.apk

echo "=== seed vault ==="
adb shell appops set com.helixnotes.app MANAGE_EXTERNAL_STORAGE allow
adb shell mkdir -p /sdcard/Documents/HelixNotes
printf '# Sample Note One\n\nHello from the seeded vault.\n' > sample-one.md
printf '# Sample Note Two\n\nSecond seeded note.\n' > sample-two.md
adb push sample-one.md "/sdcard/Documents/HelixNotes/Sample Note One.md"
adb push sample-two.md "/sdcard/Documents/HelixNotes/Sample Note Two.md"

echo "=== launch on a narrow screen ==="
adb shell wm size 720x1600
adb shell wm density 320
adb shell am start -W -n com.helixnotes.app/.MainActivity
sleep 10

echo "=== devtools socket ==="
adb shell cat /proc/net/unix | grep -i devtools | head -n 5 || true
sock=""
for _ in $(seq 1 40); do
  sock=$(adb shell cat /proc/net/unix | grep -o 'webview_devtools_remote_[0-9]*' | head -n 1 | tr -d '\r')
  [ -n "$sock" ] && break
  sleep 1
done
if [ -z "$sock" ]; then
  pid=$(adb shell pidof com.helixnotes.app | tr -d '\r')
  echo "app pid: $pid"
  sock="webview_devtools_remote_$pid"
fi
echo "devtools socket: $sock"
adb forward tcp:9222 "localabstract:$sock" || true
curl -s http://127.0.0.1:9222/json/version || true
echo

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

