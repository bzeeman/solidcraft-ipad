#!/usr/bin/env bash
# Wrap the iPad build of SolidCraft (apps/solidcraft-ios, a spike) in an app bundle.
#
# For the simulator (the default; Apple silicon only, aarch64-apple-ios-sim):
#   $DIST/SolidCraft.app          installs with `xcrun simctl install booted SolidCraft.app`, or
#                                 by dragging it onto a running Simulator window
#   $DIST/SolidCraft-ios-sim.zip  the same bundle, zipped (ditto keeps the signature)
# The simulator takes an ad-hoc signature, so this needs no Apple Developer account.
#
# For a real iPad (--device, aarch64-apple-ios):
#   $DIST/SolidCraft.app             NOT signed: an iPad installs it only once it is signed with
#                                    a development or distribution identity and carries that
#                                    identity's provisioning profile
#   $DIST/SolidCraft-ios-device.zip  the same bundle, zipped
#
# Usage: packaging/ios/bundle.sh [--device] [--profile release|ci|debug] [--skip-build]
#   DIST          where the bundle goes (default: dist/ios-sim, or dist/ios-device with --device)
#   BUILD_NUMBER  CFBundleVersion (default 1); Apple takes each number of a version once
#
# On a Mac the bundle also says which Xcode and SDK built it (the DT* keys of Info.plist) and
# carries the app icon, compiled from the macOS master (assets/icons/app/macos/source), which is
# drawn for both. Apple's upload checks want both, and the icon takes Xcode 26 or later: an older
# Xcode leaves the bundle without one, with a warning.
set -euo pipefail
DEVICE=0
for arg in "$@"; do
  if [ "$arg" = --device ]; then DEVICE=1; fi
done
if [ "$DEVICE" = 1 ]; then KIND=device; else KIND=sim; fi
DIST="${DIST:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/dist/ios-$KIND}"
export DIST
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

if [ "$DEVICE" = 1 ]; then
  TARGET=aarch64-apple-ios
  PLATFORM=iPhoneOS
else
  TARGET=aarch64-apple-ios-sim
  PLATFORM=iPhoneSimulator
fi
PROFILE=release
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --device) shift ;;
    --profile) PROFILE="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "usage: $0 [--device] [--profile release|ci|debug] [--skip-build]" >&2; exit 2 ;;
  esac
done
# The oldest iPadOS the binary runs on; the linker and Info.plist must agree.
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"

# cargo names the dev profile's directory "debug".
case "$PROFILE" in
  debug | dev) CARGO_PROFILE=dev; PROFILE_DIR=debug ;;
  *) CARGO_PROFILE="$PROFILE"; PROFILE_DIR="$PROFILE" ;;
esac
if [ "$SKIP_BUILD" = 0 ]; then
  cargo build --locked --profile "$CARGO_PROFILE" -p solidcraft-ios --target "$TARGET"
fi
BIN="$CARGO_TARGET_DIR/$TARGET/$PROFILE_DIR/solidcraft-ios"
if [ ! -f "$BIN" ]; then
  echo "error: $BIN is missing (build with: cargo build --profile $CARGO_PROFILE -p solidcraft-ios --target $TARGET)" >&2
  exit 1
fi

APP="$DIST/SolidCraft.app"
ZIP="$DIST/SolidCraft-ios-$KIND.zip"
rm -rf "$APP" "$ZIP"
mkdir -p "$APP"
cp "$BIN" "$APP/solidcraft-ios"
# CFBundleShortVersionString takes three numbers: a pre-release suffix is dropped.
SDK="$(echo "$PLATFORM" | tr '[:upper:]' '[:lower:]')"
sed -e "s/@VERSION@/${VERSION%%-*}/" -e "s/@BUILD@/${BUILD_NUMBER:-1}/" -e "s/@MIN_OS@/$IPHONEOS_DEPLOYMENT_TARGET/" \
  -e "s/@PLATFORM@/$PLATFORM/" -e "s/@PLATFORM_NAME@/$SDK/" \
  "$ROOT/packaging/ios/Info.plist" > "$APP/Info.plist"
copy_docs "$APP"
PLISTBUDDY=/usr/libexec/PlistBuddy
if [ -x "$PLISTBUDDY" ] && command -v xcrun > /dev/null; then
  sdk_version="$(xcrun --sdk "$SDK" --show-sdk-version)"
  sdk_build="$(xcrun --sdk "$SDK" --show-sdk-build-version)"
  xcode_version="$(xcodebuild -version | awk 'NR == 1 { print $2 }')"
  xcode_build="$(xcodebuild -version | awk 'NR == 2 { print $3 }')"
  # DTXcode is the version in four digits: 26.1 is 2610, 26.0.1 is 2601.
  dt_xcode="$(echo "$xcode_version" | awk -F. '{ printf "%02d%d%d", $1, $2, $3 }')"
  add() { "$PLISTBUDDY" -c "Add :$1 string $2" "$APP/Info.plist"; }
  add BuildMachineOSBuild "$(sw_vers -buildVersion)"
  add DTCompiler com.apple.compilers.llvm.clang.1_0
  add DTPlatformBuild "$sdk_build"
  add DTPlatformVersion "$sdk_version"
  add DTSDKBuild "$sdk_build"
  add DTSDKName "$SDK$sdk_version"
  add DTXcode "$dt_xcode"
  add DTXcodeBuild "$xcode_build"
  echo "built with Xcode $xcode_version ($xcode_build), SDK $SDK$sdk_version ($sdk_build)"

  # The icon: Assets.car, the loose images an older iPadOS reads, and the Info.plist keys that
  # name them (actool writes those to a file of their own, merged in here).
  icon_plist="$(mktemp -d)/icon.plist"
  if xcrun actool "$ROOT/assets/icons/app/macos/source/SolidCraft.icon" --compile "$APP" \
    --platform "$SDK" --minimum-deployment-target "$IPHONEOS_DEPLOYMENT_TARGET" --target-device ipad \
    --app-icon SolidCraft --output-partial-info-plist "$icon_plist" --output-format human-readable-text; then
    if [ -s "$icon_plist" ]; then "$PLISTBUDDY" -c "Merge $icon_plist" "$APP/Info.plist"; fi
  else
    warn "actool did not compile the app icon (it takes Xcode 26 or later; this is $xcode_version): the bundle has no icon"
  fi
else
  warn "not on a Mac with Xcode: the bundle has no icon and does not say what built it"
fi
if [ "$DEVICE" = 1 ]; then
  echo "note: $APP is not signed; an iPad installs it only once it is"
elif command -v codesign > /dev/null; then
  codesign --force --sign - --timestamp=none "$APP"
else
  warn "codesign not found: the bundle is unsigned and the simulator may refuse it"
fi
if command -v ditto > /dev/null; then
  ditto -c -k --keepParent "$APP" "$ZIP"
fi
echo "$APP"
