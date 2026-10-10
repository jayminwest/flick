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

Each module reads its own table: `app`, `desktop`, `switcher`, `window`, `quicklink`, `script`, `builtin`, `clip`, `activity`, `herdr`, `kota`, `llm`, `sys`, `task`, `keys`, `dictation`, `capture`, `feedback`, `message`, `flick`, `remote`, `help`. Set `enabled = false` in a table to turn that module off. `[launcher]` is the launcher's own table (see [Launcher appearance](#launcher-appearance)). Config files from older versions keep working: the flat keys `windows_hotkey`, `desktop_toggle`, `[window_keys]` and `[[quicklinks]]` still apply.

## Per-host overlay

When one config.toml is shared between Macs (home-manager links the same file onto each), put the settings that belong to one Mac in that Mac's overlay: `config.<host>.toml` in the same directory as config.toml (the directory of `FLICK_CONFIG` when set). `<host>` is the Mac's `LocalHostName` (`scutil --get LocalHostName`) in lowercase, without `.local`. `flick config path` prints both paths and whether each file exists. `FLICK_HOST_NAME` overrides the host name; set it empty to load no overlay.

Flick merges the overlay over config.toml:

- Tables merge key by key, at every depth: `[kota] hotkey` in the overlay replaces only that key of the base `[kota]`.
- Any other value replaces the base value whole: strings, numbers, arrays, and arrays of tables such as `[[sys.machine]]` or `[[quicklink.links]]`. The overlay's list is the list; it does not add to the base's.
- An overlay cannot remove a key. A module that a table in config.toml turns on stays on, unless the module has its own switch (`enabled = false` turns any module off). Some modules turn on as soon as their table has a key (`[kota]`, `[keys]`, `[dictation]`): put those tables only in the overlays of the Macs that use them, never in the shared file.
- Old flat keys (`[[quicklinks]]`, ...) map in each file before the merge.
- No overlay file: Flick reads config.toml alone, as before. An error in the overlay names the overlay file, and like a bad config.toml it means defaults at startup and no change on reload.
- Reload (**Reload Flick Config**, `flick reload`) reads both files again, so a new overlay applies on the next reload. Flick does not watch either file. Edits that Flick makes (quicklinks, Raycast import) go to config.toml.

Example for a laptop and a server sharing config.toml:

```toml
# config.toml (shared): hotkeys, [window.keys], [[quicklink.links]], [herdr], ...
hotkey = "alt+shift+Space"
```

```toml
# config.jaymins-macbook-pro.toml (laptop only)
[kota]
machine = "mbp-server"
hotkey = "cmd+ctrl+alt+shift+KeyO"

[dictation]
engine = "parakeet"

[[sys.machine]]
name = "mbp-server"
via = "flick"
host = "mbp-server"
ssh = "jaymin@mbp-server"

[remote]
peers = ["mbp-server"]
```

```toml
# config.mbp-server.toml (server only)
[remote]
peers = ["jaymins-macbook-pro"]

[[sys.service]]
name = "kota-memory"
kind = "http"
target = "http://127.0.0.1:8300/health"
```

To move an existing shared file over: cut `[kota]`, `[dictation]` and every `[[sys.machine]]` entry into the laptop's overlay, and every `[[sys.service]]` entry and `[remote]` into each Mac's overlay, then reload on each Mac. Run `flick config path` on each Mac to get its exact overlay name.

Hotkeys use `cmd`, `alt`, `ctrl`, and `shift` with key names such as `Space`, `KeyA`, `Digit1`, `ArrowLeft`, and `Backquote`. Window action names are the command titles in kebab case: `top-left-quarter`, `first-two-thirds`, `almost-maximize`.

> [!WARNING]
> A global hotkey overrides that shortcut in every app. For example, `cmd+KeyL` would replace the browser address bar. A Hyper key (`cmd+ctrl+alt+shift`) avoids conflicts.

Each module's settings are in its own page: [script commands](scripts.md), [keys](keys.md), [activity](activity.md), [tasks](tasks.md), [herdr](herdr.md), [KOTA](kota.md), [system and fleet](sys.md), [dictation](dictation.md), [capture](capture.md), [feedback](feedback.md), [messages](message.md), [remote](remote.md), and [rebuild](install.md#rebuild-settings).

## Launcher appearance

`[launcher]` sets how the launcher panel looks. It is not a module, so it has no `enabled` key.

```toml
[launcher]
opacity = 0.9    # 0.6 to 1.0; lower shows more of the desktop through the blur
```

`opacity` applies to the blurred background only. Text and icons stay fully opaque. Flick clamps values below 0.6 or above 1.0 into that range, so the text stays easy to read. The default is 0.9. Set 1.0 to get the solid blur of earlier versions. A reload applies a change.

Status icons in some rows have a color: green for running, done or healthy, orange for something that needs a look, red for failed or down. The colors follow light and dark mode. Rows that use them: fleet machines and services and the **Fleet** root item (only while something is wrong), herdr agents (done, blocked), tasks in the task list (running, done) and the rebuild status row (installed, failed).

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
| ↑ ↓, ⌃P ⌃N, ⌃K ⌃J | Move the selection (also in the ⌘K menu) |
| ↵ | Run the selected item |
| ⇥ | Enter a quicklink argument |
| ⎋ | Go back, or close |
| ⌫ in an empty field | Go back |
| J K, ⌃D ⌃U, G ⇧G on a confirmation | Scroll its rows: one, half a page, top or bottom |

The launcher keys are fixed. ⌃K moves up, so it does not delete to the end of the search text.
