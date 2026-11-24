# Hand checks

A [driven instance](drive.md) drives and reads the launcher panel, the HUD cards and the
surfaces. These things it cannot show, so the owner checks them at the keyboard. Drive the
rest first, then ask for a hand check of only what this list names.

## Not in a driven instance

- The modules in `modules::DRIVEN_OFF`, which act on the real desktop or hardware: desktop,
  switcher, window, tile, keys (Hyper and chords), dictation, capture (screenshots, annotation,
  drawing on screen) and flick (rebuild and reinstall). Their pages list their manual tests.
- Hotkeys and the key tap: a driven instance binds no hotkey and installs no key tap.
  `drive key` enters at `app::command`, after the panel has read the key.
- The menu bar: status items, KOTA presence.
- Notifications: banners never show.
- HUD cards and surfaces on screen (messages, KOTA chat, cards): `drive scene` reads them
  and `drive card` and `drive surface` press and type into them, but they never show. Their
  place, stacking, focus and clicks need the owner. HUD cards have no `drive shot`.
- The desktop: pasteboard writes and pastes (clip history, snippets, "Copy" actions), opening
  apps, files and URLs, app and window activation, quit, reveal, hide, the Trash, the Caps Lock
  remap. These do nothing or answer an error.
- Observers: app switches, sleep and wake, screen changes and pasteboard changes are not
  watched, so activity tracking and clip history record nothing by themselves.
- The tailnet: a driven instance serves network peers on loopback only (`localhost`, see
  [drive.md](drive.md#a-network-peer-a-simulator)), never over Tailscale. Real peers on
  other Macs, `tailscale whois` and the Tailscale addresses are [remote](remote.md) manual
  tests. `drive` requests are local only.

## Only on screen

`drive shot` renders the panel off screen. It does not show:

- the panel's place, size and opacity on a real display, and with several displays;
- focus: which app is active, where the caret is, whether the panel takes keys;
- animation, scrolling by trackpad or mouse, clicks;
- the real field editor: edit keys (⌘C, ⌘V, ⌘A), IME and dead keys;
- behavior across the owner's real data, `config.toml` and `flick.db` (migrations on an old
  database).
