#!/usr/bin/env bash
# Sign the iPad build of SolidCraft (apps/solidcraft-ios, a spike) for TestFlight and upload it.
#
# Starts from the unsigned bundle of `packaging/ios/bundle.sh --device`, built with Xcode 26
# (Apple refuses uploads from an older one), and needs a Mac: codesign, security, altool.
#
# With an App Store Connect API key in the environment it
#   1. makes a distribution certificate and an App Store provisioning profile for the bundle's
#      identifier through the API, into a keychain of its own,
#   2. signs the bundle, wraps it in SolidCraft.ipa and uploads that with altool,
#   3. waits until Apple has processed the build,
#   4. revokes the certificate and deletes the profile again. A processed build does not need
#      them (Apple signs what testers install), so nothing about signing outlives the run.
# The key (App Store Connect > Users and Access > Integrations > Team Keys, role App Manager):
#   ASC_KEY_ID      the key's id
#   ASC_ISSUER_ID   the issuer id shown above the list of keys
#   ASC_KEY_P8      the text of the key's .p8 file
# The app's record must exist in App Store Connect (Apps > + > New App, with the bundle's
# identifier): the API cannot create one.
#
# Without the key it is a dry run: the same checks and the same package, signed ad hoc, and no
# call to Apple.
#
# Usage: packaging/ios/testflight.sh [SolidCraft.app]
#        packaging/ios/testflight.sh --cleanup   revoke and delete what an interrupted run left
#   DIST           where the bundle is and SolidCraft.ipa goes (default: dist/ios-device)
#   BUILD_NUMBER   replaces the bundle's CFBundleVersion before signing; Apple takes each number
#                  of a version once
#   PROCESS_WAIT   seconds to wait for Apple's processing (default 1800)
#
# What it makes at Apple is named "SolidCraft spike build ...", and it removes only profiles of
# that name and the certificates they hold: never a certificate or profile made by hand.
#
# It prints to a public log: no name, team id, token or response body is echoed, and GitHub is
# told to blank the certificate's names wherever a tool prints them.
set -euo pipefail
DIST="${DIST:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/dist/ios-device}"
export DIST
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

API="${ASC_API:-https://api.appstoreconnect.apple.com}"
PLISTBUDDY="${PLISTBUDDY:-/usr/libexec/PlistBuddy}"
MADE_NAME="SolidCraft spike build"
MODE=upload
APP="$DIST/SolidCraft.app"
case "${1:-}" in
  --cleanup) MODE=cleanup ;;
  -h | --help) sed -n '2,36p' "${BASH_SOURCE[0]}"; exit 0 ;;
  "") ;;
  *) APP="$1" ;;
esac

die() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$*"; else echo "error: $*" >&2; fi
  exit 1
}
# Have GitHub blank a value in the log from here on.
mask() {
  if [ -n "${GITHUB_ACTIONS:-}" ] && [ "${#1}" -gt 3 ]; then echo "::add-mask::$1"; fi
}

# All three parts of the key, or none of them (a dry run). Pasted values may carry blanks.
ASC_KEY_ID="$(printf '%s' "${ASC_KEY_ID:-}" | tr -d ' \r\n\t')"
ASC_ISSUER_ID="$(printf '%s' "${ASC_ISSUER_ID:-}" | tr -d ' \r\n\t')"
case "${ASC_KEY_ID:+1}${ASC_ISSUER_ID:+1}${ASC_KEY_P8:+1}" in
  111) SIGNED=1 ;;
  "") SIGNED=0 ;;
  *) die "only some of ASC_KEY_ID, ASC_ISSUER_ID and ASC_KEY_P8 are set: it takes all three, or none for a dry run" ;;
esac

WORK="$(mktemp -d)"
chmod 700 "$WORK"
KEYCHAIN="$WORK/signing.keychain-db"
KEY_FILE=""
# Whether this run made a certificate, and whether to leave it (Apple still processing).
MADE=0
KEEP=0

# The key as a PEM file again, however the secret was pasted (on one line, with Windows line ends).
write_key() {
  local body
  mkdir -p "$WORK/private_keys"
  KEY_FILE="$WORK/private_keys/AuthKey_$ASC_KEY_ID.p8"
  body="$(printf '%s' "$ASC_KEY_P8" | sed -e 's/-----[A-Z ]*-----//g' | tr -d ' \r\n\t')"
  {
    echo "-----BEGIN PRIVATE KEY-----"
    printf '%s\n' "$body" | fold -w 64
    echo "-----END PRIVATE KEY-----"
  } > "$KEY_FILE"
  chmod 600 "$KEY_FILE"
  openssl pkey -in "$KEY_FILE" -noout 2> /dev/null || die "ASC_KEY_P8 is not the text of a .p8 key file"
}

b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

# A token for one request, good for ten minutes.
token() {
  local now head claims r s
  now="$(date +%s)"
  head="$(printf '{"alg":"ES256","kid":"%s","typ":"JWT"}' "$ASC_KEY_ID" | b64url)"
  claims="$(printf '{"iss":"%s","iat":%s,"exp":%s,"aud":"appstoreconnect-v1"}' "$ASC_ISSUER_ID" "$now" "$((now + 600))" | b64url)"
  # openssl writes the signature as DER (two integers); a token takes them as 32 bytes each.
  {
    read -r r
    read -r s
  } < <(printf '%s' "$head.$claims" | openssl dgst -sha256 -sign "$KEY_FILE" | openssl asn1parse -inform DER | awk -F: '/INTEGER/ { print $NF }')
  r="${r#"${r%%[!0]*}"}"
  s="${s#"${s%%[!0]*}"}"
  printf '%s.%s.%s' "$head" "$claims" "$(printf '%64s%64s' "$r" "$s" | tr ' ' 0 | xxd -r -p | b64url)"
}

# api METHOD PATH [JSON]: the response body on stdout. A refusal prints Apple's reasons (their
# codes and texts, not the body) and fails.
api() {
  local method="$1" path="$2" out status
  local args=(-sS -X "$method" -H "@$WORK/auth")
  out="$(mktemp "$WORK/api.XXXXXX")"
  printf 'Authorization: Bearer %s\n' "$(token)" > "$WORK/auth"
  if [ $# -ge 3 ]; then args+=(-H 'Content-Type: application/json' --data "$3"); fi
  status="$(curl "${args[@]}" -o "$out" -w '%{http_code}' "$API$path")" || status=000
  case "$status" in
    2*) cat "$out" ;;
    *)
      echo "App Store Connect answered $status to $method ${path%%\?*}" >&2
      jq -r '.errors[]? | "  \(.code): \(.title) \(.detail // "")"' "$out" >&2 2> /dev/null || true
      return 1
      ;;
  esac
}

# Delete the profiles this script made (by their name) and revoke the certificates they hold.
sweep() {
  local listed profile certs cert
  listed="$(api GET "/v1/profiles?filter%5BprofileType%5D=IOS_APP_STORE&fields%5Bprofiles%5D=name&limit=200")" || return 1
  for profile in $(jq -r --arg name "$MADE_NAME" '.data[] | select(.attributes.name | startswith($name)) | .id' <<< "$listed"); do
    certs="$(api GET "/v1/profiles/$profile/certificates?fields%5Bcertificates%5D=certificateType&limit=200")" || return 1
    api DELETE "/v1/profiles/$profile" > /dev/null || return 1
    echo "deleted at Apple: a provisioning profile this script made"
    for cert in $(jq -r '.data[].id' <<< "$certs"); do
      api DELETE "/v1/certificates/$cert" > /dev/null || return 1
      echo "revoked at Apple: its distribution certificate"
    done
  done
}

cleanup() {
  local status=$?
  trap - EXIT
  if [ "$SIGNED" = 1 ] && [ "$MADE" = 1 ] && [ "$KEEP" = 0 ] && [ -n "$KEY_FILE" ]; then
    sweep || warn "could not remove this build's certificate and profile at Apple: the next run does, or packaging/ios/testflight.sh --cleanup"
  fi
  if [ -f "$KEYCHAIN" ]; then security delete-keychain "$KEYCHAIN" 2> /dev/null || true; fi
  rm -rf "$WORK"
  exit "$status"
}
trap cleanup EXIT

if [ "$MODE" = cleanup ]; then
  if [ "$SIGNED" = 0 ]; then
    echo "no API key in the environment: nothing was made at Apple, nothing to remove"
    exit 0
  fi
  write_key
  sweep
  echo "nothing of this script's is left at Apple"
  exit 0
fi

# --- The bundle: is it one Apple would take? -------------------------------------------------
plist() { "$PLISTBUDDY" -c "$1" "$APP/Info.plist"; }
[ -f "$APP/Info.plist" ] || die "$APP is not an app bundle (build one with packaging/ios/bundle.sh --device)"
if [ -n "${BUILD_NUMBER:-}" ]; then plist "Set :CFBundleVersion $BUILD_NUMBER"; fi
BUNDLE_ID="$(plist 'Print :CFBundleIdentifier')"
VERSION_SHORT="$(plist 'Print :CFBundleShortVersionString')"
BUILD="$(plist 'Print :CFBundleVersion')"
EXECUTABLE="$(plist 'Print :CFBundleExecutable')"
echo "bundle: $BUNDLE_ID $VERSION_SHORT ($BUILD)"

sdk="$(plist 'Print :DTSDKName' 2> /dev/null || true)"
case "$sdk" in
  iphoneos*) ;;
  *) die "the bundle is not a device build made on a Mac (its DTSDKName is '${sdk:-missing}')" ;;
esac
sdk_major="${sdk#iphoneos}"
sdk_major="${sdk_major%%.*}"
if [ "$sdk_major" -lt 26 ]; then
  die "built with the iPadOS $sdk_major SDK: Apple takes uploads built with Xcode 26 or later"
fi
if [ "$sdk_major" -gt 26 ]; then
  warn "built with the iPadOS $sdk_major SDK: from 27 on UIKit starts only apps with a scene life cycle, which the window layer (winit 0.30) does not have"
fi
# What the linker wrote into the program must agree: Apple reads that too.
linked="$(xcrun vtool -show-build "$APP/$EXECUTABLE" | awk '$1 == "platform" || $1 == "minos" || $1 == "sdk" { printf "%s %s  ", $1, $2 }')"
echo "program: $linked"
linked_sdk="$(echo "$linked" | sed -n 's/.*sdk \([0-9]*\).*/\1/p')"
if [ "${linked_sdk:-0}" -lt 26 ]; then
  die "the program was linked with an SDK older than 26 (was Xcode 26 selected when cargo built it?)"
fi
info="$(plist Print)"
case "$info" in
  *CFBundleIconName*) ;;
  *) die "the bundle's Info.plist names no app icon (bundle.sh compiles it with Xcode 26's actool)" ;;
esac
[ -f "$APP/Assets.car" ] || die "the bundle has no Assets.car, where the app icon lives (bundle.sh compiles it with Xcode 26's actool)"

package() {
  IPA="$DIST/SolidCraft.ipa"
  rm -rf "$WORK/Payload" "$IPA"
  mkdir "$WORK/Payload"
  ditto "$APP" "$WORK/Payload/SolidCraft.app"
  (cd "$WORK" && zip -qry "$IPA" Payload)
  echo "package: $IPA ($(du -h "$IPA" | awk '{ print $1 }'), $(unzip -l "$IPA" | tail -1 | awk '{ print $2 }') files)"
}

# --- Dry run ---------------------------------------------------------------------------------
if [ "$SIGNED" = 0 ]; then
  echo "dry run: no API key in the environment (ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_P8), so no call to Apple"
  codesign --force --sign - "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  package
  # altool's verbs and options have changed between Xcodes: keep what this one takes in the log,
  # next to the call further down that a dry run cannot make.
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::group::altool of this Xcode"; fi
  xcrun altool --version 2>&1 || true
  xcrun altool --help 2>&1 || true
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::endgroup::"; fi
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "TestFlight: dry run only. $VERSION_SHORT ($BUILD) packaged; the API key is not in this repository's secrets." >> "$GITHUB_STEP_SUMMARY"
  fi
  exit 0
fi

# --- Apple: the app, a certificate, a profile ------------------------------------------------
write_key
found="$(api GET "/v1/apps?filter%5BbundleId%5D=$BUNDLE_ID&fields%5Bapps%5D=bundleId&limit=200")" \
  || die "App Store Connect did not accept the key: are ASC_KEY_ID, ASC_ISSUER_ID and ASC_KEY_P8 those of one team key?"
APP_ID="$(jq -r --arg id "$BUNDLE_ID" '[.data[] | select(.attributes.bundleId == $id)][0].id // empty' <<< "$found")"
[ -n "$APP_ID" ] || die "App Store Connect has no app with the identifier $BUNDLE_ID: create its record (Apps > + > New App), then run this job again"
found="$(api GET "/v1/bundleIds?filter%5Bidentifier%5D=$BUNDLE_ID&fields%5BbundleIds%5D=identifier&limit=200")" \
  || die "the key may not read identifiers: it takes the App Manager role"
BUNDLE_RESOURCE="$(jq -r --arg id "$BUNDLE_ID" '[.data[] | select(.attributes.identifier == $id)][0].id // empty' <<< "$found")"
[ -n "$BUNDLE_RESOURCE" ] || die "the identifier $BUNDLE_ID is not registered (Certificates, Identifiers & Profiles > Identifiers)"
echo "App Store Connect knows the app and its identifier"

# What an interrupted run left behind would count against the team's three certificates.
sweep || die "could not look for what an earlier run left at Apple"

openssl req -new -newkey rsa:2048 -nodes -keyout "$WORK/dist.key" -out "$WORK/dist.csr" -subj "/CN=SolidCraft spike build/C=US" 2> /dev/null
MADE=1
made="$(api POST /v1/certificates "$(jq -n --rawfile csr "$WORK/dist.csr" '{data: {type: "certificates", attributes: {certificateType: "DISTRIBUTION", csrContent: $csr}}}')")" \
  || die "Apple did not issue a distribution certificate (a team has at most three: revoke one under Certificates, Identifiers & Profiles, or the key lacks the App Manager role)"
CERT_ID="$(jq -r '.data.id' <<< "$made")"
jq -r '.data.attributes.certificateContent' <<< "$made" | openssl base64 -d -A > "$WORK/dist.cer"
# The profile comes at once, so that the certificate is never at Apple without the name that
# lets a later run find and revoke it.
made="$(api POST /v1/profiles "$(jq -n --arg name "$MADE_NAME $(date -u +%Y%m%d-%H%M%S)" --arg bundle "$BUNDLE_RESOURCE" --arg cert "$CERT_ID" \
  '{data: {type: "profiles", attributes: {name: $name, profileType: "IOS_APP_STORE"}, relationships: {bundleId: {data: {type: "bundleIds", id: $bundle}}, certificates: {data: [{type: "certificates", id: $cert}]}}}}')")" \
  || {
    api DELETE "/v1/certificates/$CERT_ID" > /dev/null || warn "a distribution certificate made just now is left at Apple without a profile: revoke the newest one by hand"
    die "Apple did not make a provisioning profile"
  }
jq -r '.data.attributes.profileContent' <<< "$made" | openssl base64 -d -A > "$WORK/embedded.mobileprovision"
echo "made at Apple: a distribution certificate and an App Store profile for this build"

# The log is public: blank the names the certificate carries (the person or company, the team).
while IFS= read -r line; do
  mask "${line#*=}"
done < <(openssl x509 -inform DER -in "$WORK/dist.cer" -noout -subject -nameopt sep_multiline,utf8 | sed -e 1d -e 's/^ *//')

# --- A keychain of this run's own ------------------------------------------------------------
keychain_password="$(openssl rand -hex 16)"
security create-keychain -p "$keychain_password" "$KEYCHAIN"
security set-keychain-settings -lut 7200 "$KEYCHAIN"
security unlock-keychain -p "$keychain_password" "$KEYCHAIN"
openssl x509 -inform DER -in "$WORK/dist.cer" -out "$WORK/dist.pem"
# macOS's own openssl: `security` does not read the PKCS#12 that OpenSSL 3 writes by default.
/usr/bin/openssl pkcs12 -export -inkey "$WORK/dist.key" -in "$WORK/dist.pem" -out "$WORK/dist.p12" -passout "pass:$keychain_password"
security import "$WORK/dist.p12" -k "$KEYCHAIN" -P "$keychain_password" -T /usr/bin/codesign > /dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$KEYCHAIN" > /dev/null
# codesign finds an identity through the user's list of keychains.
listed=()
while IFS= read -r line; do
  listed+=("$line")
done < <(security list-keychains -d user | sed -e 's/^ *"//' -e 's/"$//')
security list-keychains -d user -s "$KEYCHAIN" ${listed[@]+"${listed[@]}"}
SIGNER="$(openssl x509 -inform DER -in "$WORK/dist.cer" -noout -fingerprint -sha1 | sed -e 's/.*=//' -e 's/://g')"
# (Into a variable first: the listing names the signer, and a public log must not.)
valid() {
  local identities
  identities="$(security find-identity -v -p codesigning "$KEYCHAIN")"
  case "$identities" in *"$SIGNER"*) return 0 ;; *) return 1 ;; esac
}
if ! valid; then
  # The Mac lacks the certificate of Apple's authority that issued ours (its generation, G3 or
  # later, is in the issuer's name).
  generation="$(openssl x509 -inform DER -in "$WORK/dist.cer" -noout -issuer | sed -n 's/.*OU *= *\(G[0-9]*\).*/\1/p')"
  curl -sSf -o "$WORK/issuer.cer" "https://www.apple.com/certificateauthority/AppleWWDRCA$generation.cer" \
    && security import "$WORK/issuer.cer" -k "$KEYCHAIN" > /dev/null
  valid || die "the keychain holds no valid signing identity for the new certificate"
fi

# --- Sign, package, upload -------------------------------------------------------------------
security cms -D -i "$WORK/embedded.mobileprovision" -o "$WORK/profile.plist" 2> /dev/null
"$PLISTBUDDY" -x -c 'Print :Entitlements' "$WORK/profile.plist" > "$WORK/entitlements.plist"
# The profile allows every keychain group of the team (a wildcard). An app claims that only when
# it shares a keychain, and Apple's validation refuses the wildcard in a signature.
"$PLISTBUDDY" -c 'Delete :keychain-access-groups' "$WORK/entitlements.plist" > /dev/null 2>&1 || true
mask "$("$PLISTBUDDY" -c 'Print :TeamIdentifier:0' "$WORK/profile.plist")"
if ! grep -q beta-reports-active "$WORK/entitlements.plist"; then
  warn "the profile grants no beta-reports-active: TestFlight may not offer the build"
fi
cp "$WORK/embedded.mobileprovision" "$APP/embedded.mobileprovision"
codesign --force --sign "$SIGNER" --keychain "$KEYCHAIN" --entitlements "$WORK/entitlements.plist" --generate-entitlement-der "$APP"
codesign --verify --strict --verbose=2 "$APP"
package

export API_PRIVATE_KEYS_DIR="$WORK/private_keys"
# altool as of Xcode 26 (its usage, read from Xcode 26.3 on a GitHub Mac): the file follows the
# verb, and the app is named by its Apple ID (the id of its record in the API), its identifier
# and both version numbers. It finds the key file by the key's id in API_PRIVATE_KEYS_DIR.
upload() {
  local status=0
  xcrun altool "$1" "$IPA" --platform ios --apple-id "$APP_ID" --bundle-id "$BUNDLE_ID" \
    --bundle-version "$BUILD" --bundle-short-version-string "$VERSION_SHORT" \
    --api-key "$ASC_KEY_ID" --api-issuer "$ASC_ISSUER_ID" 2>&1 | tee "$WORK/altool.log" || status=$?
  # altool has answered an error with a zero status before.
  if [ "$status" != 0 ] || grep -q -E 'ERROR|Failed to' "$WORK/altool.log"; then return 1; fi
}
upload --validate-app || die "Apple's validation refused the package (its reasons are above)"
upload --upload-package || die "the upload failed (the reasons are above)"
echo "uploaded: $VERSION_SHORT ($BUILD)"

# --- Wait for Apple's processing, then take the certificate back ------------------------------
deadline=$(($(date +%s) + ${PROCESS_WAIT:-1800}))
while :; do
  found="$(api GET "/v1/builds?filter%5Bapp%5D=$APP_ID&filter%5Bversion%5D=$BUILD&filter%5BpreReleaseVersion.version%5D=$VERSION_SHORT&fields%5Bbuilds%5D=processingState&limit=1")" || found='{}'
  state="$(jq -r '.data[0].attributes.processingState // "not listed yet"' <<< "$found")"
  echo "  Apple's processing: $state"
  case "$state" in
    VALID) break ;;
    FAILED | INVALID) die "Apple could not process the build (its e-mail to the account says why)" ;;
  esac
  if [ "$(date +%s)" -ge "$deadline" ]; then
    KEEP=1
    warn "Apple was still processing after ${PROCESS_WAIT:-1800} s: this build's certificate and profile stay until the next run, or packaging/ios/testflight.sh --cleanup"
    break
  fi
  sleep 30
done
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "TestFlight: $VERSION_SHORT ($BUILD) uploaded; Apple's processing: $state." >> "$GITHUB_STEP_SUMMARY"
fi
echo "done: $VERSION_SHORT ($BUILD) is in App Store Connect ($state)"
