# Architecture

Flick is one binary crate. Features are modules behind one trait. A grep-based gate
(`scripts/checks/layers.sh`, rules in `scripts/layer-rules.toml`) enforces the layer
boundaries. This file describes the code as it is. When you change a contract below, change
this file in the same commit.

## Layers

| Layer | Path | Role |
|---|---|---|
| platform | `src/platform/` | All `unsafe`, objc2, `AppKit`, CF, AX, `CoreGraphics` and Carbon code. Exposes safe functions: `workspace`, `pasteboard`, `ax`, `spaces`, `screens`, `hotkeys`, `timer`, `panel`, `events`, `app`. Every function runs on the main thread. |
| core | `src/core/` | Plain Rust: `Item`, `ItemId`, `Outcome`, `ListView`, `Module`, `Registry`, `Event`, `Ranker` and frecency, control protocol types (`core::control`). Unit tests run without `AppKit`. |
| modules | `src/modules/<name>/` | One directory per feature. Registered in `src/modules/mod.rs`. |
| ui | `src/ui.rs` | Turns `Item`s into the rows that `platform::panel` draws. Forwards typing and keys to the controller. Knows no feature. |
| controller | `src/app.rs`, `src/root.rs`, `src/hotkey.rs` | `app` holds the registry, the view stack and the selection, and applies `Outcome`s. `root` ranks root search. `hotkey` binds hotkeys and routes presses to the controller. |
| shared services | `src/config.rs`, `src/store.rs` | Config loading and per-module tables; the SQLite store and migrations. |
| control | `src/control/` | The Unix socket server. Runs requests on the main thread. Streams events. |
| cli | `src/cli/` | Argument parsing and the socket client. `snapshot` and `import-raycast` run in-process. |

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
`crate::config::Section` and `crate::store::Store`, so core is free of the platform, not of
the crate. The controller (`app.rs`) may import `crate::modules`; nothing in core or a module may.

## Module contract

`src/core/module.rs`. Every method except `id` has a do-nothing default. All calls happen on
the main thread.

| Method | Called | Contract |
|---|---|---|
| `id() -> &'static str` | always | Unique. It is the config table name, the `ItemId` module part, the store owner and the CLI module word. |
| `migrations() -> &'static [&'static str]` | startup, reload | Store migrations for tables this module owns. Append only. |
| `configure(&Section) -> Result<(), String>` | registration, reload | Read the `[<id>]` table. Keep state such as history across calls. |
| `items(&mut Cx) -> Vec<Item>` | each root refresh | Root search items. Ranked by fuzzy score plus frecency. |
| `direct(&mut Cx) -> Vec<Item>` | each root refresh | Items placed above the ranked results, unranked (quicklink `<keyword> <text>`). |
| `open(view, &mut Cx) -> Option<ListView>` | `Outcome::Push`, hotkey view | Enter a named view this module owns. `None`: no such view; the screen does not change. |
| `refresh(&mut ListView, &mut Cx)` | each keystroke in that view, stale events | Fill `view.items` for `cx.query`. The module ranks its own items (`cx.ranker`). |
| `activate(&ItemId, &mut Cx) -> Outcome` | Enter (Tab when `item.tab`) | Run an item this module created. |
| `on_event(Event, &mut Cx) -> bool` | each event | `true`: this module's views show stale data. A visible view of that module then refreshes. |
| `hotkeys() -> Vec<Binding>` | startup, reload | `Binding { spec, key }`. `key: Err(msg)` reports a binding the module cannot map. |
| `hotkey(key, &mut Cx) -> Option<ListView>` | a bound hotkey press | `Some(view)` toggles the launcher on that view. `None` leaves the launcher alone. |
| `command(&[String], &mut Cx) -> Result<String, String>` | `flick <id> <verb> ...` | `args` starts at the verb. Return `Err(unknown_verb(id, args))` for verbs you do not have. |
| `verbs() -> &'static str` | `flick help` | One line naming `command`'s verbs, e.g. `toy ping`. Empty: none. |

`Cx` gives a module `query` (the search field text; `""` for events, hotkeys and commands),
`store`, `ranker`, and `hide()`. Call `cx.hide()` before an action that needs the previous app
frontmost (open, focus, paste), then return `Outcome::Hide`.

Panic isolation: `Registry::dispatch` (events) and `Registry::command` catch panics per module.
A panicking command returns `"<id>: command panicked"`. `items`, `direct`, `open`, `refresh`,
`activate` and `hotkey` are not isolated. Report errors as `Outcome::Stay(Some(text))` or
`Err(text)`; do not panic.

Registration order (the `modules!` list in `src/modules/mod.rs`) is significant. It sets root order for
equal ranks, event order, and hotkey bind order. Hotkeys bind in the order launcher, desktop,
switcher, window. When two bindings use the same spec, the later one fails with "bound twice".

## Items, ids and outcomes

`ItemId::new(module, key)` serializes as `<module>:<key>`. That string is the key of the `usage`
table. Do not change how a module builds keys: existing frecency history would split.
`src/characterization/item_ids.rs` pins the current formats: `app:<path>`, `window:<title>`,
`quicklink:<name>`, `builtin:<title>`. `with_arg` attaches data (a quicklink's typed query)
that `activate` reads with `id.arg()`. It is not part of the serialized id.

`Registry::activate` routes by `id.module()`. An unknown module returns `Stay(None)`. Before it
routes, the controller records use (`store.record_use`) for root items and for items in a view
with `record_use = true`.

`Outcome` (`src/core/view.rs`):

- `Hide`: close the launcher.
- `Stay(Option<String>)`: stay; `Some` shows a status line in the footer.
- `Push(ListView)`: a request for a view by `module` and `name`. The registry asks the owning
  module's `open`, so any module can push another module's view by name (`builtin` pushes
  `clip/history`). That is the only way modules refer to each other.
- `ReloadConfig`: reload config.toml, rebind hotkeys, go back to root search.

There is no `Pop`. In a view, Escape and Backspace in an empty field go back to root search; a
view with `escape_hides = true` hides the launcher on Escape instead. A pushed view replaces the
current one: there is one level of views, not a stack.

## Events and the main thread

`Event` (`src/core/event.rs`): `Started`, `LauncherOpened`, `AppActivated { pid }`,
`PasteboardChanged`, `Wake`, `DisplaysChanged`, `Idle { secs }`, `Active`. Each serializes as
`{"event":"<snake_case>",...}`.

Sources:

- `platform::events::start` registers `NSWorkspace`/`NSNotificationCenter` observers
  (activation, wake, screen parameters) and one 0.5 s timer. The timer compares the pasteboard
  change count, and every 10th tick (5 s) checks idle time against 60 s. Nothing else polls.
- `app::init` dispatches `Started` to modules, and a config reload to the modules it
  enabled (`Registry::dispatch_to`). It is not published to the socket.
- `app::toggle_view` dispatches and publishes `LauncherOpened` when root search opens.

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

## Store

`src/store.rs`. One SQLite connection on `~/Library/Application Support/Flick/flick.db`.

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

## Config

`src/config.rs`. File: `$FLICK_CONFIG`, else `~/.config/flick/config.toml`. The first run
writes a commented default.

- Top-level `hotkey` is the launcher hotkey. The controller owns it.
- Every other top-level key is a module table, `[<module id>]`. `Config::section(id)` returns
  `Ok(None)` for `enabled = false`, else a `Section` without the `enabled` key (empty if the
  table is missing). `enabled` that is not a boolean is an error.
- A module reads its table with `section.get::<Settings>()` into a private type. Give the type
  `#[derive(Default, Deserialize)]` and `#[serde(default)]`. Errors start with `[<id>]: `.
- `LEGACY` in `src/config.rs` is the only place that knows the old flat keys:
  `desktop_toggle` → `[desktop] hotkey`, `windows_hotkey` → `[switcher] hotkey`,
  `window_keys` → `[window] keys`, `quicklinks` → `[quicklink] links`. If both forms are set,
  arrays join (table entries first) and tables merge. Any other overlap is an error. Do not
  read legacy keys in a module.
- Startup: a bad file or a bad module table means the whole default config, with a log line.
- Reload (`Reload Flick Config`, `flick reload`): `modules::reload` builds a fresh registry to
  validate the file. On error nothing changes. Modules still enabled keep their instance and
  get `configure` again. Newly enabled modules start fresh. Migrations run again, the fresh
  modules get `Started`, then hotkeys rebind.

## Control socket and CLI

Protocol: `src/core/control.rs`. Server: `src/control/`. Client: `src/cli/`.

- Socket: `$FLICK_SOCKET`, else `~/Library/Application Support/Flick/flick.sock`, mode 0600. It
  is bound under a temporary name, chmodded, then renamed into place. A stale file is replaced.
  A socket that a live process answers on is an error, not stolen.
- Request: one line, a JSON array of strings, `["<module>","<verb>",args...]`. Reply: one
  line, `{"ok":"<text>"}` or `{"error":"<message>"}`. A connection may send many requests.
- `["reload"]` is handled by the controller. Every other request goes to
  `Registry::command`, which matches the first word against module ids.
- `["events"]` turns the connection into an event stream, one JSON object per line. A
  subscriber that falls 256 lines behind is dropped and its socket shut down. One that hangs
  up (or closes its write side) is dropped at once, not at the next event. Publishing costs
  nothing when there are no subscribers.
- Threads: one accept thread, one thread per connection, plus a writer thread per event
  subscriber while its connection thread blocks in `read`. A request runs on the main thread
  through `events::on_main`; the socket thread waits for the reply.
- `flick` with no arguments runs the launcher. `flick [--json] <module> <verb> [args]`
  sends a request; `--json` (first or last) prints the raw reply line. Exit 0 for `ok`, 1 for
  an error or no connection, 2 for usage errors. `flick events` prints the stream.
  `flick snapshot` and `flick import-raycast` do not use the socket.

## How to add a module

The example adds module `toy` with a hotkey-opened view and a `ping` verb.

1. Create `src/modules/toy/mod.rs`. Its `//!` comment names the module id and the id format
   (`Ids are toy:<key>`).
2. Define `pub struct Toy` with `#[derive(Default)]`. Implement `Module`: `id()` returns
   `"toy"`, plus only the methods you need. Import only `crate::core`, `crate::platform`,
   `crate::config::Section` and `crate::store`. Never import another module.
3. Settings: a private `Settings` type with `#[derive(Default, Deserialize)]` and
   `#[serde(default)]`, read in `configure` with `table.get()?`. Keep runtime state outside
   `Settings` because `configure` runs again on reload.
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

Nothing else changes: `git diff --stat` shows `src/modules/toy/` and one line in
`src/modules/mod.rs`. The characterization tests pin only the modules they name (root item
prefixes, ranking, tables, hotkey owners, `flick help` verbs), so a new default-enabled module does not break them. A new
table must start with its module id, or `schema_is_usage_and_clips` fails.

Optional: add the module to the module list in `DEFAULT_CONFIG` (`src/config.rs`).
