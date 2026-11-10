#!/usr/bin/env bash
# Install SolidCraft.app (packaging/ios/bundle-sim.sh) in an iPad simulator, launch it and take
# pictures of the screen:
#
#   $OUT/shot-<seconds>.png     the simulator's screen that long after launch
#   $OUT/stderr.log             what the app logged (and a panic message, if it panicked)
#   $OUT/crash-*.ips            the system's crash report, if the app crashed
#
# Usage: packaging/ios/run-sim.sh [SolidCraft.app]
#   SIM_DEVICE   the simulator to use (default: the newest iPad Pro 13-inch, else any iPad)
#   SIM_SHOTS    seconds after launch at which to take a picture (default: "5 20 45")
#
# Needs Xcode with an iOS simulator runtime. Exits non-zero when the app is no longer running
# after the last picture.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP="${1:-$ROOT/dist/ios-sim/SolidCraft.app}"
OUT="${OUT:-$ROOT/dist/ios-sim/run}"
BUNDLE_ID=io.github.bzeeman.solidcraft-ipad
SHOTS="${SIM_SHOTS:-5 20 45}"

if [ ! -d "$APP" ]; then
  echo "error: $APP is missing (run packaging/ios/bundle-sim.sh first)" >&2
  exit 1
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# The id of the last available simulator whose line matches, newest runtime last.
find_device() {
  xcrun simctl list devices available | grep -F "$1" | grep -oE '[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}' | tail -1
}
UDID=""
for name in ${SIM_DEVICE:+"$SIM_DEVICE"} "iPad Pro 13-inch (M4)" "iPad Pro 13-inch" "iPad Pro" "iPad"; do
  UDID="$(find_device "$name" || true)"
  if [ -n "$UDID" ]; then
    echo "simulator: $(xcrun simctl list devices available | grep -F "$UDID" | sed 's/^ *//')"
    break
  fi
done
if [ -z "$UDID" ]; then
  echo "error: no iPad simulator is available:" >&2
  xcrun simctl list devices available >&2
  exit 1
fi

xcrun simctl boot "$UDID" 2> /dev/null || true
xcrun simctl bootstatus "$UDID" -b
xcrun simctl install "$UDID" "$APP"
# SIMCTL_CHILD_* reaches the app as its environment.
launched="$(SIMCTL_CHILD_RUST_BACKTRACE=1 xcrun simctl launch --terminate-running-process \
  --stdout="$OUT/stdout.log" --stderr="$OUT/stderr.log" "$UDID" "$BUNDLE_ID")"
echo "$launched"
PID="${launched##*: }"

waited=0
for at in $SHOTS; do
  sleep $((at - waited))
  waited=$at
  xcrun simctl io "$UDID" screenshot "$OUT/shot-$at.png" || true
  if ! kill -0 "$PID" 2> /dev/null; then
    echo "the app stopped within $at seconds of launch"
    break
  fi
done

status=0
if kill -0 "$PID" 2> /dev/null; then
  echo "the app is still running after $waited seconds"
else
  status=1
  # The system writes its report a moment after the crash.
  sleep 5
  find "$HOME/Library/Logs/DiagnosticReports" -name 'solidcraft-ios*' -newer "$APP/Info.plist" -exec cp {} "$OUT/" \; 2> /dev/null || true
fi
echo "--- stderr.log"
cat "$OUT/stderr.log" 2> /dev/null || true
exit $status
