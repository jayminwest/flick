# Flick documentation

Start with the root [README](../README.md) for what Flick is, then pick the page for your task.

## Use Flick

- [`install.md`](install.md): build from source, the prebuilt app, launch at login, rebuild from the checkout.
- [`configuration.md`](configuration.md): `config.toml`, hotkeys, module tables, quicklinks, launcher keys.
- [`cli.md`](cli.md): the `flick` client, the control socket protocol, `--host`.

## Modules

Each page has the module's settings and commands, then a manual test checklist.

- [`scripts.md`](scripts.md): shell commands in root search, with a `{query}` argument.
- [`keys.md`](keys.md): Hyper key and key chords; moving from Hyperkey and Hammerspoon.
- [`activity.md`](activity.md): opt-in time tracking per app, and its privacy rules.
- [`tasks.md`](tasks.md): task list with one running timer.
- [`herdr.md`](herdr.md): coding agents in herdr, local and over SSH.
- [`capture.md`](capture.md): screenshots, annotation, drawing on screen.
- [`feedback.md`](feedback.md): notes about Flick, kept in `feedback.jsonl`.
- [`remote.md`](remote.md): network access over Tailscale; setup between two Macs.

## Change Flick

- [`how-it-works.md`](how-it-works.md): source map and macOS mechanics.
- [`../ARCHITECTURE.md`](../ARCHITECTURE.md): layers, the `Module` contract, events, store, config, control socket.
- [`../CLAUDE.md`](../CLAUDE.md): quality gates and agent rules.
