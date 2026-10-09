# Configuration

Flick reads `~/.config/flick/config.toml` and writes a commented default on first run. Set `FLICK_CONFIG` to use a different path. Run **Reload Flick Config** from the launcher after you edit it.

Every option, with its default and a one-line note, is in [`config.example.toml`](../config.example.toml). `flick config example` prints that file, so you can read it also when home-manager or another tool writes your config. Delete the `#` in front of a setting to use it.

```toml
hotkey = "ctrl+Space"              # launcher

[switcher]
hotkey = "cmd+Space"               # window switcher

[desktop]
hotkey = "cmd+Backquote"           # most recent app on another desktop

[window.keys]
# Hyper (cmd+ctrl+alt+shift): Caps Lock with [keys] hyper, see Key triggers
left-half = "cmd+ctrl+alt+shift+KeyH"
right-half = "cmd+ctrl+alt+shift+KeyL"
maximize = "cmd+ctrl+alt+shift+KeyK"
hide = "cmd+ctrl+alt+shift+KeyJ"
next-display = "cmd+ctrl+alt+shift+KeyN"
previous-display = "cmd+ctrl+alt+shift+KeyP"

[[quicklink.links]]
name = "GitHub Search"
keyword = "gh"
url = "https://github.com/search?q={query}&type=repositories"

[[quicklink.links]]
name = "Projects"
url = "~/Projects"
```

Each module reads its own table: `app`, `desktop`, `switcher`, `window`, `quicklink`, `script`, `builtin`, `clip`, `activity`, `herdr`, `task`, `keys`, `capture`, `feedback`, `message`, `flick`, `remote`, `help`. Set `enabled = false` in a table to turn that module off. Config files from older versions keep working: the flat keys `windows_hotkey`, `desktop_toggle`, `[window_keys]` and `[[quicklinks]]` still apply.

Hotkeys use `cmd`, `alt`, `ctrl`, and `shift` with key names such as `Space`, `KeyA`, `Digit1`, `ArrowLeft`, and `Backquote`. Window action names are the command titles in kebab case: `top-left-quarter`, `first-two-thirds`, `almost-maximize`.

> [!WARNING]
> A global hotkey overrides that shortcut in every app. For example, `cmd+KeyL` would replace the browser address bar. A Hyper key (`cmd+ctrl+alt+shift`) avoids conflicts.

Each module's settings are in its own page: [script commands](scripts.md), [keys](keys.md), [activity](activity.md), [tasks](tasks.md), [herdr](herdr.md), [capture](capture.md), [feedback](feedback.md), [messages](message.md), [remote](remote.md), and [rebuild](install.md#rebuild-settings).

## Quicklinks

### Create and edit

Run **Create Quicklink** from root search to add a link with a form. Select a quicklink and press ⌘K to edit or delete it. From a shell: `flick quicklink add <name> <url> [--keyword k] [--app a]`, `flick quicklink remove <name>` and `flick quicklink list`. Flick writes the change to config.toml and keeps all other text in the file, comments included. The change applies at once, without a reload. A renamed link loses its usage history.

### Import from Raycast

In Raycast, run **Export Quicklinks**. Then:

```bash
~/Applications/Flick.app/Contents/MacOS/Flick import-raycast ~/Downloads/Quicklinks*.json
```

Flick appends the links to the config file as `[[quicklink.links]]` entries. It changes `{argument}` placeholders to `{query}` and skips links with the same name or URL as an existing link. It reports placeholders that it cannot fill, such as `{clipboard}`.

## Launcher keys

| Key | Action |
|---|---|
| ↑ ↓, ⌃P ⌃N | Move the selection |
| ↵ | Run the selected item |
| ⇥ | Enter a quicklink argument |
| ⎋ | Go back, or close |
| ⌫ in an empty field | Go back |
