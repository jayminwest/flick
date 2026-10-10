<div align="center">

<img src="assets/icon/flick.svg" width="128" alt="Flick icon: three amber dots, each brighter than the last, moving up and to the right">

[![CI](https://img.shields.io/github/actions/workflow/status/jayminwest/flick/ci.yml?branch=main&style=for-the-badge&label=CI)](https://github.com/jayminwest/flick/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/jayminwest/flick?style=for-the-badge&label=Release)](https://github.com/jayminwest/flick/releases)
[![License: MIT-0](https://img.shields.io/badge/License-MIT--0-blue.svg?style=for-the-badge)](LICENSE)

**[Quickstart](#quickstart)** · **[Documentation](docs/README.md)** · **[Configuration](docs/configuration.md)** · **[Roadmap](#roadmap)**

</div>

# Flick

## A keyboard-first launcher and window manager for macOS.

Flick replaces a launcher, a window snapper, and a window switcher with one small native app. It is Rust on AppKit: no web view, no JavaScript runtime, no account, and no network library. Its network use is what you configure: the kota, sys, message (KOTA chat and card replies) and llm modules shell out to `ssh` and `curl`.

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
- **Window management.** Halves, quarters, thirds, two-thirds, maximize, almost maximize, center, minimize, hide, next and previous display. Bind any of them to a global hotkey. Repeating a half cycles its size, as Rectangle does. [Configuration](docs/configuration.md).
- **Window switcher.** Fuzzy search over open windows by title and app. The first result is the previous window, so the hotkey and ↵ jump back. Apps on other desktops show as one entry that switches desktops.
- **Desktop toggle.** One hotkey jumps to the most recent app on another desktop. Built for a full-screen terminal on one desktop and everything else on another.
- **Clipboard history.** The last 500 text clips, searchable. ↵ pastes into the previous app. Flick skips content that password managers mark as concealed.
- **Herdr agents.** Coding agents in herdr on this Mac and on herdr's saved SSH machines, the ones waiting on you first. ↵ jumps to the agent's pane; a notification tells you when one starts to wait. [Herdr](docs/herdr.md).
- **Screenshots and drawing.** Capture an area, a window or a screen to a PNG file and the clipboard, mark it up with arrows, boxes, a pen, a highlighter, text and redaction, and draw on the screen with a cursor halo. It replaces Shottr and Presentify. [Capture](docs/capture.md).
- **Key triggers.** Caps Lock as Hyper, and key chords that run an HTTP request, a shell command or a Flick command on key down and up (push-to-talk). [Key triggers](docs/keys.md).
- **Dictation.** Hold a chord, speak, release: the text goes into the focused field. whisper.cpp runs on this Mac; audio and text never leave it. [Dictation](docs/dictation.md).
- **KOTA.** The state of an always-on agent session in the menu bar (thinking, blocked, idle, down), the cards waiting on you, and a quick ask. A floating chat window holds threaded conversations with it, with its cards inline and context from the app you were in. [KOTA](docs/kota.md), [Chat](docs/message.md#chat).
- **Fleet.** This Mac's health and service checks, and a launcher view of several Macs and their services, with Screen Sharing, log tails and confirmed restarts. [System and fleet](docs/sys.md).
- **Activity and tasks.** Opt-in time tracking per app, kept on this Mac, and a short task list with one running timer. [Activity](docs/activity.md), [Tasks](docs/tasks.md).
- **Quicklinks.** URLs and paths with an optional `{query}` argument. Type `<keyword> <text>` to run one from the root search. Import your Raycast quicklinks with one command. [Quicklinks](docs/configuration.md#quicklinks).

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

The script builds `~/Applications/Flick.app` and starts it. Press `⌥⇧Space` to open the launcher. Window management, the switcher, and auto-paste need Accessibility permission: turn Flick on in **System Settings → Privacy & Security → Accessibility**.

[Install and update](docs/install.md) covers signing, the prebuilt app, launch at login, and rebuilding from the checkout.

## Documentation

- [Install and update](docs/install.md)
- [Configuration](docs/configuration.md): config file, hotkeys, quicklinks, launcher keys
- [Command line](docs/cli.md)
- Modules: [Script commands](docs/scripts.md) · [Key triggers](docs/keys.md) · [Activity](docs/activity.md) · [Tasks](docs/tasks.md) · [Herdr](docs/herdr.md) · [Capture](docs/capture.md) · [Feedback](docs/feedback.md) · [Messages](docs/message.md) · [Dictation](docs/dictation.md) · [KOTA](docs/kota.md) · [System and fleet](docs/sys.md) · [Remote access](docs/remote.md)
- [How it works](docs/how-it-works.md) and [ARCHITECTURE.md](ARCHITECTURE.md)

Flick keeps its data in `~/Library/Application Support/Flick/flick.db` and sends nothing over the network except what you configure: HTTP requests as key triggers, shell commands as key triggers or script commands, the checks and ssh calls of `[kota]` and `[[sys.machine]]`, the ssh calls of the `[message]` KOTA chat and `action_command`, and the curl requests of `[[llm.servers]]`. It listens on the network only when you turn on [remote access](docs/remote.md), and then only on Tailscale addresses.

## Roadmap

- **Mouse support.** Hover and click on results.
- **Clipboard images.**
- **Script commands.** Shell scripts with Raycast-style metadata as launcher commands. Today `[[script.commands]]` in config.toml runs a shell command with a `{query}` argument ([Script commands](docs/scripts.md)).

## Status

Early (`0.0.2`). The author uses Flick as a daily driver on an Apple Silicon laptop. The pure logic has unit tests: ranking, frecency, window geometry, config, storage, and import. The AppKit and Accessibility layers have no automated tests.

## License

MIT No Attribution. See [LICENSE](LICENSE).
