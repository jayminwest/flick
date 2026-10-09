# Flick

A small, fast launcher for macOS. Rust + AppKit (objc2), no web views, no runtime.

- **Apps**: fuzzy search; ranking learns from use
- **Window management**: halves, quarters, thirds, maximize, center, next display
- **Clipboard history**: text only, last 500 clips; skips password-manager content
- **Quicklinks**: URLs or paths, `{query}` arguments, `<keyword> <text>` from the root

## Run

    cargo run                      # dev
    ./scripts/bundle.sh --install  # build ~/Applications/Flick.app and launch it

Default hotkey: `alt+shift+Space`. Window management and auto-paste need
Accessibility permission (System Settings → Privacy & Security → Accessibility).
The bundle is ad-hoc signed, so macOS may ask for that permission again after a rebuild.

## Keys

| Key | Action |
|---|---|
| ↑ ↓ / ctrl-p ctrl-n | Move selection |
| ↵ | Run |
| tab | Enter a quicklink argument |
| esc | Back / close |
| ⌫ on empty field | Back |

## Config

`~/.config/flick/config.toml` (created on first run). Run **Reload Flick Config** after edits.
Data (usage, clipboard) lives in `~/Library/Application Support/Flick/flick.db`.

## Layout

| File | Role |
|---|---|
| `search.rs` | Items, fuzzy ranking (nucleo), frecency — pure |
| `windows.rs` | Window geometry (pure) + Accessibility FFI |
| `store.rs` | SQLite: usage + clipboard |
| `config.rs` | TOML config, quicklinks |
| `apps.rs` | App index |
| `ui.rs` | NSPanel, search field, rows |
| `app.rs` | Controller: modes, actions |
| `hotkey.rs` | Global hotkey |
