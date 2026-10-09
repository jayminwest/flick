#!/usr/bin/env bash
# Install a built Flick.app and restart Flick. bundle.sh --install and the in-app
# rebuild both use this script, so there is one installer.
#
#   scripts/relaunch.sh [--install <Flick.app>] [--wait-pid PID] [--no-restart]
#
# --install    Copy the bundle to $FLICK_INSTALL_DIR/.Flick.app.new (default
#              ~/Applications), verify its signature, then swap it in with two renames.
#              A failed copy or verify leaves the installed app as it was.
# --wait-pid   A Flick process to stop (the in-app caller passes its own pid). Every
#              other running Flick is stopped too.
# --no-restart Install only; do not stop or start Flick.
#
# After an install, link the CLI: $FLICK_CLI_DIR/flick (default ~/.local/bin/flick) points
# at the app's binary. A `flick` there that is not a symlink to some Flick.app is left
# alone; FLICK_CLI_DIR= (empty) skips the link.
#
# Restart: stop each Flick and wait until it exits (10 s, then kill -9), so the new copy
# does not see an old one and exit as "already running". Then start Flick through the
# launchd agent when it exists (launchctl kickstart -k), else with open.
if [ -z "${BASH_VERSION:-}" ]; then exec /bin/bash "$0" "$@"; fi
set -euo pipefail

usage() {
  echo "usage: scripts/relaunch.sh [--install <Flick.app>] [--wait-pid PID] [--no-restart]" >&2
  exit 2
}

src=""
pids=""
restart=true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --install) [[ $# -ge 2 ]] || usage; src="$2"; shift 2 ;;
    --wait-pid) [[ "${2:-}" =~ ^[0-9]+$ ]] || usage; pids="$2"; shift 2 ;;
    --no-restart) restart=false; shift ;;
    *) usage ;;
  esac
done

# The `flick` command: a symlink to the installed binary, never over a file we did not make.
link_cli() {
  local bin_dir="${FLICK_CLI_DIR-$HOME/.local/bin}"
  [[ -n "$bin_dir" ]] || return 0
  local target="$app/Contents/MacOS/Flick" link="$bin_dir/flick"
  if [[ -L "$link" ]]; then
    case "$(readlink "$link")" in
      */Flick.app/Contents/MacOS/Flick) ;;
      *) echo "relaunch: $link links elsewhere; CLI not linked" >&2; return 0 ;;
    esac
  elif [[ -e "$link" ]]; then
    echo "relaunch: $link exists and is not a symlink; CLI not linked" >&2
    return 0
  fi
  mkdir -p "$bin_dir"
  ln -sfn "$target" "$link"
  echo "relaunch: linked $link -> $target"
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "relaunch: $bin_dir is not on PATH; add it to run flick" >&2 ;;
  esac
}

dir="${FLICK_INSTALL_DIR:-$HOME/Applications}"
app="$dir/Flick.app"
new="$dir/.Flick.app.new"
old="$dir/.Flick.app.old"

# A crash between the two renames below leaves only .Flick.app.old; put it back.
if [[ ! -e "$app" && -d "$old" ]]; then
  mv "$old" "$app"
  echo "relaunch: restored $app from $old"
fi

if [[ -n "$src" ]]; then
  [[ -d "$src" ]] || { echo "relaunch: no bundle at $src" >&2; exit 1; }
  mkdir -p "$dir"
  rm -rf "$new"
  ditto "$src" "$new"
  if ! codesign --verify --strict "$new"; then
    rm -rf "$new"
    echo "relaunch: $src fails codesign --verify; $app not changed" >&2
    exit 1
  fi
  # Renames on one volume are atomic, and renaming a running bundle is safe on macOS.
  rm -rf "$old"
  [[ -e "$app" ]] && mv "$app" "$old"
  mv "$new" "$app"
  rm -rf "$old"
  echo "relaunch: installed $app"
  link_cli
fi

$restart || exit 0

# Stop every copy, including one opened by hand next to the launchd agent's.
pids="$pids $(pgrep -x Flick || true)"
for pid in $pids; do kill "$pid" 2>/dev/null || true; done
for pid in $pids; do
  n=0
  while kill -0 "$pid" 2>/dev/null && [[ $n -lt 120 ]]; do
    if [[ $n -eq 100 ]]; then
      echo "relaunch: pid $pid did not exit in 10 s; kill -9" >&2
      kill -9 "$pid" 2>/dev/null || true
    fi
    sleep 0.1
    n=$((n + 1))
  done
done

# A launchd agent (e.g. home-manager's launchd.agents.flick) owns the process when
# present; start through it so there is only ever one Flick.
agent="gui/$(id -u)/org.nix-community.home.flick"
if launchctl print "$agent" >/dev/null 2>&1; then
  launchctl kickstart -k "$agent"
  echo "relaunch: restarted $agent"
else
  open "$app"
  echo "relaunch: opened $app"
fi
