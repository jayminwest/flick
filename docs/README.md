# Flick documentation

Start with the root [README](../README.md) for what Flick is, then pick the page for your task.

## Use Flick

- [`install.md`](install.md): build from source, the prebuilt app, launch at login, rebuild from the checkout.
- [`configuration.md`](configuration.md): `config.toml`, hotkeys, module tables, quicklinks, launcher keys.
- [`cli.md`](cli.md): the `flick` client, the control socket protocol, `--host`.

In the launcher, **Flick Help** lists each feature with a one-line how-to. ↵ opens its page here; on **Draw on Screen** and **Annotate a Screenshot** it lists their keys. ⌘K: **Open Docs**, **Copy How-To**.

## Modules

Each page has the module's settings and commands, then a manual test checklist.

- [`scripts.md`](scripts.md): shell commands in root search, with a `{query}` argument.
- [`keys.md`](keys.md): Hyper key and key chords; moving from Hyperkey and Hammerspoon.
- [`activity.md`](activity.md): opt-in time tracking per app, and its privacy rules.
- [`tasks.md`](tasks.md): task list with one running timer.
- [`herdr.md`](herdr.md): coding agents in herdr, local and over SSH.
- [`capture.md`](capture.md): screenshots, annotation, drawing on screen.
- [`feedback.md`](feedback.md): notes about Flick, kept in `feedback.jsonl`.
- [`message.md`](message.md): messages from agents (KOTA) in a corner card, with history.
- [`cards.md`](cards.md): the card spec for KOTA: JSON schema v1, actions and their security model, presses back to KOTA, examples. Also `flick message card spec`.
- [`kota.md`](kota.md): KOTA's presence in the menu bar, pending cards, quick ask; moving off the `k` script command.
- [`dictation.md`](dictation.md): hold a chord, speak, release; local whisper.cpp, model download, privacy.
- [`sys.md`](sys.md): this Mac's health, service checks, and the fleet of Macs with Screen Sharing, logs and restarts.
- [`remote.md`](remote.md): network access over Tailscale; setup between two Macs; peers for the fleet.

## Ideas

- [`ios.md`](ios.md): Flick on the iPhone: Screen Time APIs, supervision, KOTA bridge, Rust core. Not built.

## Change Flick

- [`how-it-works.md`](how-it-works.md): source map and macOS mechanics.
- [`../ARCHITECTURE.md`](../ARCHITECTURE.md): layers, the `Module` contract, events, store, config, control socket.
- [`../CLAUDE.md`](../CLAUDE.md): quality gates and agent rules.
