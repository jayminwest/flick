# Driving the launcher

A driven instance (`flick --drive`) is the real launcher for an agent to drive and read. Nothing
shows on screen. It reads its config from a fixture file, keeps its store in memory, and serves
the normal control socket on a scratch path. So an agent can open views, action menus, confirms
and forms, read each screen as text or as a PNG, and leave the owner's Flick, database and
clipboard alone. Contract: [ARCHITECTURE.md](../ARCHITECTURE.md#control-socket-and-cli).

Check the launcher this way before you ask the owner to look. [hand-check.md](hand-check.md)
lists what a driven instance cannot show.

## Start one

`--drive` needs `FLICK_CONFIG` and `FLICK_SOCKET`, both set and not empty (exit 2 without
them). It exits 1 when a Flick already answers on that socket. Keep the socket path short: a
Unix socket path longer than about 100 bytes does not bind, so use `mktemp -d -t x`, not a
deep scratch folder.

```bash
cargo build
dir=$(mktemp -d -t x)
export FLICK_CONFIG=$dir/config.toml FLICK_SOCKET=$dir/s.sock
export PATH=$PWD/target/debug:$PATH   # this checkout's flick, not an installed one
printf '[quicklink]\nlinks = [{ name = "Docs", url = "https://example.com/{query}" }]\n' > "$FLICK_CONFIG"
flick --drive 2> "$dir/log" &   # "flick: driven on <socket>"
pid=$!
```

Every `flick` command below goes to the driven instance, because `FLICK_SOCKET` is exported.
Stop it with `kill $pid` when you are done.

The fixture config decides which modules run, as `config.toml` does. Some modules are always
off in a driven instance, because they act on the real desktop or hardware
(`modules::DRIVEN_OFF`): desktop, switcher, window, tile, keys, dictation, capture and flick.

## Seed state

Module verbs work on the same socket, so seed state with them before you drive:

```bash
flick task add "Write docs"           # Added 1: Write docs
flick quicklink add Crates 'https://crates.io/search?q={query}'
```

The store is in memory: tasks, clips and activity are gone when the instance stops. Commands
that edit the config, such as `quicklink add`, write to the `FLICK_CONFIG` fixture.
`flick reload` reads the fixture again.

## Drive and read

| Request | Does |
|---|---|
| `flick drive key <name>` | Presses a launcher key: `up`, `down`, `enter`, `tab`, `shift-tab`, `escape`, `backspace`, `cmd-k`, `cmd-q`, `cmd-enter`, `line-up`, `line-down`, `page-up`, `page-down`, `top`, `bottom`. |
| `flick drive type <text>` | Adds text at the end of the focused form field, else of the search field. An error on a screen with a title in place of the search field (a confirm, a titled list). |
| `flick drive field <n\|label> <text>` | Replaces the text of a form field, by 1-based number or label (any case). |
| `flick drive scene` | The screen as text, or `screen: hidden`. |
| `flick drive shot <abs.png>` | Writes the launcher as it stands to a PNG. The path must be absolute. |
| `flick drive wait [ms]` | The scene once it differs from the one the last drive request left, else an error after `ms` (default 5000, at most 60000). |

`key`, `type` and `field` answer `taken` when the launcher handled the input, `passed` when
not (the real panel would do its default, for example `bottom` in the action menu). When the
launcher is hidden, `key` and `type` open root search first, as the launcher hotkey does. So
`drive key escape` on a hidden launcher opens it and then hides it again.

Replies and exit codes are the usual ones: 0 for ok, 1 for an error reply, `--json` for the raw
line. A normal Flick answers every `drive` request with
`drive: only a driven instance answers (flick --drive)`, and no Flick takes one over the
network.

## A session

Open the task list, rename a task through its form, then delete it through the confirm:

```bash
flick drive type tasks       # taken: opens root search, types "tasks"
flick drive key enter        # opens the Tasks list
flick drive scene
# screen: list
# query:
# rows: 1-1 of 1
# > 1 Write docs  [0:00]
# footer: Tasks  ·  ⌘K for more  ·  esc to go back
# action: Start Task  ↵    Actions  ⌘K
flick drive key cmd-k        # the action menu: Start Task, Mark Done, Rename Task, Delete Task
flick drive key down; flick drive key down; flick drive key enter
flick drive scene
# screen: form
# title: Rename Task
# fields:
# > Title: "Write docs" (required)
#   Project: "" (placeholder "Optional: no project")
# footer: ⇥  Next Field
# action: Rename Task  ↵
flick drive field title Draft
flick drive type " docs"     # Title is now "Draft docs"
flick drive key enter        # submits: root search, footer "Renamed Draft docs"
```

A destructive action asks for ⌘↵: `enter` on a confirm only shows "Press ⌘↵ to ...".

```bash
flick drive type tasks; flick drive key enter; flick drive key cmd-k
flick drive key down; flick drive key down; flick drive key down; flick drive key enter
flick drive scene
# screen: confirm
# title: Delete task "Draft docs"?
# ...
# action: Delete Task  ⌘↵
flick drive key cmd-enter
flick task ls                 # check the result with the module's own verb
flick drive shot "$dir/tasks.png"
kill $pid
```

## Read the scene

- Line 1 is `screen: <kind>`: `hidden`, `root`, `list`, `actions`, `confirm` or `form`.
- A list screen then has `title:` or, when it has a search field, `query:`. Then `rows: a-b of n`
  and the rows the panel shows, the selected one marked `>`, each as
  `<n> Title — subtitle  [accessory]` with `(ok)`, `(warn)` or `(error)` for its tone; or
  `empty:` with the empty text. Then `footer:`, `action:` (the hints), and `text:` for a text
  block.
- A form has `title:`, then `fields:` with the focused field marked `>`, `error:` when the last
  submit failed, then `footer:` and `action:`.

Read the footer after a key: results such as "Renamed ..." or "Deleted ..." show there.

## Background answers

Some answers come later than the reply to a key (a module's `ModuleChanged`, an async form).
Use `flick drive wait` to get the next scene, or read `drive scene` again. `wait` compares with
the scene the last drive request left, so it waits only for changes that no request caused.

## Limits

- The fixture config can turn on modules that read live data: the apps scan reads the real
  `/Applications`, and herdr, sys and kota read their real sources when the fixture turns them
  on. Keep the fixture to what you test.
- Pasteboard writes, pastes, opening apps and URLs, the Trash and the Caps Lock remap do
  nothing or answer an error. A test that needs them is a [hand check](hand-check.md).
- `scene` and `shot` show the launcher panel only.
