#!/usr/bin/env bash
# Build Flick.app (release) into target/, or install it with --install.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release
app=target/Flick.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/flick "$app/Contents/MacOS/Flick"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Flick</string>
  <key>CFBundleIdentifier</key><string>com.jayminwest.flick</string>
  <key>CFBundleExecutable</key><string>Flick</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"
echo "built $app"

if [[ "${1:-}" == "--install" ]]; then
  # A launchd agent (e.g. home-manager's launchd.agents.flick) owns the process
  # when present; restart through it so there is only ever one Flick.
  agent="gui/$(id -u)/org.nix-community.home.flick"
  launchd=false
  launchctl print "$agent" >/dev/null 2>&1 && launchd=true
  $launchd || pkill -x Flick || true
  rm -rf ~/Applications/Flick.app
  mkdir -p ~/Applications
  cp -R "$app" ~/Applications/
  if $launchd; then
    launchctl kickstart -k "$agent"
  else
    open ~/Applications/Flick.app
  fi
  echo "installed ~/Applications/Flick.app"
fi
