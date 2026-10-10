# Architecture

Flick is one binary crate. Features are modules behind one trait. A grep-based gate
(`scripts/checks/layers.sh`, rules in `scripts/layer-rules.toml`) enforces the layer
boundaries. This file describes the code as it is. When you change a contract below, change
this file in the same commit.

## Layers

| Layer | Path | Role |
|---|---|---|
| platform | `src/platform/` | All `unsafe`, objc2, `AppKit`, CF, AX, `CoreGraphics` and Carbon code. Exposes safe functions: `workspace`, `files` (Trash; the only removal path), `pasteboard` (text, and PNG with TIFF through `set_png`; `snapshot`/`restore` of every item and type, and `set_transient_text`, marked `org.nspasteboard.TransientType` + `ConcealedType` so clip history skips it, returning the change count to compare before a restore; `set_text_concealed`, the same markers for copies out of a private surface), `mic` (`AVCaptureDevice` audio authorization: `status`, `request`; class looked up at run time, no AVFoundation crate), `ax`, `spaces`, `screens`, `hotkeys`, `keytap` (the shared keyboard event tap), `hid` (Caps Lock to F18 via `hidutil`), `timer`, `panel` (the launcher: rows with a symbol tint `panel::Tone` (system green, orange, red; secondary label for `Neutral`); the read-only text block under them, a non-editable, non-selectable text view in a scroll view sized to whole lines, scrolled by the mouse and by `scroll_text(TextScroll::{Lines, HalfPages, Top, Bottom})`, with new text at its top, or at its end and following it for `Frame::text_tail` (pure `panel::scroll`, 100% floor); and `set_opacity`, the alpha of the blur backdrop that sits under the content, so text stays opaque), `edit` (the shared ⌘X/⌘C/⌘V/⌘A/⌘Z/⇧⌘Z routing that `panel`, `hud` and `surface` windows call first in `performKeyEquivalent:`, since Flick has no Edit menu), `events`, `app`, `axwatch` (one AX observer for window and title changes), `status_item` (menu bar items, one per owner module: `activity`, `kota`), `hud` (corner cards, one non-activating `NSPanel` per card id that never activates Flick and becomes key only when the user clicks a text field or `hud::focus_top` moves the keyboard into the newest card (then it routes the edit keys and the card keys itself): `show`/`show_card` upsert by id in place, `update`/`update_card`, `dismiss`, `dismiss_all`; content is a `hud::Content` drawn by `text`, or a `core::card::Card` with the module-owned `hud::CardUi` (pending, error line, shell confirm, a muted note line) drawn by `card_view` (blocks, status line, action buttons, the confirm step with the exact command), with its pure layout and text rules in `hud::card_layout` (100% floor); a redraw keeps what the user typed or picked unless the update changed that input's initial value; the pure `hud::stack` (100% floor) stacks them newest nearest the corner with at most `max_cards` visible and a `+N more` pill, on the screen the stack first appeared on; each card has its own timeout with hover hold, which runs only while the user is at the Mac (`hud::timeout`: no input since it was set, or a timer that fired late after a sleep, waits for input and then restarts the full timeout), and an x close button, and Esc (a key monitor installed only while cards show) dismisses the cards that are not `sticky`; user and timeout dismissals go to one fn-pointer handler, `hud::on_dismiss`, and action presses (card id, action id, values JSON read from the live inputs) to another, `hud::on_press`; Cancel on a confirm presses `hud::CANCEL`; `hud::embed::Embedded::draw(card, ui, width, before)` draws the same card into another window's view (a surface transcript row), carrying live input values from `before`, and presses on embedded cards go to `hud::embed::on_press(fn(view key, tag))` instead; card text blocks render styled markdown-lite through `surface::render` in a non-selectable text view; keyboard reach: `hud::focus_top` makes the newest card key without activating Flick and `hud::unfocus` gives the keyboard back to the front app, the card staying; while a card holds it, the pure `hud::keys` (100% floor) maps Tab/Shift-Tab to its controls, ⌘1..⌘6 and ⌘↵ to presses through the same path as clicks, ⌥↑/⌥↓ to the card above/below and Esc to giving the keyboard back (never a dismissal); any press gives it back too), `surface` (generic key-capable floating windows for conversations and dashboards, knowing no module: several keyed by `SurfaceId(&'static str)` in a `thread_local` map; `open(id, &Spec { title, autosave, size, min_size, placeholder, input: Single|Multi, hide_on_blur, private: Option<banner text> }, Handlers)` once; a private surface (flick-f218) has `sharingType` none (out of screenshots, screen recording and window sharing), is not restorable or in the Windows menu, keeps the spec title as its window title, shows a banner strip and an accent outline, turns off every input text service (spell check, grammar, autocorrect, completion, inline prediction, Writing Tools, math, link/data detection, undo; pure `surface::privacy`, 100% floor) and the input's context menu, makes bubbles unselectable, copies ⌘C/⌘X selections only through `pasteboard::set_text_concealed` (nothing selected: the key goes to `Handlers::key`), and refuses `snapshot`; `show` (on the screen under the pointer, kept in place if it already is, else centred; makes it key, and the input first responder only if it is not already), `hide`, `is_visible`, `is_key`, `snapshot` (`cacheDisplayInRect`); `set_header(&Header { title, subtitle, status: Idle|Busy|Error })`, `set_chips(&[Chip { label, symbol }])`, `input`, `set_input`, `set_notice`; `set_rows(&[Row])`, the transcript: `Row::Bubble { key, version, side: Mine|Theirs|System, header, time, md, state: Done|Streaming|Pending|Failed }` (selectable markdown-lite text in a bubble with a header line and a spinner or failure badge; in a normal surface (`surface::bubble`, flick-0955) the row is not window background, so a drag on it selects instead of moving the window, a click on the fill goes to the text, the text takes the first click of a window that is not key, and its context menu starts with Copy Message, the whole message `rows::message_text`), `Row::Card { key, version, card: &core::card::Card, ui: hud::CardUi }` (the HUD card renderer, embedded) and `Row::Divider { text }`, stacked in a flipped scroll document, bottom-aligned while short; rows are diffed by (kind, key, version) so unchanged rows keep their views (and a card its typed values), and the module must bump `version` whenever a row's content or card UI changes; redraws coalesce on a 50 ms timer (flushed at once by `show` and `snapshot`); the view stays scrolled to the bottom only if it was at the bottom; `surface::render` turns `core::markup` into an `NSAttributedString` (semibold, italic trait, monospaced shaded code and code blocks, heading sizes, hanging bullet indents, the link attribute only for `http(s)` URLs). Window: a borderless `NSPanel` subclass, Borderless|NonactivatingPanel|Resizable, floating, CanJoinAllSpaces|FullScreenAuxiliary, movable by background, frame autosave. Focus: Flick never activates; the app in front stays active and gets its key window back when the surface hides. Keys: edit keys first (`edit`), then every other ⌘ key and Return/Escape/Up/Down in the input reach `Handlers::key` as a `Keystroke { key: Return|Escape|Up|Down|Char, cmd, shift, opt }`; an untaken Return submits (⇧/⌥ Return adds a line in `Multi`), an untaken Escape hides and calls `closed`. Handlers are fn pointers called on the main thread with no surface state borrowed: `key` synchronously, `submit` (input cleared first), `chip_removed`, `closed` (user hides only, never after `hide`) and `card_action(id, card id, action id, values JSON)` (an embedded card's press; over-cap values show the error on the card instead) on a later run loop turn; they only queue or post, never borrow app state. Pure `surface::geometry` (placement, min size, part frames), `surface::keys` (keystroke mapping, fallbacks), `surface::rows` (row diff, scroll pin, coalescing, badges, row frames) and `surface::style` (markup to styled runs, link filter) have 100% floors), `pill` (the dictation recording pill, separate from `hud`: one non-activating, mouse-ignoring `NSPanel` at the bottom centre of the screen under the pointer; `show(Phase)` with `Recording(fn() -> f32)` (red dot, level bar redrawn by a self-rescheduling 50 ms `timer::after` only while recording, so idle Flick has no timer), `Transcribing`, `Result(&str)` (hides after 1.5 s) and `Error(&str)` (4 s); `hide`, `shown`; Esc through key monitors installed only while it shows hides it and, while recording or transcribing, calls the one fn-pointer handler `pill::on_cancel`; the pure `pill::layout` has a 100% floor), `clock` (local UTC offset), `notify` (`UNUserNotificationCenter`), `capture` (`/usr/sbin/screencapture`, PNG size, the Screen Recording check and prompt), `context` (what the user is looking at when a window is summoned, knowing no module: `front(selection) -> Option<Front { pid, app, bundle_id, window, selection }>` from `workspace::app_identity`, `ax::focused_window_title` and `ax::selected_text(pid)` (the focused element's `AXSelectedText` with a 0.2 s AX messaging timeout per element, nothing for a secure text field or a whitespace-only selection, never a synthetic ⌘C), None when Flick itself is in front; `shoot_display(hide, restore, work)` checks Screen Recording first (`ShotError::NotPermitted`, nothing hidden), resolves the display under the pointer, calls `hide`, and on a worker waits `rules::SETTLE` (150 ms), screencaptures that display silently to a fresh temp PNG, queues `restore` on the main thread (also if the worker dies) and then hands `work` the `Shot` or `ShotError`, still on the worker, which owns and deletes the file; pure `context::rules`, 100% floor), `ink` (annotation: the pure shape model `ink::model` with a 100% coverage floor, the key legend `ink::legend` (pure, 100% floor; the `help` module lists its keys too) and `ink::hud` that draws it, the `FlickInkView` canvas, the editor window and the per-display draw overlay with the cursor halo), `poll` (`poll(2)` on sockets, for the network accept threads). Every function runs on the main thread, except `keytap`'s and `poll`'s (any thread) and `capture::run`, which blocks until screencapture exits (`context::shoot_display` runs it and its `work` on a worker): launcher and hotkey captures run it on a worker thread, never an area or window selection on the main thread. |
| core | `src/core/` | Plain Rust: `Item`, `ItemId`, `Outcome`, `ListView`, `Form`, `Action`, `Confirm`, `Module`, `Registry`, `Event`, `Ranker` and frecency, control protocol types (`core::control`) and later answers of verbs (`core::later`), the span clock (`core::track`: idle backdating, pause/resume, title normalization, flicker merge, local-day split; `track::civil`: day starts, dates, local times, short durations and range clipping for the `activity` and `task` reports), the card model (`core::card`: KOTA card schema v1, parser with caps and degrade-to-text, normalized JSON, `plain()` fallback; `core::card::action`: the closed `do` vocabulary, its policy (remote-origin `flick` words must pass `net_policy`; `open_app` takes a name or bundle id, never a path), the press values JSON), shared by `message`, `platform::hud`, chat and the status item; the markdown-lite parser (`core::markup`: `parse(md) -> Vec<Block>` of paragraph / heading 1-3 / bullet / fenced code blocks holding `Span { text, style: Style { bold, italic, code }, link }`, never panics, unclosed markers stay literal; `platform::surface::render` draws the blocks styled, for surface bubbles and HUD card text blocks; `plain(md)` is the plain-text form), and the SQLite store and its migrations (`core::store`). Unit tests run without `AppKit`. |
| modules | `src/modules/<name>/` | One directory per feature. Registered in `src/modules/mod.rs`. |
| ui | `src/ui.rs` | Turns `Item`s and `Form`s into the rows and form fields that `platform::panel` draws. Forwards typing and keys to the controller. Knows no feature. |
| controller | `src/app.rs`, `src/app/`, `src/root.rs`, `src/hotkey.rs` | `app` holds the registry, the screen on the panel and the selection, and applies `Outcome`s. `app/screen.rs` is the `Screen` enum and its pure decisions; `app/overlay.rs` runs the action menu, confirmation and form screens. `root` ranks root search. `hotkey` binds hotkeys and routes presses to the controller. |
| shared services | `src/config.rs` | Config loading and per-module tables. |
| control | `src/control/` | The Unix socket server and the network transport over Tailscale (`control::net`). Runs requests on the main thread. Streams events. |
| cli | `src/cli/` | Argument parsing and the socket client. `snapshot`, `import-raycast`, `config example` and `config path` run in-process. |

There is no `src/ui/` directory. `src/raycast.rs` (quicklink import) is CLI code that uses
`crate::modules::quicklinks` directly.

### Boundary rules (`scripts/layer-rules.toml`)

| Rule | Scans | Forbids |
|---|---|---|
| `platform-owns-ffi` | `src/` except `src/platform/` | `unsafe`, `objc2*`, `block2`, `extern "C"` |
| `core-is-pure` | `src/core/` | `crate::platform`, `unsafe`, `objc2` |
| `modules-are-independent` | each `src/modules/<name>/` | `crate::modules::` (except its own path), `super::super::` |

Comment lines do not count. To exempt one file, add an `[[allow]]` entry with a `why`. An allow
entry that matches no violation fails the gate. Not enforced: core imports
`crate::config::Section`, so core is free of the platform, not of the crate.
The controller (`app.rs`) may import `crate::modules`; nothing in core or a module may.

## Module contract

`src/core/module.rs`. Every method except `id` has a do-nothing default. All calls happen on
the main thread.

| Method | Called | Contract |
|---|---|---|
| `id() -> &'static str` | always | Unique. It is the config table name, the `ItemId` module part, the store owner and the CLI module word. |
| `migrations() -> &'static [&'static str]` | startup, reload | Store migrations for tables this module owns. Append only. |
| `configure(&Section) -> Result<(), String>` | registration, reload | Read the `[<id>]` table. Keep state such as history across calls. |
| `items(&mut Cx) -> Vec<Item>` | each root refresh | Root search items. Ranked by fuzzy score plus frecency. |
| `direct(&mut Cx) -> Vec<Item>` | each root refresh | Items placed above the ranked results, unranked (quicklink and script `<keyword> <text>`). |
| `open(view, &mut Cx) -> Option<ListView>` | `Outcome::Push`, hotkey view | Enter a named view this module owns. `None`: no such view; the screen does not change. |
| `closed(view, &mut Cx)` | the screen stops holding the module's view `view` | The view left the launcher: another view, root search or a form replaced it (an action menu or a confirmation over it still holds it). It comes after the next view's `open`, so a module moving between its own views hears `open` of the new one, then `closed` of the old: compare the name. Hiding the launcher does not close a view (ask the panel's visibility); the next screen change does. After a reload it may name a view the instance never opened. Use it to stop work that runs only while a view shows (`sys`'s fleet cadence), instead of inferring it from `items` or `LauncherOpened` (flick-7638). |
| `refresh(&mut ListView, &mut Cx)` | each keystroke in that view, stale events | Fill `view.items` for `cx.query`. The module ranks its own items (`cx.ranker`). It may set `view.text`: read-only text the panel wraps under the rows in a fixed-width font. It scrolls; it starts at its top, or at its end with `view.text_tail` (output that grows: it follows the end while the user is there). A view with `view.title` is read-only, so the vim keys scroll the text (see Screens). |
| `activate(&ItemId, &mut Cx) -> Outcome` | Enter (Tab when `item.tab` is `Tab::Activate`) | Run an item this module created. |
| `form(name, &mut Cx) -> Option<Form>` | `Outcome::Form` | Build a named form this module owns. Form names are a namespace apart from view names. `None`: the screen does not change and the footer says the form can't open. |
| `submit(&Form, &mut Cx) -> Result<String, String>` | Enter or ⌘↵ in a form | Save a form this module built. Required fields are already non-blank. `Ok(status)`: back to root search with `status` in the footer. `Err(text)`: the form stays, `text` under the fields. |
| `actions(&ItemId, &mut Cx) -> Vec<Action>` | each render of a list, ⌘K | The item's action menu, in display order. Asked again each time, so keep it cheap and current (e.g. "Quit" only while running). Empty: no menu, and no "Actions ⌘K" hint. |
| `act(&ItemId, key, &mut Cx) -> Outcome` | Enter in the action menu; Tab on a list item whose `tab` is `Tab::Act(key)` | Run action `key` (an `Action::key` from `actions`) on the item. `cx.query` is the search text of the list the menu came from. |
| `confirmed(token, &mut Cx) -> Outcome` | the user confirms an `Outcome::Confirm` | Do what `Confirm::token` names. `cx.query` is the search text of the screen the question came from. |
| `on_event(Event, &mut Cx) -> bool` | each event | `true`: this module's views show stale data. A visible view of that module then refreshes. |
| `hotkeys() -> Vec<Binding>` | startup, reload | `Binding { spec, key }`. `key: Err(msg)` reports a binding the module cannot map. |
| `hotkey(key, &mut Cx) -> Option<ListView>` | a bound hotkey press | `Some(view)` toggles the launcher on that view. `None` leaves the launcher alone. |
| `command(&[String], &mut Cx) -> Result<String, String>` | `flick <id> <verb> ...` | `args` starts at the verb. Return `Err(unknown_verb(id, args))` for verbs you do not have. |
| `verbs() -> &'static str` | `flick help` | One line naming `command`'s verbs, e.g. `toy ping`. Empty: none. |

Teardown is `Drop`, not a trait method. A reload drops the instance of a module it disabled
(and the unused fresh instances it built to validate the file), on the main thread, with no
`Cx`. A module that puts something on the system (a menu bar item, an AX observer, the key tap,
a listener, a worker thread) takes it down in `Drop`. Guard that with the module's own "is
up" state, so a never-started instance does nothing. Modules with a `Drop`: `activity`,
`kota`, `keys`, `remote`, `herdr`, `llm`.
State in the store (e.g. `activity`'s recording flag) stays for a module enabled again.

`Cx` gives a module `query` (the search field text; `""` for events, hotkeys and commands),
`store`, `ranker`, and `hide()`. Call `cx.hide()` before an action that needs the previous app
frontmost (open, focus, paste), then return `Outcome::Hide`.

Panic isolation: `Registry::dispatch` (events) and `Registry::command` catch panics per module.
A panicking command returns `"<id>: command panicked"`. `items`, `direct`, `open`, `refresh`,
`activate`, `form`, `submit`, `actions`, `act`, `confirmed` and `hotkey` are not isolated. Report errors as `Outcome::Stay(Some(text))` or
`Err(text)`; do not panic.

Registration order (the `modules!` list in `src/modules/mod.rs`) is significant. It sets root order for
equal ranks, event order, and hotkey bind order. Hotkeys bind in the order launcher, desktop,
switcher, window. When two bindings use the same spec, the later one fails with "bound twice".

## Items, ids and outcomes

`ItemId::new(module, key)` serializes as `<module>:<key>`. That string is the key of the `usage`
table. Do not change how a module builds keys: existing frecency history would split.
`src/characterization/item_ids.rs` pins the current formats: `app:<path>`, `window:<title>`,
`quicklink:<name>`, `script:<name>`, `builtin:<title>`. `with_arg` attaches data (a quicklink's typed query)
that `activate` reads with `id.arg()`. It is not part of the serialized id.

`Registry::activate` routes by `id.module()`, so a view may list another module's item by its
id: the `kota/ask` view's Open Chat row is `message:chat:open`, and Enter (and ⌘K) on it go to
`message`. An unknown module returns `Stay(None)`. Before it
routes, the controller records use (`store.record_use`) for root items and for items in a view
with `record_use = true`.

`Item::tone` (`core::Tone`: `Neutral` default, `Ok`, `Warn`, `Error`) is the status an item's
SF Symbol icon shows: `ui` maps it to `panel::Tone`, drawn as a system green, orange or red
tint that follows light and dark mode (a file icon ignores it). Only the icon is tinted, never
the row's text or background. Use it only for plainly good or bad state (a service up or
down, an agent done or blocked, a task done); leave everything else `Neutral`. A root item
tints only for a problem, so all-well stays quiet in root search.

`Outcome` (`src/core/view.rs`):

- `Hide`: close the launcher.
- `Stay(Option<String>)`: stay; `Some` shows a status line in the footer.
- `Push(ListView)`: a request for a view by `module` and `name`. The registry asks the owning
  module's `open`, so any module can push another module's view by name (`builtin` pushes
  `clip/history`). Views by name, item ids (above) and hotkey keys (status item `Open`
  rows) are the only ways modules refer to each other.
- `Form { module, name }`: a request for form `name` of `module`, by name like `Push`. The
  registry asks that module's `form`, so `builtin` can open the quicklink form without
  importing it.
- `Confirm(Confirm)`: ask before acting. On confirm the registry calls
  `confirmed(token)` on `Confirm::module`; its `Outcome` is applied in turn.
- `ReloadConfig`: reload config.toml, rebind hotkeys, go back to root search.

An `Outcome` from `act` or `confirmed` applies to the list the menu or question came from:
`Stay` returns there (refreshed, same search text and selection) and shows its status;
`Push`, `Form` and `Confirm` replace it; `Hide` hides the launcher.

## Screens

In every list, ⌃J/⌃N and ⌃K/⌃P move the selection like Down and Up, even while the search
field has the keyboard (`platform::panel::keys`; ⌃K no longer kills to the end of the query).
A form takes none of them, so they keep their text-editing defaults there.

`app::State::screen` (`src/app/screen.rs`) is one of:

- `Root`: root search.
- `List(ListView)`: a module's view. There is no `Pop`: Escape and Backspace in an empty field
  go back to root search; a view with `escape_hides = true` hides the launcher on Escape
  instead. A pushed view replaces the current one: one level of views, not a stack.
  A view with `ListView::title` shows that read-only title in place of the search field and
  takes no typing (herdr's Show Output, flick-0b6a). There J/K, ⌃D/⌃U and G/⇧G
  (`Key::LineUp`/`LineDown`/`PageUp`/`PageDown`/`Top`/`Bottom`) scroll `ListView::text` by a
  line, half a page or to an end (`screen::list_nav`, `ui::scroll_text`), or move the
  selection when the view has no text; Up/Down and ⌃J/⌃K always move the selection.
  Tab on a selected item of `Root` or `List` does what its `Item::tab` says: `None` nothing,
  `Activate` the same as Enter (a quicklink's argument), `Act(key)` runs `act(id, key)` as the
  action menu would, with the list's search text. The module names the Tab key in its footer.
- `Actions { target, actions, back }`: ⌘K on a selected item of `Root` or `List` whose
  module returns a non-empty `actions(id)`. The footer of those lists shows
  `Actions  ⌘K` beside the Enter verb when the selected item has actions. In the menu, typing
  filters the actions (fuzzy; module order while the field is empty), Enter runs `act`,
  Escape, ⌘K or Backspace in an empty field go back to `back`: the same screen, search text
  and selection. The action keybinding hints (`Action::shortcut_hint`) are display only.
- `Confirm { confirm, back }`: from `Outcome::Confirm`. `Confirm::title` replaces the search
  field (read-only); `Confirm::rows` are read-only rows (Up/Down, ⌃J/⌃K, J/K scroll; ⌃D/⌃U half a page; G/⇧G top/bottom). Non-destructive:
  Enter or ⌘↵ confirms. `destructive = true`: only ⌘↵ confirms, and plain Enter shows
  `Press ⌘↵ to <label>`. Escape cancels back to `back` and the module hears nothing.
- `Form(Form)`: from `Outcome::Form`. Up to 6 labelled field rows (`FORM_FIELDS` in the
  panel); a `Field::multiline()` field takes 3 rows and wraps, and fields past the last row are
  not shown. Focus on `Form::focused`. Tab and Shift-Tab move focus (wrapping), Enter or ⌘↵
  submits; in a multiline field Return inserts a newline and only ⌘↵ submits, and the footer
  shows `⌘↵` (`Form::submit_hint`). Escape goes back to root search. A blank required field is an inline
  `<Label> is required` error, and `submit` is not called.

`Actions` and `Confirm` hold the screen under them in `back`, so Escape restores it.
`State::show` (every screen change) tracks the module view the screen holds, directly or in
`back` (`screen::hold`), and calls the owner's `closed` when a new screen no longer holds it.
A restore from `back` does not call `open` again, which is why `back` still counts as holding. Module
contract for a confirmable action (e.g. Delete, Uninstall): return
`Outcome::Confirm(Confirm { token: "<verb>/<key>", label: "<Verb>", destructive, rows, .. })`
from `act`, then do the work in `confirmed(token)` and return `Stay(Some(status))` (back to
the list, refreshed) or `Hide`.

## Events and the main thread

`Event` (`src/core/event.rs`): `Started`, `Reloaded`, `LauncherOpened`, `AppActivated { pid }`,
`AppTerminated { pid }`, `PasteboardChanged`, `Wake`, `DisplaysChanged`, `Idle { secs }`, `Active`,
`ModuleChanged { module }`, `Chord { index, down }`, `WindowChanged { pid }`, `Sleep`,
`Locked`, `Unlocked`, `TaskChanged { task }` (the `task` module's running task, posted with
`events::post`), `CardsPending { count, unread }` (cards waiting on the user, unread posts).
Each serializes as
`{"event":"<snake_case>",...}`.
`ModuleChanged` is a module's background thread reporting progress (`events::post`); the
named module's view is stale whatever its `on_event` returns. Root search lists every
module's items, so a visible root search refreshes on any `ModuleChanged` too. Both refreshes
keep the selected item when it is still in the list (`refresh_keeping_selection`).

Sources:

- `platform::events::start` registers `NSWorkspace`/`NSNotificationCenter` observers
  (activation, app termination, wake, screen parameters, and through `events::on_session` sleep, screen
  lock/unlock and fast user switching as `Sleep`/`Locked`/`Unlocked`) and one 0.5 s timer.
  The timer compares the pasteboard change count, and every 10th tick (5 s) checks idle time
  against 60 s. Nothing else polls.
- `platform::axwatch::follow(pid, on_change)` installs one AX observer on an app; the
  `activity` module follows the front app only while recording with `titles` or `urls` on and
  posts `WindowChanged` from the coalesced (1 s trailing) callback. `axwatch::stop` removes it.
  `activity`'s `Drop` (a reload disabled it) stops the follow and hides its recording dot.
- `platform::browser::front_tab_url(bundle)` runs `osascript` and blocks on the browser (and
  on its Automation prompt, killed after 30 s as `Failed`), so `activity` calls it only from
  its one URL worker thread, which leaves the answer in the module's inbox and posts `ModuleChanged`.
- `platform::status_item::show(owner, symbol, tooltip, menu, on_pick)` puts one
  `NSStatusItem` per owner (a module id) in the menu bar (`activity`'s recording dot is owner
  `activity`, KOTA's presence is owner `kota`); `hide(owner)` removes only that owner's item.
  `show` again while shown updates the title and the same `NSMenu` in place, so an open
  menu takes the new rows. Menu rows are typed `Entry`s:
  `Info(text)` (disabled), `Separator`, `Pick { title, key }`, which calls the owner's
  `on_pick(key)`, and `Open { title, module, key }`, which goes to the one opener the
  controller installs at `app::init` (`status_item::set_opener`): it defers through
  `events::on_main` to `app::hotkey(module, key)`, so an `Open` row behaves exactly like that
  module's hotkey press. `on_menu_open(owner, fn())` (`NSMenuDelegate` `menuWillOpen:`) is
  called as the owner's menu opens. Pick handlers and menu-open hooks run inside `AppKit`'s
  event handling, so they only set a flag and post an event (the module reads it on
  `ModuleChanged`), never borrow the state (mx-fcbc43). Items do not activate Flick or
  change its accessory activation policy; their order in the menu bar is up to macOS.
- `app::init` dispatches `Started` to modules, and a config reload to the modules it
  enabled (`Registry::dispatch_to`). It is not published to the socket.
- A config reload then dispatches `Reloaded` to every module (not published either). A
  module that announces state to other modules by event sends it again there, so a module
  the reload just enabled hears it without either importing the other (`message` re-sends
  `CardsPending` for `kota`, flick-b220; `task` re-sends `TaskChanged` for `activity`,
  flick-4bb4).
- `app::toggle_view` dispatches and publishes `LauncherOpened` when root search opens.
- The key tap thread posts `Chord` (see below).
- `platform::notify::on_click(fn(&str))` reports a click on a notification by its id, on the
  main thread. One handler per process (the first `on_click` wins); `herdr` sets it at
  `Started`, queues the id and posts `ModuleChanged`. `notify::post` and
  `request_permission` return `Err` without a `.app` bundle id (`cargo run`, tests), because
  `UNUserNotificationCenter` raises there.
- `ModuleChanged` producers: `flick` (`src/modules/rebuild/`: git check, build runner),
  `activity` (the status item's Stop Recording, its tab URL worker), `herdr` (its I/O
  threads and notification clicks), `capture` (its shutter thread, and the annotation
  editor's `on_done` when it closes), `sys` (its probe, service-check and fleet threads,
  its fleet timer, and its tail and restart threads),
  `dictation` (its recorder and transcription threads, the pill's Esc, and its
  modifier-release poll), `kota` (its poll round, its timer, its ask thread, and its menu
  bar item's picks and menu opens), `llm` (its model-list and reply-stream threads, at most
  one undelivered event at a time, and its chat and private chat windows' handlers; the
  event never carries chat text, `Event` being `Copy`), `app` (its uninstall sizing thread:
  Uninstall… pushes view `app/uninstall`, which lists the paths with "Sizing…" until the
  sizes arrive, and Enter there asks for the confirmation; `app uninstall --dry-run` answers
  later through `core::later`).
- `TaskChanged { task }` is the one link between `task` and `activity`, which never read
  each other's tables. Producer: the `task` module (`src/modules/tasks/`), with
  `events::post` on every start, switch and stop (launcher or CLI), at `Started` when a
  running task was restored and at `Reloaded` while one runs (for an `activity` the reload
  enabled, flick-4bb4), so it arrives on the next main-queue turn, after the current
  dispatch. Consumers: `activity` closes the open span and opens the same app with the new
  task id (`activity_spans.task`); `task` marks its views stale. `flick events` publishes it.
  The `task` timer is its own `core::track::Clock` on `Idle`/`Active`, `Sleep`/`Wake` and
  `Locked`/`Unlocked`, so a task's `task_time` and the activity spans tagged with it agree.
- `CardsPending { count, unread }` is the one link between `message` (which owns the cards) and
  `kota` (its menu bar badge and `kota status`), which never reads the `messages` table.
  Producer: `message` (`src/modules/message/pending.rs`), with `events::post` at `Started`,
  at `Reloaded` (even unchanged, for a `kota` the reload enabled) and after any drain or
  verb that changed the count. A card counts when it is `open` with
  an enabled action (`Card::waits_on_user`), no send, KOTA wait or local run is in flight
  for it, and the user has not dismissed it (a timeout does not count as a dismissal).
  Dismissals are stored (`messages.dismissed`, flick-07cd), so a restart does not count a
  dismissed card again. `unread` counts posts nobody asked for (`message post` without
  `--reply-to`, not pending or partial, outside threads) that the user has not seen: closing
  the card or opening the message list reads them (`src/modules/message/seen.rs`, flick-cb7d).
  The JSON adds the `unread` key; `count` keeps its meaning. Consumer: `kota` keeps both
  (`view::Badge`), shows their sum next to its menu bar glyph and both in the Inbox row and
  `kota status`. `flick events` publishes it.

Flow: observer → `app::on_event` → `Registry::dispatch` (every module, registration order) →
`control::publish` → refresh of a visible stale view. On `DisplaysChanged` the controller
also re-places a visible panel (`ui::place`). The `app` module rescans installed apps on
`LauncherOpened` and `Wake`.

Main-thread rules:

- Controller state is a `thread_local` `RefCell` in `src/app.rs`. Modules live there.
- `AppKit` can deliver a notification while the controller is inside a call. `app::on_event`
  uses `try_borrow_mut`; on contention it re-queues the event with `platform::events::post`.
  Do not call `borrow_mut` on the state from a callback.
- Other threads never touch modules. They use `events::post(event)` or
  `events::on_main(job)`. Both go through `dispatch_async_f` on the main queue. A panic in an
  `on_main` job is logged and does not unwind into libdispatch.
- A control request that finds the state borrowed returns `"Flick is busy; try again"`.
- A verb that only knows its answer after work on a thread calls `core::later::answer_later`
  and hands the `Sender` to that thread. The control layer takes the slot right after the
  verb returns (`later::take`, on the main thread) and its socket thread waits (at most
  `later::MAX_WAIT`, 60 s) for the answer instead of the verb's immediate text; an error from
  the verb, or a dropped sender, keeps the immediate answer. The main thread stays free, so
  the thread may itself send requests to this Flick (`kota ask`). `control::local` drops the
  slot: nothing waits there.
- `app::on_terminate` hooks can run while the controller holds the state (**Quit Flick**
  calls `app::quit` from `activate`). A hook uses only data it owns: `activity` and `task`
  each close their open row through a second `Store` connection to the same file.

Background work (pattern of `src/modules/rebuild/`): slow work (git, cargo, any child
process) never runs on the main thread, because a stalled main thread freezes the launcher
and every hotkey.

- The module starts a named `std::thread` from `on_event`, `activate` or `command`, and
  moves only owned data and an `Arc<Mutex<..>>` into it, never the module.
- The thread writes its result or progress into that shared value, then posts
  `Event::ModuleChanged { module: "<id>" }`.
- `items` and `refresh` read the shared value without waiting (`lock`, not a join). The
  first refresh after the event shows the new state.
- Post `ModuleChanged` only while work runs (the runner posts once a second during a
  build), so an idle Flick has no timers. Rate-limit work started from events (the git
  check runs at most once per 30 s).
- Give every child process a time budget. A command that must answer at once may run a
  short bounded call on the main thread (`flick flick version`: git with a 2 s budget).
- A cached command (`src/modules/sys/`: `sys snapshot`, `sys services`) answers from its
  cache and starts a refresh on a thread, at most once per 5 s. Only the first call with an
  empty cache waits for that thread on a `Condvar`, bounded (2.25 s for the 2 s probe); the
  child itself never runs on the main thread. Nothing refreshes unless a verb asks.
- A polled fleet (`src/modules/sys/poll.rs`, like herdr's remote round): a round starts
  one thread per due machine that is not being read already (a peer's Flick through
  `core::control::PeerHooks`, or `/usr/bin/ssh -o BatchMode=yes -o ConnectTimeout=5
  <target> /bin/sh -s` with the probe on stdin, 10 s budget). `busy` is per machine, so a
  slow ssh read holds only its own machine; each read's end wakes the `Condvar` waiters.
  The round captures an epoch; a reload or `Sleep` bumps it, so late results are dropped.
  It polls on `LauncherOpened`, `Wake`, `sys fleet` and when its view opens, then every
  15 s only while its view shows (each poll leaves one tick thread pending that sleeps and
  posts once, so a fleet of only `via = "local"` machines refreshes too), and on its own
  timer only with `[sys] refresh_secs`; `Sleep` stops the timer. With no
  machines it starts nothing and the root item `sys:fleet` is hidden. `open` of `fleet` or
  `machine` records which one shows; `closed` of that view clears it (a failed push, such
  as Herdr Agents with herdr disabled, changes no screen and so keeps the cadence).
  The fleet window (`sys window`, surface "fleet", `src/modules/sys/window.rs`) counts as
  a visible view while `surface::is_visible` says it shows, so it keeps the 15 s cadence;
  hidden, it adds nothing. Its handlers only queue notes and post `ModuleChanged`.
  Settings, a fleet example and the manual checklist: `docs/sys.md`.

Long-lived I/O threads (`src/modules/herdr/io.rs`): the same rules, for a thread that
follows an external server.

- One thread per stream blocks in `read` with no timeout (the local herdr subscription), so
  idle costs no CPU. On EOF it marks its state and ends; the module starts it again on the
  next `LauncherOpened`, view open or `Wake`, never in a retry loop. An epoch counter in the
  shared state retires an old thread after a reload instead of joining it.
- Polled sources (remote `herdr --machine`) run one round on a thread when due, one child
  per machine in parallel, each with a time budget. A visible launcher keeps polling by
  having the round sleep and post once more; an opt-in timer thread sleeps between posts.
- Hooks (`io::Hooks`, plain `fn` pointers for posting, the clock, the panel and
  notifications) let tests swap in no-ops, so no test reaches the real server or posts a
  real notification.
- State changes that need the main thread (notifications) queue in the shared state and are
  drained in `on_event(ModuleChanged)`.

Timed polls (`src/modules/kota/io.rs`): one round (two children in parallel, each with a
budget) at a time; its result goes through a pure reducer under the lock, then it posts
`ModuleChanged`. On that event the main thread either starts the next round, if due, or
arms one timer thread that sleeps until it is due and posts. Arming bumps a counter, so an
older timer that wakes exits; `Sleep`/`Locked` bump the round epoch too (a round in flight
drops its result) and stop timed rounds until `Wake`/`Unlocked`. The module polls only when
its `[kota]` table sets a key (`Section::is_set`; an empty table counts as none). When
config.toml is shared across Macs, `[kota]` belongs in the KOTA Mac's per-host overlay.

Menu bar item (`src/modules/kota/item.rs`): shown only while the module polls (started, a
`[kota]` key set) and `status_item` is true, so a Mac without `[kota]` has none. Title: the
state glyph plus the `CardsPending` count and unread posts; redrawn only when title, tooltip or rows change,
on `Started`, `ModuleChanged`, `CardsPending`, sleep/wake, lock/unlock and `LauncherOpened`.
Rows: Ask KOTA…, Open Chat and Inbox are `Open` rows (kota hotkeys `ask` and `inbox`; `inbox`
returns a request for the `message` module's `recent` view by name; Open Chat is the `message`
module's hotkey key `chat:open`, which shows the chat window and never hides it), Open
Dashboard and Refresh Now are picks. The pick handler and the menu-open hook only queue a key in `wire.rs` and post
`ModuleChanged`; the module then opens the URL or starts a rate-limited round (as `kota
refresh`). A committed change to down posts one `notify::post` (`notify_down`), never
`notify::on_click`. Dropping the module (disabled on reload) hides its item.

Quick ask (`src/modules/kota/ask.rs`): `kota ask <text>` and the launcher view `kota/ask`
start one `kota-ask` thread. It makes the request id (`k` + base-36 ms + counter), runs this
binary as a client (`message post --pending --id <id> --title KOTA -- <text>`, 2 s, one retry
on busy; the main thread answers it while only the thread waits), then `ssh -o
BatchMode=yes -o ConnectTimeout=8 <ssh> <kota_ask> --id <id>` with the text on stdin (15 s;
never in argv). On an ssh failure it posts `message post --reply-to <id>` with the reason, so
the pending card does not stay. The verb's answer is the outcome, through `core::later`.
Settings, state meanings and the manual checklist: `docs/kota.md`.

KOTA chat (`src/modules/message/chat/`, plan pl-75d3): the `message` module owns it, since
KOTA's replies and cards land in its store. The window is `platform::surface` "chat"
(`Input::Multi`, not `hide_on_blur`), built on the first summon, never at `Started` or in
`configure`. `[message] chat_hotkey` (unset: chat is inert) shows it with the keyboard, or
hides it when it has the keyboard; the hotkey returns no view. The pid in front at summon is
kept, and when the window hides (Esc, ⌘W, the hotkey) that app is made frontmost again
(`ax::make_frontmost`) only if it is still in front. Every surface handler queues a
`chat::session::Note` in a static and posts `ModuleChanged { module: "message" }`; the
module drains them on the main thread after the HUD notes, then redraws the window if it
shows (also after every `message` command, so a `post` into the shown thread appears at
once). Rows: `chat::model::transcript` of `Messages::thread` mapped by `chat::view` onto
`surface::Row`, a card's press state (`dispatch.rs`) folded into its version; card presses
in the window go through the HUD cards' `press`. Keys: ⌘N new thread (`t` + base-36 ms),
⌘[ / ⌘] older/newer (`model::neighbor`), ⌘R resend a failed question, ⌘W hide, ⌘⇧C copy the
newest reply with text (`model::last_reply`, the `copy` hook: `pasteboard::set_text`).
Context chips (`chat/context.rs`, flick-65bd): summoned from hidden, the window reads
`platform::context::front(true)` before it shows and makes app, window-title and selection
chips (on by default, each removable by a click, `Note::Unchip`); a new summon replaces all
chips. ⌘⇧V adds the clipboard text (`pasteboard::copied_text`, one chip), ⌘⇧S a screenshot
(`context::shoot_display` with the window hidden and re-shown; the PNG is read into memory
and the temp file deleted on the shot's worker, then queued as `Note::Shot`; at most 3; no
Screen Recording: a notice, and `capture::request_permission` once per run). Return sends
the chips with the question and clears them; `message ask` sends none. Each screenshot `n`
uploads first, in the ask's worker job, with `attach::upload_argv` (one ssh, the PNG on
stdin, `run::exec_bytes` writing stdin on its own thread, 30 s) to `<[message]
attach_dir>/<req>-<n>.png` on `kota_host`; a failed upload fails the ask and kota-ask does
not run. The `[context]` block (`ask::compose`, capped) names `~/<attach_dir>/<req>-<n>.png`.
A failed question keeps its chips (last 4) for ⌘R, under the same paths.
Asks (`chat/asks.rs`): the question is stored at once as a `role` me message whose id is
the request id, then one worker at a time (FIFO) runs `/usr/bin/ssh -o BatchMode=yes -o
ConnectTimeout=8 <kota_host> <kota_ask> --id <req> --thread <t>` with the question on
stdin (`message::run::exec`, 20 s). A non-zero exit marks the question `failed` and puts the
reason on the window's notice line; a wake (`Env::wake_after`) just past
`model::THINKING_SECS` redraws so an unanswered "Thinking…" goes. KOTA answers with
`message post [--thread <t>] --reply-to <req> --id <x> [--partial]` (a 'me' row is never
pending, so the reply never takes the question); a post or card without a thread takes the
one its id is stored in, else its `reply_to`'s (`thread::inherit`). HUD policy
(`model::alert`): while the window shows a thread, posts and cards to it show no corner card
and play no sound (a corner card already up only redraws in place); other threaded posts
show a card on the first and the final post of an id. Thread cards are transcript rows
(`view::rows` adds the press state by card id and folds it into the row version); their
presses queue `Note::Press` and go through `dispatch::press`, as corner cards' do. Verbs: `message chat [--thread t] [--snapshot <png>]`, `message ask
[--thread t] <text...>` (answers once ssh is done, `core::later`); the launcher view
`message/threads` (⌘K Chat Threads on the root item) lists threads as
`message:thread:<t>`, Enter opens the window on one. All macOS and ssh calls go through
`chat::session::Hooks` (`chat/wire.rs`; fakes in `chat/fake.rs`). User docs, the KOTA-side
contract (ask argv, `[context]` block and caps, screenshot upload, reply and streaming posts)
and the smoke checklist: `docs/message.md#chat`.

Dictation (`src/modules/dictation/`): two child processes and the main-thread steps around
them; every program and macOS call goes through `dictation::Hooks` (`wire::REAL`; scripted
fakes in `fake.rs` for tests). Setup, the model download, privacy, limits and the manual
checklist: `docs/dictation.md`.

- `dictation start` (a keys chord's `on_down`) refuses while the microphone is denied
  (pill "Microphone denied") or a program or model is missing. While the authorization is
  not determined it calls `mic::request` and records nothing, so the first clip is never the
  silence captured while the prompt shows. Else it spawns `rec --buffer 1600 -q -t raw -r
  16000 -e signed -b 16 -c 1 -` and a named reader thread (`recorder.rs`) keeps the PCM in
  memory (no file), publishes each 50 ms buffer's meter level in an `Arc<AtomicU32>` for
  `pill::show(Recording)`, and at `max_seconds` stops `rec` and posts `ModuleChanged`.
- `dictation stop` (`on_up`) discards a hold under `min_hold_ms`; else it records the
  frontmost pid and starts a worker thread (`worker.rs`): SIGTERM the recorder, drain and
  reap it (SIGKILL after 1 s), gate silence (`audio::is_silent`), write the clip to
  `~/Library/Caches/Flick/dictation/<pid>-<epoch>.wav` (dir 0700, file 0600; `clip.rs`),
  run the engine (`engine.rs`: whisper-cli, parakeet-cli or a `command` template; stderr
  closed) within `timeout_secs` and the cancel flag, delete the clip on every outcome
  (`Clip` deletes on drop), clean the text, and push the result to the inbox. `Started` (when on)
  wipes leftover clips. Stop while idle (the key-up after an Esc) answers quietly.
- `on_event(ModuleChanged { module: "dictation" })` takes a queued Esc (`pill::on_cancel`,
  set once before the first recording, only sets a flag and posts), a recorder that ended
  by itself, and the inbox. Results carry the session epoch; a cancelled run's late result
  is stale. Insertion waits, never blocking, for the chord's modifiers to come up
  (`keytap::current_flags`, re-polled by `timer::after(0.02)` posting `ModuleChanged`, at
  most 500 ms), then refuses under `keytap::secure_input` or when the frontmost pid differs
  from the one at chord-up (the text stays in `last`, memory only, and the pill says why);
  else `insert = "paste"`: `pasteboard::snapshot`, `set_transient_text`, `keytap::paste`,
  and after `restore_ms` `restore` only if the change count is still the one
  `set_transient_text` returned; `insert = "type"`: `keytap::type_text`.
- Transcripts and audio are never logged or stored; log lines carry durations and
  character counts.

Model servers (`src/modules/llm/`): OpenAI-compatible HTTP through `/usr/bin/curl`, one
child per call on a named thread (`transport.rs`, `io.rs`); Flick links no HTTP or TLS crate.

- argv: `/usr/bin/curl -q` (first, so `~/.curlrc` is skipped), fixed flags (`-sS -N
  --fail-with-body --noproxy * --proto =http,https --connect-timeout 5 --max-time <s>`)
  and `--url <url>`; a request body goes over stdin (`--data-binary @-`), so prompt text
  is never in argv. A server's `api_key` (flick-d73a) is never in argv either: `-K <file>`
  (after `-q`) names a curl config holding only `header = "Authorization: Bearer <key>"`,
  created new with mode 0600 in `$TMPDIR/flick-llm` (0700, refused if a symlink or open to
  others) by `keyfile::KeyFile`, which removes it on drop once curl is reaped, on a spawn
  failure or a panic. `Server`'s `Debug` prints only whether a key is set. The spawn is an
  `io::Hooks` fn pointer; tests use `testkit`'s fake curls, never a server.
- A streamed reply's SSE events (`openai::Events`: the `data:` lines up to a blank line,
  joined with `\n`, flick-d73a; a new `data:` line after data that already reads as a whole
  chunk ends the event, so a server that skips blank lines still streams) become
  `openai::Piece`s (text, reasoning, finish, usage, done, error) in the stream's inbox in `io::Shared`; the main thread drains it with
  `Shared::take(id)`. `io::cancel(id)` kills curl and keeps what arrived. A watchdog thread
  kills a call still running past its budget. Request and line buffers are zeroed before
  they are freed (best effort), and so are pieces left in an inbox when a stream is dropped.
  Nothing logs prompts or replies.
- Private chat core (`private.rs`, flick-56dc): `PrivateSession` holds the transcript in
  memory only; it has no store handle, no `Serialize` or `Clone` (a test fails to compile if
  one is added) and a `Debug` with counts only. `PrivateSession::open` takes only a
  `private = true` server (`Settings::private`); with none it refuses (`NO_PRIVATE`), never
  falling back to a normal server (`Settings::normal` likewise never returns a private one).
  Text is moved in (`send`, `absorb`), grown into fresh buffers whose outgrown copies are
  wiped, and wiped on `clear` and drop. `Wipe` names when the private chat clears it: window
  closed, `Event::Locked`, `Event::Sleep` (`wipe_on`), config reload, quit, by hand.
- With no `[[llm.servers]]` nothing runs; `llm ping|models` start a fetch only when asked
  and wait for it at most 3.5 s on the main thread (the child runs on its thread).
  `llm ping private [server]` (flick-72b5) is the one verb that reaches a private server:
  the same bodiless `GET /v1/models`, answering only name, time and model count.
- Normal chat (`chat.rs`, `view.rs`, `views.rs`, flick-6a0d): with a normal server the root
  item `llm:chat` (⌘K: `models` view, `threads` view) and `[llm] hotkey` show the
  `platform::surface` "llm" (`Input::Multi`, not private, built on the first summon, never
  at `Started` or in `configure`); the hotkey hides it when it has the keyboard and returns
  no view. As with KOTA chat, the pid in front at summon is made frontmost again on hide if
  it still is in front. Every surface handler queues a `chat::Note` in a static and posts
  `ModuleChanged { module: "llm" }`; `on_event` drains the notes, then `Shared::take`s the
  reply's pieces, then redraws if the window shows. Return sends the whole thread
  (`system_prompt` first) through `openai::chat_body` and `io::chat`; one reply at a time.
  ⌘. is `io::cancel` (the reply so far is kept as stopped), ⌘N a new thread (a reply in
  flight is stopped and kept in its own), ⌘W hide. The model is the `models` view's pick,
  else `default_model`, else the first one `/v1/models` lists (fetched on the first send; the
  prompt waits for a list fetched after it, `Chat::asked`, so an old error does not refuse it). `models` lists `llm:model:<server>/<model>` from `State::models`,
  refetched on open; a server's error row `llm:retry:<server>` refetches. `threads` lists
  `llm:thread:<id>`. All window calls go through `chat::Ui` (`wire::UI`; fakes in
  `testkit::UI`).
- Private chat (`private_chat.rs`, flick-c325): the root item `llm:private` (shown with any
  server) and `[llm] private_hotkey` (bound whenever set) show the private surface
  "llm-private" (`Spec::private = Some("Private - nothing is saved")`, title "Private
  Chat") on a `PrivateSession` in `private_chat::Room` (`Arc<Mutex<Option<Open>>>`). With
  no `private = true` server the item answers `NO_PRIVATE` and the hotkey returns the
  `private` view saying it; nothing falls back to a normal server. Its handlers queue
  `chat::Note`s in their own static. Its functions take no `Cx`, so they cannot reach the
  store; it has no control verb (`llm ping private` only lists models), no notification and no log line; its header and window
  title never hold chat text. Model: `default_model` if the private server lists it, else
  the first listed; a prompt sent with no model waits for a list fetched after it (`Open::asked`). ⌘C with nothing selected copies the last reply with
  `pasteboard::set_text_concealed`. Wipes (drop the session, `io::cancel` its stream and
  wipe the taken pieces, empty rows and input, notice the `Wipe`): hide (⌘W, Esc, hotkey),
  ⌘N, `Event::Locked`/`Sleep`, `configure` (only the running module ever had a session),
  and quit: `wire` installs one `app::on_terminate` hook (and `quit_on_sigterm`) that
  reaches the room and `io::Shared` through `Weak`s in a static and calls
  `private_chat::wipe_for_quit` (`try_lock`, never waits). Window calls go through
  `private_chat::Ui` (`wire::PRIVATE_UI`; fakes in `testkit::PRIVATE`).

### Key tap

`platform::keytap` owns the one active `CGEventTap` (session level, key down, key up,
flags changed). Only the `keys` module uses it (`src/modules/keys/wire.rs`).

- The tap runs on its own thread with its own `CFRunLoop`, so a busy main thread never makes
  macOS time it out and stall keyboard input. The handler runs on that thread: one
  `core::keys::Engine::process` step under a short `Mutex`, then `events::post` of each chord
  edge as `Event::Chord { index, down }`. `index` is the chord's position in
  `[[keys.chord]]`. `Keys::on_event` (main thread) only sends the chord's action to the
  module's FIFO worker thread. `http` and `shell` actions never run on the tap or main thread.
- A `{ flick = "<module> <verb> [args]" }` action is a local control request (words split
  shell-like by `keys/words.rs`, checked by `core::card::Do::check` with `Origin::Local`). `Keys::on_event` runs inside `Registry::dispatch` with the
  controller's state borrowed, so it never calls `app::control`: it hands the words to its
  `keys::Local` hook, a plain `fn(Vec<String>)` that the `modules!` line passes to
  `Keys::new` (`crate::control::local`, like `Remote::new(control::net::HOOKS)`). The hook
  does `events::on_main(|| app::control(&words, Flags::default()))`, so the request runs as
  a local caller right after the current event, and logs an error reply. It skips the
  worker, so a stuck `http` action never delays it; the main queue keeps edge order.
- The tap exists only while the rules are non-empty, from `Event::Started` on. A reload
  installs new rules in place and posts `up` for chords held under the old ones.
- Re-arm: the callback re-enables the tap on a disable notice, and a 5 s timer on the tap
  thread re-enables it or creates it again (creation fails without Accessibility, so a later
  grant needs no restart). After a re-arm, and on `Event::Wake`, `Engine::resync` posts `up`
  for chords no longer held, so push-to-talk never stays on.
- Events posted by `keytap::post_key` (the hyper tap key), `post_combo`, `paste` (cmd+V)
  and `type_text` (`CGEventKeyboardSetUnicodeString`, 20 UTF-16 units per event, never
  splitting a surrogate pair) carry a mark, and the tap passes them untouched. They set
  their flags explicitly, so a physically held modifier never leaks into them (cmd+V stays
  cmd+V under a held shift). `ax::send_paste` is unmarked; dictation uses `keytap::paste`.
- `platform::hid` maps Caps Lock to F18 with `hidutil property UserKeyMapping` for
  `hyper = "caps_lock"`. The list is global: set and clear read, merge and write, and touch
  only Flick's entry. The mapping outlives the process. The first time Flick sets it, it
  installs `app::on_terminate` (clear on quit) and `app::quit_on_sigterm` (SIGTERM quits
  through `terminate:`, so the hook runs). Before that, SIGTERM has its default action.
  A crash skips the hook, so at `Started` without `hyper = "caps_lock"`, `Wire::start` reads
  the list and clears Flick's entry if an earlier run left it set.
- Conflict checks (Hyperkey with the remap, Hammerspoon with chords, a global hotkey on the
  hyper key or a chord key) run in `flick keys status` and after `Started`. `Started` runs
  before the controller binds hotkeys, so `Wire::log_conflicts_later` queues the startup
  log on the main queue (`Sys::later`); it runs once the hotkeys are bound (flick-6968).

## Store

`src/core/store.rs`. One SQLite connection on `~/Library/Application Support/Flick/flick.db`.

- `schema_versions(owner, version)` records how many migrations each owner ran. Owner `core`
  owns `usage`. Each module is an owner under its `id()`.
- `Store::migrate(owner, migrations)` runs the steps not yet run, in one transaction, and
  records the new count. A failed step rolls back all of them.
- Migrations are append-only. Do not edit, remove or reorder a released step: existing
  databases already counted it. Add a step to change a table.
- Migration 1 of `core` and `clip` adopts a pre-versioning table: the original `CREATE TABLE`
  with `IF NOT EXISTS`.
- `app::init` and `State::reload` call `Registry::migrate`. A failure is logged and the app
  continues.
- A module's SQL touches only its own tables, through `store.conn()`. Pattern: a
  `<module>/store.rs` file with `MIGRATIONS` and an extension trait on `Store` (see `Clips` in
  `src/modules/clipboard/store.rs`).
- `messages` (owner `message`, `src/modules/message/store.rs`) holds the HUD history, cards
  and chat threads (plan pl-75d3). Migration 3 adds `thread` (NULL: HUD history), `role`
  (`'me'`: typed in this Mac's chat; NULL: a post), `state` (NULL done, `'partial'`
  streaming, `'failed'` an ask that did not go out) and an index on `(thread, ts)`, and copies
  a stored card's JSON `thread` into the column. `put_message` replaces by id; a replacement of
  a threaded or `partial` row keeps its `ts` and rowid (a stream keeps its place) and its
  thread when the new post names none; any other replacement is a new row at the new `ts`,
  as before. Trimming is per scope: unthreaded rows to `max_history`, each thread to
  `chat_history`, and the thread posted to plus the `chat_threads - 1` others with the newest
  message; a chat post never evicts unthreaded history. Migration 4 adds `unread` (1: a post
  nobody asked for, not seen yet) and `dismissed` (NULL, `'user'` or `'timeout'`: how its
  card left the corner), both written by `put_message` and updated by `seen::Seen`.
- `llm_threads` and `llm_messages` (owner `llm`, `src/modules/llm/store.rs`) hold the normal
  chat (flick-6a0d): a thread's server, model, title and times; each message's `seq`, role
  (`user`, `assistant`), model, body and `state` (NULL done, `'stopped'`, `'failed'` with
  `error`). Written only with `[llm] history` on: a prompt when sent, a reply when it ends.
  Each write moves its thread to the top and drops the oldest threads past `max_threads`
  with their messages. The private chat has no path here.

## Config

`src/config.rs`. File: `$FLICK_CONFIG`, else `~/.config/flick/config.toml`. The first run
writes a commented default (`DEFAULT_CONFIG`).

- Per-host overlay (`src/config/overlay.rs`, pure): `config.<host>.toml` in the same
  directory, `<host>` = `config::host_name()`: `$FLICK_HOST_NAME` (empty: no overlay), else
  `platform::host::local_host_name()` (`scutil --get LocalHostName`, else `hostname -s`),
  normalized by `overlay::normalize` (lowercase, no `.local`, only `[a-z0-9._-]`). Each file
  goes through `parse` on its own (legacy keys map per file), then `overlay::merge` deep-merges
  the overlay's tables over the base's: tables merge recursively, every other value (arrays
  and arrays of tables included) replaces; nothing is deleted. The overlay's `hotkey`
  replaces the base's only when the overlay sets it. No overlay file: the result is exactly
  `parse(config.toml)`. Overlay errors name the overlay path; module-table errors name
  `Config::files` (`config.toml + config.<host>.toml`). No file watching: reload re-reads
  both. `config::edit` and the Raycast import write config.toml only. `flick config path`
  (in-process) prints both paths (`overlay::report`).

- `config.example.toml` (repo root, embedded by `src/config/example.rs`, printed by `flick
  config example`) lists every table and key, commented out, with its default. A setting line
  is `#` then a letter or `[`; a note is `# `. Tests keep it exact: each module with settings
  calls `config::example::assert_documents::<Settings>("<id>")` (also for nested array
  tables, e.g. `"keys.chord"`), which compares the table's keys with the type's serde
  fields; `src/characterization/config_example.rs` checks one table per module id and that
  every module's `configure` accepts the uncommented file. A new or renamed key goes in the
  example in the same commit.

- Top-level `hotkey` is the launcher hotkey. The controller owns it.
- `[launcher]` (`src/config/launcher.rs`) is the launcher's own table, not a module's (no
  `enabled`). The controller owns it: `Config::launcher()` gives `opacity` (default 0.9,
  clamped to 0.6..=1.0, NaN means the default), which `app` passes to `ui::set_opacity` at
  start and on reload. `parse` checks the table (wrong type or unknown key is an error), and
  it stays in the module tables, so the per-host overlay merges it like any table.
- Every other top-level key, `[launcher]` aside, is a module table, `[<module id>]`. `Config::section(id)` returns
  `Ok(None)` for `enabled = false`, else a `Section` without the `enabled` key (empty if the
  table is missing). `enabled` that is not a boolean is an error. `Section::is_set` is true when
  the table sets a key besides `enabled`; a module that stays off until configured (kota,
  dictation) checks it, so a bare `[<module id>]` header turns nothing on.
- A module reads its table with `section.get::<Settings>()` into a private type. Give the type
  `#[derive(Default, Deserialize)]` and `#[serde(default)]`. Errors start with `[<id>]: `.
- `LEGACY` in `src/config.rs` is the only place that knows the old flat keys:
  `desktop_toggle` → `[desktop] hotkey`, `windows_hotkey` → `[switcher] hotkey`,
  `window_keys` → `[window] keys`, `quicklinks` → `[quicklink] links`. If both forms are set,
  arrays join (table entries first) and tables merge. Any other overlap is an error. Do not
  read legacy keys in a module.
- Startup: a bad file (syntax, `hotkey`, legacy keys, `[launcher]`) means the whole default
  config, with a log line. A bad module table does not: `modules::lenient` skips that table
  (its module is configured from an empty table, so it runs on its defaults; a bad `enabled`
  leaves it on) and applies every other table. The errors, named with `Config::files`, are
  logged and shown as root search's footer until a reload succeeds.
- Reload (`Reload Flick Config`, `flick reload`): `modules::reload` builds a fresh registry to
  validate the file. On error nothing changes. Modules still enabled keep their instance and
  get `configure` again. Newly enabled modules start fresh. Migrations run again, the fresh
  modules get `Started`, every module gets `Reloaded`, then hotkeys rebind.
- Writes: `config::edit::edit_entries(module, key, &Edit)` appends, replaces or removes one
  `[[<module>.<key>]]` entry, found by its `name` field. It uses `toml_edit`, so comments,
  blank lines and key order outside that entry do not change. Replace and remove also look in
  the legacy array that `LEGACY` maps there; append always writes the table form. It re-reads
  the file each time, refuses a file or a result that `parse` rejects, writes through a
  symlink, and replaces the file atomically (temp file, fsync, rename). It does not reload:
  the caller updates its own state.

## Control socket and CLI

Protocol: `src/core/control.rs`. Server: `src/control/`. Client: `src/cli/`.

- Socket: `$FLICK_SOCKET`, else `~/Library/Application Support/Flick/flick.sock`, mode 0600. It
  is bound under a temporary name, chmodded, then renamed into place. A stale file is replaced.
  A socket that a live process answers on is an error, not stolen.
- Request: one line, a JSON array of strings, `["<module>","<verb>",args...]`. Reply: one
  line, `{"ok":"<text>"}` or `{"error":"<message>"}`. A connection may send many requests.
- Structured replies: a request whose last word is `--json` asks for JSON. The control glue
  (`control::on_main`) takes that word off and sets `Cx::json` for `Module::command` (false
  for events, hotkeys and the launcher). A module that reads it may return a JSON object or
  array as its `Ok` text; the reply is then `{"ok":<value>}`, re-serialized on one line.
  Any other text, and every reply to a request without `--json`, stays `{"ok":"<text>"}`, so
  modules that ignore `Cx::json` answer exactly as before. Only a trailing `--json` counts.
- Remote callers: a `--remote` word that is last, or just before a trailing `--json`, marks
  a request from a session that may send its output to a remote model (an agent).
  `core::control::split_flags` takes `--json` off first, then `--remote`; `control::on_main`
  passes both as `Flags` to `app::control`, which sets `Cx::remote` (false for events,
  hotkeys, the launcher and `test_cx`). A module may refuse or trim a remote request; a
  module that ignores `Cx::remote` answers exactly as without it. The word is a guard against
  accidents, not a security boundary: any local process can send or omit it.
- `["reload"]` is handled by the controller. Every other request goes to
  `Registry::command`, which matches the first word against module ids.
- `["events"]` turns the connection into an event stream, one JSON object per line. A
  subscriber that falls 256 lines behind is dropped and its socket shut down. One that hangs
  up (or closes its write side) is dropped at once, not at the next event. Publishing costs
  nothing when there are no subscribers.
- Writes: every write to a client, reply or event line, times out after 10 s
  (`server::Limits::write`, on both transports). A client that stops reading is dropped
  then, so it cannot pin its connection thread or event writer forever (flick-646d).
- Threads: one accept thread, one thread per connection, plus a writer thread per event
  subscriber while its connection thread blocks in `read`. A request runs on the main thread
  through `events::on_main`; the socket thread waits for the reply.
- Network transport (`src/control/net.rs`): the same protocol over TCP, off until the remote
  module calls `control::net::HOOKS.apply`. `apply` returns at once and starts or stops on a
  background thread (the latest overlapping apply wins); `status` reads cached state and
  never blocks, so both are safe on the main thread. Both transports share `server::connection`
  through the `server::Stream` trait; `server::Limits` holds what differs (line cap, idle
  timeout, write timeout, whether `["events"]` is allowed). It binds one listener per address `tailscale ip`
  reports that `core::control::is_tailnet` accepts, never a wildcard; loopback is admitted
  only by the test constructor `Net::loopback`. Before reading a request it requires a
  Tailscale peer address and a `tailscale whois` name in `peers`; any failure closes the
  connection and is kept as `NetStatus::last`. Each request is `split_flags`, then forced
  `remote = true`, then checked by `net_policy`. Limits: 64 KiB lines, 30 s idle, 10 s writes,
  16 open connections. Each address binds once (duplicates from `tailscale ip` are dropped);
  a bind that finds the address in use retries for about 0.5 s, and its error names the
  address and, from `lsof`, the process that holds the port. Accept threads wait in
  `platform::poll` on the listener and the read end of a socket pair. Stop: a flag, then
  close the write end of the pair to wake every accept thread, join them (so every listener
  is closed and its port free before a restart binds), then shutdown of every open network
  connection. Stop never connects to a listener: on macOS a connection to this Mac's own
  Tailscale IPv6 address times out (flick-3d4c). `src/control/tailscale.rs` is the `Tailnet` seam: the CLI
  (Homebrew or the app binary) under a 2 s budget, whois cached per address (60 s, errors
  5 s). Tests use fakes and never run the CLI.
- Network policy: `core::control::net_policy` is a deny table, not an allowlist. A network
  caller may not send `reload`, `flick rebuild|cancel`, `keys fire`, `app uninstall`,
  `quicklink add|remove`, `capture` (any verb), `feedback resolve`, `task rm`, `script run`,
  `message card press|focus`, `message chat` (a peer never pops a key window on this Mac or
  writes its snapshot), `message ask` (nor makes this Mac ssh a question to KOTA),
  `dictation` (any verb: a peer never starts the microphone) or
  `kota ask` (a peer, KOTA included, never makes this Mac ssh text to KOTA), `llm` (any verb:
  a peer never makes this Mac send requests to model servers), `sys restart|tail` (a peer
  never restarts this Mac's services or reads its logs), `sys window` (a peer never pops
  the fleet window on this Mac or writes its snapshot), and of `remote` only
  `remote status`. A table verb may be
  several words; it matches a prefix of the words after the module, so `message card press`
  is denied and `message card post` is not. `["events"]` needs `[remote] events = true`. Everything
  else reaches the module with `Cx::remote` set, so module remote guards (activity's grant) still apply.
  `message` (every verb but `card press|focus`, `chat` and `ask`, notably `post` with `--thread`/`--partial`, `card post`, `card spec` and the read-only `threads` and `thread <t>`) is allowed on purpose: peers post messages to this
  Mac's card, which shows text and offers an http(s) link only on a click; `card spec` prints `docs/cards.md`
  (compiled in with `include_str!`), the card contract a peer such as KOTA reads.
  `sys snapshot`, `sys services` and `sys fleet` are allowed on purpose too: they are read-only;
  the first two are how a peer's fleet view reads this Mac (their JSON is the peer contract,
  `src/modules/sys/report.rs`), and `sys fleet` only reads what this Mac's own `[[sys.machine]]`
  polling found: for a remote caller (`Cx::remote`) it starts no round, so a peer never makes
  this Mac ssh or ask its peers.
  The fleet's actions never go through a peer's Flick (flick-4a4c): `sys tail` and
  `sys restart` (denied over the network) run `tail -n 100 -- <log>` and
  `launchctl kickstart -k gui/<uid>/<label>` on this Mac or over this Mac's own ssh, only for
  a service this Mac's config defines with `log` or `restart = true`, and the launcher's
  Restart is a destructive `Confirm` whose row is the exact command. A label, path or command
  from a peer's snapshot never runs.
  Service checks, including `command` argvs, and ssh targets come only from this Mac's config.
  `kota status` (cached presence, no I/O) and `kota refresh` (one read-only herdr + curl
  round, at most one per 10 s) are allowed on purpose too.
  **Adding a verb that changes config, runs code, reads the screen or writes files means
  reviewing `NET_DENIED` in `src/core/control.rs`**; otherwise peers can call it. The
  refusals are pinned in `src/characterization/control_replies.rs`.
- Card local actions (`src/modules/message/{dispatch,local}.rs`): a press is checked again
  with `core::card::Do::check` and the origin stored with the card (`messages.remote`), so a
  peer's card never runs a `flick` verb `net_policy` denies. `script` and `flick` actions
  re-run the Flick binary (`current_exe`, inside the app `Flick.app/Contents/MacOS/Flick`)
  as a CLI client on a worker thread, so they take the normal control path; the main thread
  answers that socket request while only the worker waits, so there is no deadlock. The
  child has no `$FLICK_HOST` and has `$FLICK_REMOTE` set for a peer's card (module remote
  guards apply). `flick` words may not start with a command the CLI runs itself
  (`snapshot`, `import-raycast`, `help`, `config`). `shell` always needs the in-card confirm,
  then runs `/bin/sh -c` in the home folder under a 60 s budget (process group killed).
- Wiring: the `remote` module (`src/modules/remote/`) owns `[remote]` (`peers`, `port`,
  `events`), the on/off switch (`remote_state`, off by default), root item `remote:network`
  and `remote status|on|off`. It never imports `crate::control`: its `modules!` line passes
  `control::net::HOOKS` (`core::control::NetHooks`, plain fn pointers) to `Remote::new`.
  The listener runs iff the switch is on and `peers` is not empty. User guide and manual
  smoke test: `docs/remote.md`.
- Asking a peer from a module: `core::control::PeerHooks` (one plain fn pointer, `ask(host,
  words, flags) -> Result<Reply, String>`) is the client seam. `cli::client::PEER` implements
  it with the `--host` client code (no subprocess): `cli::client::Host::parse`, a 5 s connect
  per resolved address, then 10 s read and write timeouts for the one reply line; it always
  sends `--remote` (the peer's transport forces it anyway) and `--json` when asked. `Err`
  means no reply (bad host, unreachable, timed out, dropped, unparsable); a peer's
  `{"error":...}` is `Ok(Reply::Error)`. Modules never import `crate::cli`: the `modules!`
  line passes `crate::cli::PEER`, as it passes `control::net::HOOKS` to `remote`. It blocks,
  so only background threads call it. The peer applies its own `net_policy`.
- `flick` with no arguments runs the launcher. `flick [--json] <module> <verb> [args]`
  sends a request; `--json` (first or last) sends `--json` as the last request word and
  prints the raw reply line, so `flick --json <module> <verb> | jq .ok` works. With
  `$FLICK_REMOTE` set and not empty, the client sends `--remote` before `--json` (`flick
  events` ignores it). Exit 0 for `ok`, 1 for
  an error reply, 3 (`cli::client::UNREACHABLE`) when no reply came (no connection, host
  unreachable, connection dropped), 2 for usage errors. `flick events` prints the stream.
  `flick snapshot`, `flick import-raycast`, `flick config example` and `flick config path` do
  not use the socket.
- `flick --host <name[:port]> ...` (first, or after a leading `--json`), else `$FLICK_HOST`
  when not empty, sends requests and `flick events` over TCP to the Flick on another Mac
  (`cli::client::Target::Host`; port default `core::control::DEFAULT_PORT`, IPv6 in brackets
  to give a port). The name resolves through `ToSocketAddrs` (MagicDNS), each address gets a
  5 s connect timeout. A host request always sends `--remote`. `--host` with a command that
  runs in this process (launcher, help, snapshot, import-raycast, config example, config path)
  is a usage error;
  `$FLICK_HOST` leaves those alone. An error line instead of the event stream exits 1.
- `--stdin` (`src/cli/stdin.rs`): before a request is sent, locally or to a host, a request
  word equal to `--stdin` is replaced by stdin's contents, verbatim. At most one such word;
  twice, or stdin a terminal, exits 2. Over 16 KiB or not UTF-8 exits 1 and sends nothing.
  Requests without it never read stdin. The server never sees the word, so this is a client
  convenience (keeps card JSON off argv), not part of the protocol.

## Build stamp, install and rebuild

- Stamp: `scripts/bundle.sh` exports `FLICK_BUILD_SHA`, `FLICK_BUILD_DIRTY` (`1`/`0`),
  `FLICK_BUILD_TIME` (RFC 3339 UTC) and `FLICK_BUILD_SOURCE` (checkout path) to cargo. Each
  defaults to the checkout's value; a caller may set it first (an exported tree has no
  `.git`). `src/modules/rebuild/stamp.rs` reads them with `option_env!`; a plain
  `cargo build` has no sha (`dev`). `FLICK_CARGO_ARGS` adds cargo flags.
- Install: `scripts/relaunch.sh [--install <Flick.app>] [--wait-pid PID] [--no-restart]`
  is the only installer. It copies to `.Flick.app.new`, runs `codesign --verify`, and swaps
  with two renames; it restores `.Flick.app.old` when `Flick.app` is missing. Restart stops
  every Flick, waits for each to exit (10 s, then `kill -9`) so the new copy does not exit as
  already running, then uses the launchd agent or `open`. `FLICK_INSTALL_DIR` replaces
  `~/Applications`.
- Rebuild (module `flick`, `src/modules/rebuild/`): one build at a time on a thread.
  A rev build exports the rev with `git archive` to `~/Library/Caches/Flick/rebuild/src`;
  a dirty build runs in the checkout. Both run `bundle.sh` through `$SHELL -lc` with
  `CARGO_TARGET_DIR=<source>/target/flick-rebuild` and `--locked --offline`, optionally after
  `scripts/check-all.sh --bail` (`[flick] gates`). Output: `~/Library/Logs/Flick/rebuild.log`.
  The build and relaunch.sh each run in their own process group: Cancel kills the build's
  group, and the installer outlives Flick when it restarts it. Every build bakes this Flick's
  checkout (`[flick] source`, else its own `FLICK_BUILD_SOURCE`) as `FLICK_BUILD_SOURCE`, also
  for `--source <worktree>`, so the installed app keeps checking that checkout (flick-be14).
- No network: `git::git` allows only `rev-parse`, `rev-list`, `log`, `status`, `archive`
  and `merge-base`, and a unit test pins that list.
- Tests never install or restart the real app: they set `restart = false` and a temp
  `install_dir`, and never run `relaunch.sh` or `bundle.sh --install` without them.

Manual smoke check after a change to bundle.sh, relaunch.sh or the rebuild module. It
restarts the real Flick, so run it by hand, not from tests or agents:

1. Install with `scripts/bundle.sh --install`. `flick flick version` prints the checkout's
   full `HEAD` sha, the build time and `clean`. `defaults read
   ~/Applications/Flick.app/Contents/Info.plist CFBundleVersion` prints `0.0.1+<short sha>`.
2. Make a local commit and open the launcher. **Rebuild Available** shows, and the **Flick
   Version** subtitle reads `1 commit newer in ~/Projects/flick: <subject>`.
3. Leave an uncommitted edit and run **Rebuild Flick**. The build view counts seconds and
   shows the last log line; `flick flick status` prints `building <n>s`. Flick restarts,
   `pgrep -x Flick | wc -l` prints 1, and the version is the new sha, `clean`.
4. Run **Rebuild Flick (Dirty)**. After the restart the version ends in `-dirty`.
5. Add a compile error and run a dirty build. The log opens, the view shows **Build Failed
   (see log)**, and `pgrep -x Flick` and `flick flick version` do not change.
6. Start a build and run **Cancel Build**. `pgrep cargo` prints nothing.
7. With the launcher closed, run `flick flick rebuild` in a terminal. It returns at once
   and Flick restarts on the new build.
8. Repeat step 3 under the launchd agent (`launchctl print
   gui/$(id -u)/org.nix-community.home.flick`) and without it. Each time one Flick runs, and
   with an Apple Development identity, window snap works without an Accessibility prompt.
9. `flick flick rebuild --source <agent worktree>` installs that worktree's `HEAD`, and
   `flick flick version` reports its sha with `source` still the main checkout; **Rebuild
   Available** offers the main checkout's `HEAD`.
10. With no build running, Activity Monitor shows Flick near 0% CPU over 60 s.

## How to add a module

The example adds module `toy` with a hotkey-opened view and a `ping` verb.

1. Create `src/modules/toy/mod.rs`. Its `//!` comment names the module id and the id format
   (`Ids are toy:<key>`).
2. Define `pub struct Toy` with `#[derive(Default)]`. Implement `Module`: `id()` returns
   `"toy"`, plus only the methods you need. Import only `crate::core`, `crate::platform`,
   `crate::config::Section` and `crate::core::store`. Never import another module.
3. Settings: a private `Settings` type with `#[derive(Default, Deserialize)]` and
   `#[serde(default)]`, read in `configure` with `table.get()?`. Keep runtime state outside
   `Settings` because `configure` runs again on reload. Add a commented `#[toy]` table with
   every key and its default to `config.example.toml` (even with no settings: `#enabled =
   true`), and a test calling `crate::config::example::assert_documents::<Settings>("toy")`.
4. Items: build ids with `ItemId::new("toy", key)`. Treat the key format as permanent once
   shipped. In `activate`, match on `id.key()`.
5. Views: return a `ListView` from `open` for each view name you own. Fill `view.items` in
   `refresh` and rank them with `cx.ranker`.
6. Commands: match `args` in `command` and end with `_ => Err(unknown_verb("toy", args))`.
   Return the verbs from `verbs()` (`"toy ping"`) so `flick help` lists them.
7. Tables: put the SQL in `src/modules/toy/store.rs`. Return its `MIGRATIONS` from
   `migrations()`. Name tables after the module.
8. Register it with one line in the `modules!` list in `src/modules/mod.rs`:
   `mod toy => "toy", toy::Toy::default;`. The line declares the directory and registers the
   module. The string is the config table name and must equal `id()`. Its position sets root,
   event and hotkey order.
9. Tests: put unit tests in the module file. Use `crate::core::test_cx` for a `Cx` over an
   in-memory store, and `crate::config::parse` plus `section("toy")` for config.
10. Run `scripts/check-all.sh`.

Nothing else changes: `git diff --stat` shows `src/modules/toy/`, one line in
`src/modules/mod.rs` and the `[toy]` table in `config.example.toml`. The characterization tests pin only the modules they name (root item
prefixes, ranking, tables, hotkey owners, `flick help` verbs), so a new default-enabled module does not break them. A new
table must start with its module id, or `schema_is_usage_and_clips` fails.

Optional: add the module to the module list in `DEFAULT_CONFIG` (`src/config.rs`).
