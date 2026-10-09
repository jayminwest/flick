# Key triggers

The `keys` module replaces Hyperkey and small Hammerspoon configs. It turns one key into Hyper, and runs an action when a set of keys goes down and again when it comes up (push-to-talk). It is off until you add a `[keys]` table: with no table, Flick creates no key tap and does not remap Caps Lock.

```toml
[keys]
hyper = "caps_lock"                # Caps Lock adds hyper_mods to keys held with it
hyper_mods = "cmd+ctrl+alt+shift"  # the default
hyper_tap = "Escape"               # a tap shorter than hyper_tap_ms sends this; "" for nothing
hyper_tap_ms = 300

[[keys.chord]]
name = "ptt"
keys = ["right_cmd", "right_alt"]  # modifiers plus at most one other key
on_down = { http = "POST http://localhost:8600/pipeline/listen/start" }
on_up = { http = "POST http://localhost:8600/pipeline/listen/stop" }
```

- **Hyper.** While the hyper key is held, Flick adds `hyper_mods` to each key event, so `[window.keys]` bindings such as `cmd+ctrl+alt+shift+KeyH` fire. `hyper = "caps_lock"` remaps Caps Lock to F18 with `hidutil`, so Caps Lock never toggles uppercase; any other key name (for example `hyper = "F18"`) uses that key and skips the remap.
- **Chords.** Key names: `cmd`, `alt`, `ctrl`, `shift` (either side), `left_cmd`, `right_cmd`, `left_alt`, `right_alt`, `left_ctrl`, `right_ctrl`, `left_shift`, `right_shift`, `fn`, and one hotkey key name such as `KeyA` or `F13`. A sided name does not match the other side. Modifiers pass through to apps; a chord's other key does not type. Key repeat never fires a chord again.
- **Actions.** `{ http = "[METHOD ]http://host[:port]/path" }` sends one request with an empty body (method defaults to `POST`; `http://` only, 2 s timeout). `{ shell = "..." }` runs `/bin/sh -c` with `FLICK_CHORD=<name>` and `FLICK_CHORD_STATE=down|up`. Actions run in order on one worker thread, so an `up` never overtakes its `down`.
- **Events.** Each edge is also an event: `flick events` prints `{"event":"chord","index":0,"down":true}`.

```bash
flick keys list                # <index>\t<name>\t<keys>\tdown: <action>\tup: <action>, then hyper
flick keys status              # key tap, secure input, Caps Lock remap, conflicts
flick keys fire ptt down       # run a chord's action without the keyboard
```

`flick keys status` reports conflicts: Hyperkey running with `hyper = "caps_lock"` (two remaps of one key), Hammerspoon running with a chord configured (its taps may run the same chord), a global hotkey on the hyper key (the tap swallows it), and a chord key that is also a global hotkey. The startup log lists the same conflicts, but it misses hotkey conflicts, because hotkeys bind after the key tap starts.

Limits:

- The key tap needs Accessibility. Without it, `flick keys status` says `Accessibility needed`, and with `hyper = "caps_lock"` Caps Lock does nothing (it sends F18) until you grant it. Flick retries every 5 s, so a grant needs no restart.
- Secure input (password fields, Terminal's Secure Keyboard Entry) hides key presses from the tap: Hyper and chords with a non-modifier key pause there. Modifier-only chords still work.
- Flick clears the Caps Lock remap when it quits through **Quit Flick** or SIGTERM (launchd stop, `kill`). It handles SIGTERM itself only after it has set the remap; before that, SIGTERM ends Flick at once, with nothing to clear. After a crash, Caps Lock stays F18 until Flick starts again. At start, Flick clears its own entry if `hyper` is no longer `caps_lock` (or `[keys]` is gone), and leaves other `hidutil` mappings alone. To reset every mapping by hand:

  ```bash
  hidutil property --set '{"UserKeyMapping":[]}'   # clears every hidutil key mapping
  ```

## Cutover and manual tests

The `keys` module replaces two background apps:

- **Hyperkey** turns Caps Lock into Hyper (`cmd+ctrl+alt+shift`), and a quick press into Escape.
- **Hammerspoon** runs `~/.hammerspoon/init.lua` (written by a voice app).
  It starts the voice app listening while right-Command and
  right-Option are both held, and stops it when one is released.

This section has the dotfiles config, the steps to move over, and a manual test checklist.
Nothing here runs by itself: do each step by hand.

### Dotfiles config

Add this to `~/.dotfiles/home/.config/flick/config.toml`. The existing `[window_keys]`
bindings (`cmd+ctrl+alt+shift+KeyH` and so on) do not change: Flick's Hyper key fires them.

```toml
# Caps Lock is Hyper (cmd+ctrl+alt+shift); a quick tap is Escape. Replaces Hyperkey.
[keys]
hyper = "caps_lock"
hyper_tap = "Escape"
hyper_tap_ms = 300

# Voice push-to-talk: hold right-Command and right-Option. Replaces Hammerspoon.
# The endpoints are idempotent, and both keys still reach the app under them.
[[keys.chord]]
name = "ptt"
keys = ["right_cmd", "right_alt"]
on_down = { http = "POST http://localhost:8600/pipeline/listen/start" }
on_up = { http = "POST http://localhost:8600/pipeline/listen/stop" }
```

Also change the `[window_keys]` comment `Caps Lock via Hyperkey` to `Caps Lock via [keys] hyper`.

### Cutover

Do the steps in this order. Hyperkey and Flick must never both remap Caps Lock.

1. Build and install Flick with the `keys` module (`./scripts/bundle.sh --install`).
2. Check Accessibility: **System Settings → Privacy & Security → Accessibility** lists Flick,
   turned on. An ad-hoc signed rebuild loses the grant; turn it off and on again.
3. Quit Hyperkey, and turn off its **Launch on login**. Remove it from
   **System Settings → General → Login Items** if it is listed there.
4. Quit Hammerspoon, and remove it from Login Items.
5. Add the config above to the dotfiles, apply it (home-manager), and run
   **Reload Flick Config** (or `flick reload`).
6. Run `flick keys status`. Expect:

   ```text
   tap: key tap running
   secure input: off
   chords: 1
   hyper: caps_lock (cmd+ctrl+alt+shift)
   caps lock remap: set
   ```

   and no `conflict:` lines. A conflict line names the app or hotkey to remove.
7. Do the manual tests below.
8. Remove the old apps from the dotfiles: the `"hammerspoon"` cask in
   `~/.dotfiles/hosts/workstation.nix` and `~/.dotfiles/hosts/mbp-server.nix`. Hyperkey is not
   in the dotfiles: move `Hyperkey.app` to the Trash by hand. Then stop the voice app
   from writing `~/.hammerspoon/init.lua`, and delete that file.

To go back: remove the `[keys]` table, reload Flick (this clears the Caps Lock remap), and
start Hyperkey and Hammerspoon again.

After a crash, starting Flick again sets or clears its Caps Lock entry to match the config.
If Caps Lock still does nothing, reset the HID key mappings. This clears every `hidutil`
mapping, not only Flick's:

```bash
hidutil property --set '{"UserKeyMapping":[]}'
hidutil property --get UserKeyMapping   # expect (null) or ()
```

### Manual test checklist

Run with Hyperkey and Hammerspoon quit, and the config above loaded.

Push-to-talk:

- [ ] In one terminal, run `flick events`. Hold right-Command + right-Option, then release one.
      Expect `{"event":"chord","index":0,"down":true}`, then `..."down":false}`.
- [ ] The voice app starts listening on the press and stops on the release (its log shows one
      start and one stop). Without the voice app: quit it, run `nc -l 8600`, and expect one
      `POST /pipeline/listen/start` request per press (start `nc` again for the stop).
- [ ] 20 presses in a row: each one starts and stops listening.
- [ ] Left-Command + left-Option does nothing. Right-Command + right-Option + a letter still
      reaches the app.
- [ ] `flick keys fire ptt down`, then `flick keys fire ptt up`, start and stop listening
      without the keyboard.

Hyper:

- [ ] Caps Lock + H / L / K / J / N / P run left-half, right-half, maximize, hide,
      next-display and previous-display.
- [ ] Caps Lock + L three times cycles 1/2 → 2/3 → 1/3.
- [ ] A quick Caps Lock tap sends Escape (closes a dialog; leaves vim insert mode). A hold of
      more than 300 ms, or a hold with another key, sends no Escape.
- [ ] The Caps Lock light never comes on, and typing never turns uppercase.

Everything else still works:

- [ ] The launcher hotkey, the window switcher and the desktop toggle.
- [ ] Typing in an editor: no dropped or doubled keys. `cmd+H`, `cmd+L` and the other plain
      `cmd` shortcuts still work in apps.

Recovery:

- [ ] Sleep the Mac while holding right-Command + right-Option, wake it: the voice app stops
      listening, and push-to-talk and Hyper work again within 5 s.
- [ ] Turn Flick off in Accessibility: `flick keys status` says `Accessibility needed`, and
      Caps Lock does nothing. Turn it on again: within 5 s the status says
      `key tap running` and Hyper works, with no restart.
- [ ] Turn on **Terminal → Secure Keyboard Entry**: `flick keys status` says
      `secure input: on`, and push-to-talk still works. Turn it off again.
- [ ] Start Hyperkey: `flick keys status` shows the Hyperkey conflict. Start Hammerspoon:
      it shows the Hammerspoon conflict. Quit both again.
- [ ] **Quit Flick**: `hidutil property --get UserKeyMapping` no longer lists `0x700000039`,
      and Caps Lock toggles uppercase again. Start Flick: the remap is back.
- [ ] Stop Flick with launchd (`launchctl kickstart -k gui/$(id -u)/org.nix-community.home.flick`):
      the remap is cleared on stop and set again on start, and everything above works.
- [ ] Remove `hyper` from the config and reload: the remap is cleared.
- [ ] With the remap set, `kill -9` Flick, remove `hyper` from the config and start Flick:
      the remap is cleared (`hidutil property --get UserKeyMapping` has no Flick entry).
- [ ] Idle CPU stays near 0% for 60 s in Activity Monitor.
