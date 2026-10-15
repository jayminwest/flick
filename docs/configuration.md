# Configuration

Flick reads `~/.config/flick/config.toml` and writes a commented default on first run. Set `FLICK_CONFIG` to use a different path. Run **Reload Flick Config** from the launcher after you edit it.

Every option, with its default and a one-line note, is in [`config.example.toml`](../config.example.toml). `flick config example` prints that file, so you can read it also when home-manager or another tool writes your config. Delete the `#` in front of a setting to use it.

```toml
hotkey = "ctrl+Space"              # launcher

[switcher]
hotkey = "cmd+Space"               # window switcher
cycle_hotkey = "cmd+Tab"           # hold to cycle windows, ⇧ back, ⌘Q quits; replaces macOS ⌘Tab

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

Each module reads its own table: `app`, `desktop`, `switcher`, `window`, `quicklink`, `script`, `ext`, `builtin`, `clip`, `activity`, `herdr`, `kota`, `llm`, `sys`, `task`, `keys`, `dictation`, `capture`, `feedback`, `message`, `flick`, `remote`, `help`. Set `enabled = false` in a table to turn that module off. Three modules are opt-in: `kota`, `llm` and `dictation` (KOTA and the local models). Flick registers them only when their table sets a key, so a Mac whose config never names them runs none of their code and shows no item, hotkey, help topic or verb of theirs. `[launcher]` is the launcher's own table (see [Launcher appearance](#launcher-appearance)). Config files from older versions keep working: the flat keys `windows_hotkey`, `desktop_toggle`, `[window_keys]` and `[[quicklinks]]` still apply.

## Per-host overlay

When one config.toml is shared between Macs (home-manager links the same file onto each), put the settings that belong to one Mac in that Mac's overlay: `config.<host>.toml` in the same directory as config.toml (the directory of `FLICK_CONFIG` when set). `<host>` is the Mac's `LocalHostName` (`scutil --get LocalHostName`) in lowercase, without `.local`. `flick config path` prints both paths and whether each file exists. `FLICK_HOST_NAME` overrides the host name; set it empty to load no overlay.

Flick merges the overlay over config.toml:

- Tables merge key by key, at every depth: `[kota] hotkey` in the overlay replaces only that key of the base `[kota]`.
- Any other value replaces the base value whole: strings, numbers, arrays, and arrays of tables such as `[[sys.machine]]` or `[[quicklink.links]]`. The overlay's list is the list; it does not add to the base's.
- An overlay cannot remove a key. A module that a table in config.toml turns on stays on, unless the module has its own switch (`enabled = false` turns any module off). Some modules turn on as soon as their table has a key (`[kota]`, `[llm]`, `[keys]`, `[dictation]`): put those tables only in the overlays of the Macs that use them, never in the shared file.
- Old flat keys (`[[quicklinks]]`, ...) map in each file before the merge.
- No overlay file: Flick reads config.toml alone, as before. An error in the overlay names the overlay file, and like a bad config.toml it means defaults at startup and no change on reload.
- Reload (**Reload Flick Config**, `flick reload`) reads both files again, so a new overlay applies on the next reload. Flick does not watch either file.
- **Open Flick Config** opens config.toml and, when it exists, this Mac's overlay. Edits that Flick makes (quicklinks, Raycast import) go to the file that sets the list: the overlay when it has `[[quicklink.links]]` (its list replaces the shared one), else config.toml. See [Files managed by Nix](#files-managed-by-nix).

Example for a laptop and a server sharing config.toml:

```toml
# config.toml (shared): hotkeys, [window.keys], [[quicklink.links]], [herdr], ...
hotkey = "alt+shift+Space"
```

```toml
# config.my-macbook-pro.toml (laptop only)
[kota]
machine = "my-server"
hotkey = "cmd+ctrl+alt+shift+KeyO"

[dictation]
engine = "parakeet"

[[sys.machine]]
name = "my-server"
via = "flick"
host = "my-server"
ssh = "me@my-server"

[remote]
peers = ["my-server"]
```

```toml
# config.my-server.toml (server only)
[remote]
peers = ["my-macbook-pro"]

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

## Files managed by Nix

home-manager can link config.toml and the overlay into the read-only Nix store. Flick follows each symlink to the real file:

- An out-of-store link (`config.lib.file.mkOutOfStoreSymlink`) ends at a writable file, such as the copy in your dotfiles. **Open Flick Config** opens that file, and quicklink edits write it.
- A file built into the store (`source = ./config.toml`) is read-only. A quicklink edit fails and says so: `<path>: not edited, it links to /nix/store/... in the read-only Nix store; edit the file your Nix (home-manager) config builds it from, rebuild, then run Reload Flick Config`. **Open Flick Config** still opens the file, and the launcher footer says that it is read-only.

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

Run **Create Quicklink** from root search to add a link with a form. Select a quicklink and press ⌘K to edit or delete it. From a shell: `flick quicklink add <name> <url> [--keyword k] [--app a]`, `flick quicklink remove <name>` and `flick quicklink list`. Flick writes the change to config.toml, or to this Mac's [overlay](#per-host-overlay) when the overlay sets `[[quicklink.links]]`, and keeps all other text in the file, comments included. A file in the read-only Nix store is not changed; see [Files managed by Nix](#files-managed-by-nix). The change applies at once, without a reload. A renamed link loses its usage history.

### Import from Raycast

In Raycast, run **Export Quicklinks**. Then:

```bash
~/Applications/Flick.app/Contents/MacOS/Flick import-raycast ~/Downloads/Quicklinks*.json
```

Flick appends the links as `[[quicklink.links]]` entries to the file that holds the links (config.toml, or the overlay when it sets them). It changes `{argument}` placeholders to `{query}` and skips links with the same name or URL as an existing link. It reports placeholders that it cannot fill, such as `{clipboard}`.

## Extensions

An extension is a program, in any language, that adds to the launcher. Add one `[[ext.extensions]]` table for each:

```toml
[[ext.extensions]]
name = "notes"                          # 1-32 of a-z, 0-9 and -; unique
argv = ["~/bin/notes-ext", "--flick"]   # no shell; a leading ~/ is the home folder
timeout_ms = 2000                       # the default
hotkeys = { inbox = "cmd+ctrl+alt+shift+KeyE" }   # <view> = "<hotkey>": opens the view
keywords = ["n"]                        # "n <text>" in root search asks the extension
subscribe = ["wake"]                    # events it gets, as op: event
```

Each call runs the program once. Flick writes one JSON request to its stdin and reads one JSON reply from its stdout; the program exits 0. The program gets `FLICK_EXT` (its name), `FLICK_SOCKET` (this Flick's control socket, for `flick` commands back) and `FLICK_VERSION`. Calls to one extension run one at a time on its own thread, never on the launcher's, so a slow extension does not slow the launcher. A call that takes longer than `timeout_ms` is stopped: Flick kills the program and everything it started. Actions and form submits get at least 10 s. A failed call (a timeout, an exit other than 0, a reply that is not JSON or has an `error`) is logged and shown as a notification, `<name>: <why>`; for a failed exit, `<why>` is the last line of stderr. An action or form that fails is shown as a card in the top right corner instead, and in the footer of its view or in its form while that is still on the screen. A config reload stops the calls of an extension whose entry changed, after the call that runs.

Root search shows the extension's root items with the other modules' items, ranked by the same fuzzy match and usage. Flick keeps the last items it got and shows them at once; it asks the extension again each time the launcher opens and at each config reload, so a change shows a moment later. A view (`<name>/<view>`, opened by a hotkey or an item) shows its last items too, or `Loading…` the first time, and asks again each time it opens; Flick filters its items as you type. A view whose reply says `"filter": "ext"` is searched by the extension instead: 60 ms after the last keystroke Flick sends `op: query` with the search text and shows the items of the reply in the extension's order. Typing `<keyword> <text>` in root search, with one of the extension's `keywords`, sends `op: keyword` the same way, and the reply's items show above the ranked results. Only the newest query runs: a newer keystroke kills the program that answers an older one, and drops older queries that still wait. An item's id is `ext:<name>/<key>`, so its usage history stays while its key stays. An icon file that is missing or is not an image is left out (logged). A failed call for the view on the screen shows in its footer instead of a notification.

`subscribe` lists the events an extension gets, as `op: event` with `event` one of `started` (Flick started), `reloaded` (a config reload), `wake` (the Mac woke from sleep), `unlocked` (the screen unlocked) and `launcher_opened`. Flick ignores the reply, and a failed event call is only logged, never shown. Events wait behind the extension's other calls: one runs only when no other call is queued, so an action or a search waits at most for the event that runs already. An event of a kind that waits already is dropped, so a burst of them sends one.

`flick ext ls` lists the extensions with how each one's last call went (its op, ok or the error, and how long it took). `flick ext refresh <name>` asks one again for its root items and for its view on the screen. `flick ext open <name> <view>` opens a view as its hotkey does. `flick ext call <name> <json>` (or `--stdin` for the JSON) is for debugging: it sends the request to the program exactly as given and prints the program's stdout as it is, without checking it. A peer over the network may not send `ext open` or `ext call`.

## Launcher keys

| Key | Action |
|---|---|
| ↑ ↓, ⌃P ⌃N, ⌃K ⌃J | Move the selection (also in the ⌘K menu) |
| ↵ | Run the selected item |
| ⇥ | Enter a quicklink argument |
| ⎋ | Go back, or close |
| ⌫ in an empty field | Go back |
| J K, ⌃D ⌃U, G ⇧G on a confirmation | Scroll its rows: one, half a page, top or bottom |
| J K, ⌃D ⌃U, G ⇧G on an output view (herdr Show Output) | Scroll the text: one line, half a page, top or bottom. ↑ ↓ still move the selection |

The launcher keys are fixed. ⌃K moves up, so it does not delete to the end of the search text.
