#!/usr/bin/env bash
# Build Flick.app (release) into $CARGO_TARGET_DIR (default target/).
# --install [relaunch.sh options]: then install and restart it with scripts/relaunch.sh.
#
# The build stamp goes to cargo as FLICK_BUILD_SHA (full sha), FLICK_BUILD_DIRTY (1/0),
# FLICK_BUILD_TIME (RFC 3339 UTC) and FLICK_BUILD_SOURCE (checkout path). Each one
# defaults to the value of this checkout; a caller can set it first (an exported tree
# has no .git). FLICK_CARGO_ARGS adds cargo build flags, e.g. "--locked --offline".
#
# Every failure exits non-zero before the bundle exists, so a failed or missing cargo
# never installs or relaunches anything (flick-d4b3). Keep it bash 3.2 compatible.
if [ -z "${BASH_VERSION:-}" ]; then exec /bin/bash "$0" "$@"; fi
set -euo pipefail
trap 'echo "bundle.sh: failed (exit $?) at line $LINENO; nothing installed" >&2' ERR
cd "$(dirname "$0")/.."

die() {
  trap - ERR
  echo "bundle.sh: $*; nothing installed" >&2
  exit 1
}

install=false
if [[ "${1:-}" == "--install" ]]; then
  install=true
  shift
elif [[ $# -gt 0 ]]; then
  echo "usage: scripts/bundle.sh [--install [relaunch.sh options]]" >&2
  exit 2
fi

if [[ -z "${FLICK_BUILD_SHA:-}" ]]; then
  FLICK_BUILD_SHA="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
fi
if [[ -z "${FLICK_BUILD_DIRTY:-}" ]]; then
  FLICK_BUILD_DIRTY=0
  if [[ -n "$(git status --porcelain 2>/dev/null || true)" ]]; then FLICK_BUILD_DIRTY=1; fi
fi
FLICK_BUILD_TIME="${FLICK_BUILD_TIME:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
FLICK_BUILD_SOURCE="${FLICK_BUILD_SOURCE:-$(pwd -P)}"
export FLICK_BUILD_SHA FLICK_BUILD_DIRTY FLICK_BUILD_TIME FLICK_BUILD_SOURCE
echo "stamp: $FLICK_BUILD_SHA dirty=$FLICK_BUILD_DIRTY $FLICK_BUILD_TIME $FLICK_BUILD_SOURCE"

target="${CARGO_TARGET_DIR:-target}"
app="$target/Flick.app"
# Remove the old bundle first: a failed build must not leave one for a caller to install.
rm -rf "$app"

# rustup's cargo proxy is on PATH even with no toolchain; check that cargo runs.
command -v cargo >/dev/null 2>&1 || die "cargo not found: install Rust with rustup (https://rustup.rs)"
cargo --version >/dev/null 2>&1 || die "cargo does not run (no Rust toolchain?): run rustup default stable"
# FLICK_CARGO_ARGS is split on spaces on purpose.
# shellcheck disable=SC2086
if ! cargo build --release ${FLICK_CARGO_ARGS:-}; then
  die "cargo build failed"
fi
[[ -x "$target/release/flick" ]] || die "cargo made no binary at $target/release/flick"
version="$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)"
bundle_version="$version+${FLICK_BUILD_SHA:0:7}"
if [[ "$FLICK_BUILD_DIRTY" == 1 ]]; then bundle_version="$bundle_version-dirty"; fi
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/flick" "$app/Contents/MacOS/Flick"
cp assets/icon/Flick.icns "$app/Contents/Resources/Flick.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Flick</string>
  <key>CFBundleIdentifier</key><string>com.jayminwest.flick</string>
  <key>CFBundleExecutable</key><string>Flick</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>Flick</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$bundle_version</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSAppleEventsUsageDescription</key><string>With [activity] urls = true, Flick reads the front tab URL of your browser to record it in activity spans on this Mac.</string>
</dict>
</plist>
PLIST
# A stable identity keeps macOS privacy grants (Accessibility) across rebuilds;
# ad-hoc signatures change every build, so macOS would ask again each time.
identity="${FLICK_SIGN_IDENTITY:-$({ security find-identity -v -p codesigning 2>/dev/null || true; } | awk -F'"' '/Apple Development/ {print $2; exit}')}"
if [[ -z "$identity" ]]; then
  identity=-
  echo "warning: no Apple Development identity; ad-hoc signing (Accessibility resets on rebuild)" >&2
fi
codesign --force --sign "$identity" "$app"
echo "built $app ($bundle_version, signed: $identity)"

if $install; then
  exec scripts/relaunch.sh --install "$app" "$@"
fi
