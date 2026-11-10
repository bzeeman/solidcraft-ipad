#!/usr/bin/env bash
# Wrap the iPad simulator build of SolidCraft (apps/solidcraft-ios, a spike) in an app bundle:
#
#   $DIST/SolidCraft.app        installs with `xcrun simctl install booted SolidCraft.app`, or by
#                               dragging it onto a running Simulator window
#   $DIST/SolidCraft-ios-sim.zip  the same bundle, zipped (ditto keeps the signature)
#
# Usage: packaging/ios/bundle-sim.sh [--profile release|ci|debug] [--skip-build]
#
# Apple silicon only (aarch64-apple-ios-sim). The simulator takes an ad-hoc signature, so this
# needs no Apple Developer account; a build for a real iPad does (not handled here).
set -euo pipefail
DIST="${DIST:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/dist/ios-sim}"
export DIST
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

TARGET=aarch64-apple-ios-sim
PROFILE=release
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "usage: $0 [--profile release|ci|debug] [--skip-build]" >&2; exit 2 ;;
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
rm -rf "$APP" "$DIST/SolidCraft-ios-sim.zip"
mkdir -p "$APP"
cp "$BIN" "$APP/solidcraft-ios"
# CFBundleShortVersionString takes three numbers: a pre-release suffix is dropped.
sed -e "s/@VERSION@/${VERSION%%-*}/" -e "s/@MIN_OS@/$IPHONEOS_DEPLOYMENT_TARGET/" "$ROOT/packaging/ios/Info.plist" > "$APP/Info.plist"
copy_docs "$APP"
if command -v codesign > /dev/null; then
  codesign --force --sign - --timestamp=none "$APP"
else
  warn "codesign not found: the bundle is unsigned and the simulator may refuse it"
fi
if command -v ditto > /dev/null; then
  ditto -c -k --keepParent "$APP" "$DIST/SolidCraft-ios-sim.zip"
fi
echo "$APP"
