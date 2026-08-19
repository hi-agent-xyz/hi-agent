#!/usr/bin/env bash
#
# Verify that every committed version stamp agrees with the root VERSION file.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(tr -d '\r\n' < VERSION)"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?$ ]] || {
  echo "error: VERSION must look like X.Y.Z (got '$VERSION')" >&2
  exit 1
}

failed=false
check() {
  local label="$1" actual="$2"
  if [ "$actual" != "$VERSION" ]; then
    echo "error: $label is $actual, expected $VERSION (run 'make bump-version V=$VERSION')" >&2
    failed=true
  fi
}

check "Cargo.toml package version" \
  "$(awk -F'"' '/^version *=/ { print $2; exit }' Cargo.toml)"
check "Cargo.lock hi-agent version" \
  "$(awk -F'"' 'prev == "name = \"hi-agent\"" && /^version *=/ { print $2; exit } { prev=$0 }' Cargo.lock)"
check "Info.plist short version" \
  "$(awk '/CFBundleShortVersionString/ { line=$0; sub(/^.*<string>/, "", line); sub(/<\/string>.*$/, "", line); print line; exit }' scripts/Info.plist)"
check "Info.plist bundle version" \
  "$(awk '/CFBundleVersion/ { line=$0; sub(/^.*<string>/, "", line); sub(/<\/string>.*$/, "", line); print line; exit }' scripts/Info.plist)"
check "iOS marketing version" \
  "$(awk -F= '/MARKETING_VERSION =/ { value=$2; gsub(/[;[:space:]]/, "", value); print value }' \
    app/apple/ios/HiAgentIOS.xcodeproj/project.pbxproj | sort -u | paste -sd, -)"
check "web package version" \
  "$(awk -F'"' '/"version":/ { print $4; exit }' src/appearance/web/package.json)"
check "web lock package version" \
  "$(awk -F'"' '/"version":/ { print $4; exit }' src/appearance/web/package-lock.json)"
check "web lock root version" \
  "$(awk -F'"' '/"version":/ { seen++; if (seen == 2) { print $4; exit } }' src/appearance/web/package-lock.json)"

$failed && exit 1
echo "version $VERSION is consistent"
