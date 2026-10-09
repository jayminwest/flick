<div align="center">

[![CI](https://img.shields.io/github/actions/workflow/status/jayminwest/flick/ci.yml?branch=main&style=for-the-badge&label=CI)](https://github.com/jayminwest/flick/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/jayminwest/flick?style=for-the-badge&label=Release)](https://github.com/jayminwest/flick/releases)
[![License: MIT-0](https://img.shields.io/badge/License-MIT--0-blue.svg?style=for-the-badge)](LICENSE)

**[Quickstart](#quickstart)** · **[Configuration](#configuration)** · **[Keys](#keys)** · **[Roadmap](#roadmap)**

</div>

# Flick

## A keyboard-first launcher and window manager for macOS.

Flick replaces a launcher, a window snapper, and a window switcher with one small native app. It is Rust on AppKit: no web view, no JavaScript runtime, no account, no network calls. The app is about 3,200 lines of Rust, not counting tests, and a binary under 4 MB.

<p align="center">
  <img src="docs/screenshots/launcher.png" alt="The Flick launcher over a blurred desktop: an empty search field and suggestions ranked by recent use: commands, quicklinks, and applications" width="750">
</p>

## Why

Raycast, Rectangle, and AltTab each own one hotkey and one background process. Flick puts the parts you use every day behind one config file that you can keep in your dotfiles:

```text
⌃Space ──► launcher ──┬── apps (fuzzy, learns from use)
                      ├── quicklinks ("g rust" searches Google)
                      ├── clipboard history
                      └── window commands
⌘Space ──► window switcher ──► any window, any desktop
⌘`     ──► other desktop
Hyper+HJKL ──► snap windows (repeat to cycle 1/2 → 2/3 → 1/3)
```

## What Flick does

- **Launcher.** Fuzzy search over apps and commands. Acronyms match (`vsc` finds Visual Studio Code). Results you pick often rank higher, with a 14-day half-life.
- **Window management.** Halves, quarters, thirds, two-thirds, maximize, almost maximize, center, minimize, hide, next and previous display. Bind any of them to a global hotkey. Repeating a half cycles its size, as Rectangle does.
- **Window switcher.** Fuzzy search over open windows by title and app. The first result is the previous window, so the hotkey and ↵ jump back. Apps on other desktops show as one entry that switches desktops.
- **Desktop toggle.** One hotkey jumps to the most recent app on another desktop. Built for a full-screen terminal on one desktop and everything else on another.
- **Clipboard history.** The last 500 text clips, searchable. ↵ pastes into the previous app. Flick skips content that password managers mark as concealed.
- **Quicklinks.** URLs and paths with an optional `{query}` argument. Type `<keyword> <text>` to run one from the root search. Import your Raycast quicklinks with one command.

## See it

| | |
|---|---|
| ![Window commands](docs/screenshots/window-commands.png) | ![Quicklink](docs/screenshots/quicklink.png) |
| *Window commands: type a few letters, or bind them to hotkeys* | *Quicklinks: `gh flick` searches GitHub from the root* |

## Quickstart

Flick needs macOS 13 or later and a Rust toolchain.

```bash
git clone https://github.com/jayminwest/flick
cd flick
./scripts/bundle.sh --install
```

The script builds `~/Applications/Flick.app` and starts it. Press `⌥⇧Space` to open the launcher.

Window management, the window switcher, and auto-paste need Accessibility permission. macOS asks the first time you use one of them. Turn Flick on in **System Settings → Privacy & Security → Accessibility**.

> [!NOTE]
> The script signs Flick with your Apple Development certificate when you have one. With that identity, macOS keeps the Accessibility permission across rebuilds. Without one, the script signs ad hoc, and macOS asks again after each rebuild.

### Prebuilt app

Each [release](https://github.com/jayminwest/flick/releases) has an Apple Silicon build. It is not notarized, so remove the quarantine flag after you unzip it:

```bash
xattr -dr com.apple.quarantine Flick.app
mv Flick.app ~/Applications/
```

### Launch at login

Add Flick to **System Settings → General → Login Items**. With nix-darwin and home-manager, use a launchd agent:

```nix
launchd.agents.flick = {
  enable = true;
  config = {
    ProgramArguments = [ "/Users/you/Applications/Flick.app/Contents/MacOS/Flick" ];
    RunAtLoad = true;
    KeepAlive.SuccessfulExit = false; # restart on crash; "Quit Flick" stays quit
    ProcessType = "Interactive";
  };
};
```

`bundle.sh --install` restarts Flick through this agent when it exists.

## Configuration

Flick reads `~/.config/flick/config.toml` and writes a commented default on first run. Set `FLICK_CONFIG` to use a different path. Run **Reload Flick Config** from the launcher after you edit it.

```toml
hotkey = "ctrl+Space"              # launcher

[switcher]
hotkey = "cmd+Space"               # window switcher

[desktop]
hotkey = "cmd+Backquote"           # most recent app on another desktop

[window.keys]
# Hyper (cmd+ctrl+alt+shift), e.g. Caps Lock via Hyperkey
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

Each module reads its own table: `app`, `desktop`, `switcher`, `window`, `quicklink`, `builtin`, `clip`. Set `enabled = false` in a table to turn that module off. Config files from older versions keep working: the flat keys `windows_hotkey`, `desktop_toggle`, `[window_keys]` and `[[quicklinks]]` still apply.

Hotkeys use `cmd`, `alt`, `ctrl`, and `shift` with key names such as `Space`, `KeyA`, `Digit1`, `ArrowLeft`, and `Backquote`. Window action names are the command titles in kebab case: `top-left-quarter`, `first-two-thirds`, `almost-maximize`.

> [!WARNING]
> A global hotkey overrides that shortcut in every app. For example, `cmd+KeyL` would replace the browser address bar. A Hyper key (`cmd+ctrl+alt+shift`) avoids conflicts.

### Import Raycast quicklinks

In Raycast, run **Export Quicklinks**. Then:

```bash
~/Applications/Flick.app/Contents/MacOS/Flick import-raycast ~/Downloads/Quicklinks*.json
```

Flick appends the links to the config file as `[[quicklink.links]]` entries. It changes `{argument}` placeholders to `{query}` and skips links with the same name or URL as an existing link. It reports placeholders that it cannot fill, such as `{clipboard}`.

### Command line

A running Flick listens on `~/Library/Application Support/Flick/flick.sock` (mode 0600; set `FLICK_SOCKET` to use another path). The `flick` binary is its client:

```bash
flick app list                 # <name>\t<path> per app
flick app open Safari
flick app quit Safari          # asks it to quit, like cmd+Q; force-quit forces it
flick app reveal Safari        # show the bundle in Finder
flick app uninstall Foo --dry-run  # <size>\t<path> for the bundle and its leftovers
flick app uninstall Foo --yes  # move exactly those to the Trash
flick clip list                # <id>\t<first line>, newest first
flick clip get 42              # one clip's full text
flick window left-half         # any action from `flick window list`
flick reload                   # reload config.toml
flick --json clip list         # the raw reply: {"ok":"..."} or {"error":"..."}
flick events | jq .            # app_activated, pasteboard_changed, wake, idle, ... as JSON lines
```

The protocol is one JSON array of strings per line, `["<module>","<verb>",args...]`, answered by one JSON line. An error reply exits with status 1.

## Keys

| Key | Action |
|---|---|
| ↑ ↓, ⌃P ⌃N | Move the selection |
| ↵ | Run the selected item |
| ⇥ | Enter a quicklink argument |
| ⎋ | Go back, or close |
| ⌫ in an empty field | Go back |

## How it works

| Path | Role |
|---|---|
| [`modules/`](src/modules) | One directory per feature: apps, quicklinks, clipboard, windows, switcher, desktop, flick. Registered in [`modules/mod.rs`](src/modules/mod.rs) |
| [`core/`](src/core) | Items and their ids, the `Module` trait and registry, events, the control protocol, fuzzy ranking ([nucleo](https://github.com/helix-editor/nucleo)) and frecency |
| [`platform/`](src/platform) | All `unsafe` and macOS API calls, behind safe functions |
| [`app.rs`](src/app.rs) | Controller: the view stack, routing keys and hotkeys to modules |
| [`root.rs`](src/root.rs) | Root search ranking |
| [`ui.rs`](src/ui.rs) | The launcher view over the platform panel |
| [`hotkey.rs`](src/hotkey.rs) | Global hotkey bindings |
| [`store.rs`](src/store.rs) | SQLite: one connection, per-module migrations, usage counts |
| [`config.rs`](src/config.rs) | TOML config: per-module tables, legacy keys |
| [`raycast.rs`](src/raycast.rs) | Raycast quicklink import |
| [`control/`](src/control), [`cli/`](src/cli) | Control socket server, and the command-line client |

[ARCHITECTURE.md](ARCHITECTURE.md) has the layer rules, the module contract, and how to add a module.

The panel is a non-activating `NSPanel`, so the app you came from stays active while Flick has keyboard focus. That is why window commands and paste act on the right app.

macOS has no public API for Spaces. Flick switches desktops by activating an app the way a Dock click does, and macOS follows the app to its desktop. The Accessibility API returns window titles only for the current desktop, so the switcher shows one entry per app on other desktops.

`flick snapshot <out.png> [query]` draws the launcher to a PNG without showing it. It uses the default config and an empty database, so the screenshots hold no personal data.

Flick keeps usage and clipboard data in `~/Library/Application Support/Flick/flick.db`. It sends nothing over the network.

## Roadmap

- **Triggers.** Hotkeys that run shell commands or HTTP requests, and hold-a-modifier triggers, to replace small Hammerspoon configs.
- **Mouse support.** Hover and click on results.
- **Actions menu.** `⌘K` on a result: copy path, reveal in Finder, quit app.
- **Clipboard images.**
- **Script commands.** Shell scripts with Raycast-style metadata as launcher commands.

## Status

Early (`0.0.1`). The author uses Flick as a daily driver on an Apple Silicon laptop. The pure logic has unit tests: ranking, frecency, window geometry, config, storage, and import. The AppKit and Accessibility layers have no automated tests.

## License

MIT No Attribution. See [LICENSE](LICENSE).
