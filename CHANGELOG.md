# Changelog

## Unreleased

- **Messages:** new `message` module. `flick message post [--title] [--url] [--reply-to] [--pending] <body>`, locally or from a remote peer, shows a corner card that does not take focus (click opens the link, Esc or a timeout dismisses it), a notification, or both; **Messages** in the launcher lists the history. `[message]` sets `name`, `style`, `position`, `width`, `timeout_secs`, `max_history`, `sound` and `hotkey`. A `--pending` post is replaced by its `--reply-to` reply. [Messages](docs/message.md).
- **Script commands:** new `[[script.commands]]` table. Each command is a root item that runs `shell` with `/bin/sh -c`; one with `{query}` takes an argument, typed as `<keyword> <text>` and passed as one single-quoted word. A failed command posts a notification. [Script commands](docs/scripts.md).

## 0.0.2 — 2026-10-09

New macOS permissions: capture needs **Screen Recording**. Activity URLs need **Automation** for each browser. The key tap and activity window titles need **Accessibility**. Notifications (herdr) ask on first launch.

Config moved to one `[<module>]` table per feature, each with `enabled = false` to turn it off. Old keys (`windows_hotkey`, `desktop_toggle`, `[window_keys]`, `[[quicklinks]]`) still load.

- **Launcher:** cmd+K action menus on items, forms, and confirm screens for destructive actions.
- **Windows:** Minimize and Hide actions. Hide works like ⌘H, with no animation.
- **Apps:** Show in Finder, Quit and Force Quit on app items; a Quit Applications view of running apps; Uninstall… moves the app and its leftovers to the Trash after a confirm (`flick app uninstall --dry-run|--yes`). Apps rescan on wake.
- **Quicklinks:** Create Quicklink form, Edit and Delete in cmd+K, `flick quicklink add|remove|list`. Edits keep the comments in config.toml. A link can name an `app` to open it with. Duplicate names are rejected at load.
- **Key triggers:** new `[keys]` table. Caps Lock as Hyper (with a tap key), and `[[keys.chord]]` push-to-talk chords that run an `http` or `shell` action on key down and up. `flick keys list|status|fire`. Needs Accessibility.
- **Activity:** opt-in, local-only time tracking with `[activity]` rules, a menu bar indicator, Activity Today, and `flick activity today|week|spans|forget`. Optional window titles (`titles`) and browser tab URLs (`urls`, Brave/Chrome/Edge/Chromium). Spans record the running task (`--by task`).
- **Tasks:** a short task list with one running timer. Start Task, Switch Task, Tasks Today; `flick task start|switch|stop|ls|add|done|report`.
- **Herdr:** Herdr Agents view of coding agents on this Mac and herdr SSH machines, blocked first. ↵ jumps to the pane; notifications on `blocked`/`done` (`[herdr] notify`). `flick herdr ls|jump|status`.
- **Capture:** area, window and screen screenshots to file and clipboard, an annotation editor (arrow, box, pen, highlighter, text, redact), Draw on Screen and Highlight Cursor. Recent Captures. `flick capture ...`. Needs Screen Recording.
- **Feedback:** Add Feedback…, `fb <text>` and `flick feedback add|ls|path` append notes to `feedback.jsonl`.
- **Remote access:** `[remote]` lets named Tailscale peers send commands; off by default, `flick remote on|off|status`. `flick --host <mac>` / `FLICK_HOST` send commands to another Mac's Flick. Peers cannot reload, rebuild, uninstall, capture or change config.
- **Rebuild:** Flick Version and Rebuild Available items, Rebuild Flick from the checkout in the background (`[flick] source`, `gates`), `flick flick rebuild|status|cancel|version`.
- **Command line:** `flick <module> <verb>` over the control socket, `--json` replies as `{"ok":...}`, and `flick events` streams events as JSON lines.
- `flick snapshot` renders the launcher to a PNG without showing it.
- Fix: a second Flick started by launchd no longer runs next to an existing one.
- Fix: a Caps Lock remap left by a crash is cleared when `hyper` is no longer `caps_lock`.
- Fix: a config reload that enables a module now starts it; the launcher hotkey printed at start is the one actually bound.

## 0.0.1 — 2026-10-09

First release.

- Launcher: fuzzy app and command search with frecency ranking.
- Window management: 18 window actions, global hotkeys through `[window_keys]`, size cycling on repeated halves.
- Window switcher: fuzzy search over open windows; apps on other desktops get one entry.
- Desktop toggle: jump to the most recent app on another desktop.
- Clipboard history: last 500 text clips, paste into the previous app, skips concealed content.
- Quicklinks: URLs and paths with `{query}` arguments and keywords; `flick import-raycast`.
- `scripts/bundle.sh`: builds and signs `Flick.app` with a stable identity when one exists.
