# How it works

| Path | Role |
|---|---|
| [`modules/`](../src/modules) | One directory per feature: apps, quicklinks, clipboard, windows, switcher, desktop, flick, rebuild, keys, dictation, activity, tasks, herdr, kota, llm, sys, capture, feedback, message, remote, help. Registered in [`modules/mod.rs`](../src/modules/mod.rs) |
| [`core/`](../src/core) | Items and their ids, the `Module` trait and registry, events, the control protocol, fuzzy ranking ([nucleo](https://github.com/helix-editor/nucleo)) and frecency |
| [`platform/`](../src/platform) | All `unsafe` and macOS API calls, behind safe functions |
| [`app.rs`](../src/app.rs) | Controller: the view stack, routing keys and hotkeys to modules |
| [`root.rs`](../src/root.rs) | Root search ranking |
| [`ui.rs`](../src/ui.rs) | The launcher view over the platform panel |
| [`hotkey.rs`](../src/hotkey.rs) | Global hotkey bindings |
| [`core/store.rs`](../src/core/store.rs) | SQLite: one connection, per-module migrations, usage counts |
| [`config.rs`](../src/config.rs) | TOML config: per-module tables, legacy keys |
| [`raycast.rs`](../src/raycast.rs) | Raycast quicklink import |
| [`control/`](../src/control), [`cli/`](../src/cli) | Control socket server, the network transport over Tailscale, and the command-line client |

[ARCHITECTURE.md](../ARCHITECTURE.md) has the layer rules, the module contract, and how to add a module.

The panel is a non-activating `NSPanel`, so the app you came from stays active while Flick has keyboard focus. That is why window commands and paste act on the right app.

macOS has no public API for Spaces. Flick switches desktops by activating an app the way a Dock click does, and macOS follows the app to its desktop. The Accessibility API returns window titles only for the current desktop, so the switcher shows one entry per app on other desktops.

`flick snapshot <out.png> [query]` draws the launcher to a PNG without showing it. It uses the default config and an empty database, so the screenshots hold no personal data.

Flick keeps its data in `~/Library/Application Support/Flick/flick.db`: usage counts for ranking, clipboard history, activity spans, the paths of captures (the images go to `[capture] dir`, default `~/Pictures/Flick`), tasks and their time, HUD messages and KOTA chats, local-model chats while `[llm] history` is on, and the remote-access switch. It sends nothing over the network except what you configure: HTTP requests as key triggers, shell commands as key triggers or script commands, the checks and ssh calls of `[kota]` and `[[sys.machine]]`, the ssh calls of the `[message]` KOTA chat and `action_command`, and the curl requests of `[[llm.servers]]`. It listens on the network only when you turn on [remote access](remote.md), and then only on Tailscale addresses for the peers you name.
