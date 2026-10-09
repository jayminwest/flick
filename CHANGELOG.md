# Changelog

## Unreleased

- Minimize window action.
- `flick snapshot` renders the launcher to a PNG without showing it.
- Fix: a second Flick started by launchd no longer runs next to an existing one.

## 0.0.1 — 2026-10-09

First release.

- Launcher: fuzzy app and command search with frecency ranking.
- Window management: 18 window actions, global hotkeys through `[window_keys]`, size cycling on repeated halves.
- Window switcher: fuzzy search over open windows; apps on other desktops get one entry.
- Desktop toggle: jump to the most recent app on another desktop.
- Clipboard history: last 500 text clips, paste into the previous app, skips concealed content.
- Quicklinks: URLs and paths with `{query}` arguments and keywords; `flick import-raycast`.
- `scripts/bundle.sh`: builds and signs `Flick.app` with a stable identity when one exists.
