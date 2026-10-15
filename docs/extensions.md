# Extensions: protocol v1

An extension is a program, in any language, that adds root items, views, actions and forms to the launcher. Flick runs the program once for each call. It writes one JSON request to the program's stdin and reads one JSON reply from its stdout. This page is the contract. The code wins where this page and the code disagree: `src/modules/ext/`, with the wire shapes in `src/modules/ext/proto.rs` and `src/modules/ext/proto/`.

## Set up

Add one `[[ext.extensions]]` table for each extension, then run **Reload Flick Config**:

```toml
[[ext.extensions]]
name = "hello"                         # 1-32 of a-z, 0-9 and -; unique
argv = ["~/bin/hello-ext"]             # no shell; a leading ~/ is the home folder
timeout_ms = 2000                      # the default
hotkeys = { greetings = "cmd+ctrl+alt+shift+KeyG" }   # <view> = "<hotkey>"
keywords = ["hi"]                      # "hi <text>" in root search sends op: keyword
subscribe = ["wake"]                   # events it gets, as op: event
```

| Key | Rule |
|---|---|
| `name` | Required. 1-32 of `a-z`, `0-9` and `-`, unique. It is the prefix of every error and of item ids (`ext:<name>/<key>`). A new name loses the usage history of its items. |
| `argv` | Required. The program and its arguments. Flick runs it without a shell. A leading `~/` in any argument is the home folder. Flick does not check that the program exists: the first call fails instead. |
| `timeout_ms` | Time one call may take, in milliseconds. Default 2000; 0 is an error. `act` and `submit` get at least 10 s. |
| `hotkeys` | Global hotkeys that open the extension's views, `<view> = "<hotkey>"`. A view name is 1-64 of `A-Z a-z 0-9 . _ -`. A blank hotkey is left out. |
| `keywords` | Words that send `<keyword> <text>` in root search to the extension (`op: keyword`). At most 16, each one word of 1-80 chars, unique among all extensions. |
| `subscribe` | Events the extension gets (`op: event`): `started`, `reloaded`, `wake`, `unlocked`, `launcher_opened`. |

A bad entry fails the config load with `[ext] extensions: "<name>": <why>`. A reload stops the worker of an extension whose entry changed or went away, after the call that runs.

## How a call runs

- **One process per call.** Flick runs `argv` in a new process group. It writes the request, one JSON object and a newline, to stdin and closes stdin. The program must write one JSON object to stdout and exit 0. Whitespace around the object is allowed; a second object is not.
- **Environment.** The program gets Flick's environment, plus `FLICK_EXT` (its `name`), `FLICK_SOCKET` (this Flick's control socket) and `FLICK_VERSION` (Flick's version). `FLICK_HOST` is removed, so a `flick ...` command that the program runs talks to this Mac's Flick. A Flick started by launchd or from the app has a short `PATH`: use full paths for tools outside `/usr/bin` and `/bin`.
- **Working directory.** The program starts in Flick's working directory. For `Flick.app` that is `/`. Use absolute paths for files.
- **One at a time.** Each extension has its own worker thread. Its calls run one at a time, in the order Flick queued them. The launcher never waits for a program: it shows what it has and updates when the reply comes.
- **Timeout.** A call that runs longer than its time is stopped: Flick kills the process group, so the program and the processes it started stop. The time is `timeout_ms`, and at least 10 s for `act`, `submit` and `flick ext call`.
- **Stdin and stdout.** Flick writes stdin on a separate thread, so a program that does not read stdin still works. Flick reads at most 1 MiB of stdout and keeps the last 4 KiB of stderr.

## Requests

Every request has `"v": 1` and an `op`:

| `op` | Other keys | When | Reply |
|---|---|---|---|
| `root` | | the launcher opens on root search, a config reload, `flick ext refresh`, a stale item | [items](#items) |
| `open` | `view` | a view opens (hotkey, `push` action, `flick ext open`) | a [view](#views) |
| `query` | `view`, `query` | a keystroke in a `"filter": "ext"` view | items |
| `keyword` | `keyword`, `query` | `<keyword> <text>` in root search | items |
| `act` | `view` (null in root search), `item`, `action` | an `ext` action runs | [done](#act-and-submit-replies) |
| `form` | `form` | a form opens | a [form](#forms) |
| `submit` | `form`, `values` (field key to text) | a form is submitted | done |
| `event` | `event` | a [subscribed event](#events) | anything; Flick ignores it |

Examples:

```json
{"v":1,"op":"root"}
{"v":1,"op":"open","view":"greetings"}
{"v":1,"op":"query","view":"greetings","query":"wor"}
{"v":1,"op":"keyword","keyword":"hi","query":"Ada"}
{"v":1,"op":"act","view":"greetings","item":"world","action":"greet"}
{"v":1,"op":"act","view":null,"item":"hello","action":"open"}
{"v":1,"op":"form","form":"new"}
{"v":1,"op":"submit","form":"new","values":{"name":"Ada"}}
{"v":1,"op":"event","event":"wake"}
```

`view` is the extension's own view name, without `<name>/`. `submit` sends the text of every field, empty ones too.

## Replies

Any reply may carry `"error": "<text>"`. A non-blank `error` makes the call fail with that text. `"v"` is optional; a value other than 1 is read as v1, with a warning.

Flick refuses the whole reply only when it can show nothing sensible:

- the reply is over 1 MiB, is not JSON, or is not a JSON object;
- a top-level key has the wrong type: `items` or `fields` is not a list, or `title`, `placeholder`, `status`, `submit` or `error` is not a string or holds a control character;
- a form has no `title` or no valid field.

Everything else degrades with a warning: Flick skips a bad item, action or field, drops entries past a cap, cuts long text, and uses the default for an unknown icon, `tone`, `after` or `filter`. Warnings go to the log only (see [Errors](#errors)).

### Items

A reply to `root`, `query` and `keyword` is `{"items": [ … ]}`. A view has `items` too.

```json
{"key": "world", "title": "Hello, World", "subtitle": "…", "accessory": "…",
 "icon": "hand.wave", "keywords": ["earth"], "tone": "ok",
 "actions": [ … ], "tab": "copy"}
```

| Key | Rule |
|---|---|
| `key` | Required. 1-128 chars, not blank, no control characters. Unique in the reply: a repeated key is skipped. Flick never cuts a key. |
| `title` | Required, not blank. Cut to 200 chars. |
| `subtitle` | Cut to 500 chars. |
| `accessory` | Short text on the right. Cut to 80 chars. |
| `icon` | See [Icons](#icons). Default: `puzzlepiece.extension`. |
| `keywords` | Extra words that Flick's search matches. At most 16 (the rest are dropped), each a string of at most 80 chars. Another type, or a word that is too long, skips the item. |
| `tone` | `neutral` (default), `ok` (green), `warn` (orange), `error` (red): the icon's tint. |
| `actions` | See [Actions](#actions). Without `actions`, the item gets one action: `{"key": "open", "title": "Open"}` (`do: ext`, `after: hide`). A list with no valid action skips the item. |
| `tab` | The key of the action that Tab runs. A key that names no action is ignored. |

Enter runs the first action, ⌘K lists all of them, and Tab runs the one `tab` names. The row shows the first action's title as its verb.

An item's id is `ext:<name>/<key>`. Flick counts its use for ranking by that id, so keep a key stable while the item is the same thing.

### Icons

- `"star.fill"` or `{"symbol": "star.fill"}`: an SF Symbol name, 1-100 chars of `a-z`, `0-9` and `.`.
- `{"file": "/abs/path/icon.png"}`: an absolute path to an image file of at most 1024 chars, with the extension png, jpg, jpeg, gif, tiff, heic, webp, icns or pdf. The file must exist and start like an image of that kind. If it does not, Flick uses the default icon and logs a warning.

### Actions

```json
{"key": "copy", "title": "Copy", "icon": "doc.on.doc",
 "do": {"copy": "Hello, World"}, "after": "hide", "confirm": "Copy it?"}
```

| Key | Rule |
|---|---|
| `key` | Required. 1-64 of `A-Z a-z 0-9 . _ -`. Unique on the item. `act` sends it back. |
| `title` | Required, not blank. Cut to 200 chars. |
| `icon` | As for items. Default: `bolt`. |
| `do` | What the action does. Default `ext`. See below. An invalid `do` skips the action. |
| `after` | What the launcher does when the action starts: `hide` (default), `stay` or `root` (back to root search). |
| `confirm` | A question Flick asks first. The confirm button has the action's title. Cut to 500 chars. |

At most 16 actions on an item; the rest are dropped.

#### `do`

`do` is a string or an object with exactly one key:

| `do` | Runs |
|---|---|
| `"ext"`, or no `do` | sends `op: act` to the extension. |
| `{"push": "<view>"}` | opens the extension's view `<view>` (`op: open`). `after` does not apply. |
| `{"form": "<form>"}` | opens the extension's form `<form>` (`op: form`). `after` does not apply. |
| `{"open_url": "https://…"}` | opens an http or https link. |
| `{"open_app": "Safari"}` | opens an app by name or bundle id, never a path. |
| `{"copy": "text"}` | copies the literal text. |
| `{"script": {"name": "deploy", "query": "prod"}}` | runs the `[[script.commands]]` entry `name`, with `query` if given. |
| `{"flick": ["task", "start", "x"]}` | runs one `flick` request: module, verb, arguments. |
| `"dismiss"` or `{"dismiss": true}` | hides the launcher, whatever `after` says. |

View and form names are 1-64 of `A-Z a-z 0-9 . _ -`. The card actions (`open_url` to `dismiss`) pass the same policy as a local card's buttons ([cards.md](cards.md)): a `flick` request must start with a module (not `events`, `snapshot`, `import-raycast`, `help` or `config`) and may not hold `--host`, `--json`, `--remote` or `--stdin`.

`{"shell": …}` is refused, and the action is skipped. An extension is already a program: it runs commands itself, in `act`.

`script` and `flick` run Flick's own command line on a separate thread, for at most 30 s. The last line it prints is the action's status, as for an `act` reply. Its error reads `<name>: flick <words>: <why>`.

#### `after`

`after` applies at once, before any reply (optimistic). With `hide` the launcher closes; with `root` it goes back to root search; with `stay` it stays. For an `ext`, `script` or `flick` action that stays in a view, the view's footer shows `<title>…` until the reply comes. For `open_url`, `open_app` and `copy`, Flick hides the launcher first, then runs the action.

### Views

A view is `<name>/<view>`. A hotkey, a `push` action or `flick ext open` opens it. The reply to `open`:

```json
{"title": "Greetings", "placeholder": "Search greetings…", "filter": "flick", "items": [ … ]}
```

| Key | Rule |
|---|---|
| `title` | The footer text. Default: the extension's name. Cut to 200 chars. |
| `placeholder` | The search field's placeholder. Default: `Search <name>…`. Cut to 80 chars. Flick reads it when the view opens, so a new placeholder shows from the next open. |
| `filter` | `flick` (default): Flick ranks the items as you type, by its fuzzy match on the title and keywords, and usage. `ext`: the extension searches (see [Search](#search)). |
| `items` | The view's items. |

Flick keeps the last good reply for each view. A view shows that reply at once, or `Loading…` the first time, and sends `op: open` again each time it opens.

### Forms

The reply to `form`:

```json
{"title": "New Greeting", "submit": "Greet",
 "fields": [{"key": "name", "label": "Name", "value": "", "placeholder": "Ada",
             "required": true, "multiline": false}]}
```

| Key | Rule |
|---|---|
| `title` | Required, not blank. Cut to 200 chars. |
| `submit` | The submit button label. Default `Submit`. Cut to 80 chars. |
| `fields` | At least one valid field, at most 6; the rest are dropped. |
| field `key` | Required. 1-64 of `A-Z a-z 0-9 . _ -`, unique. Its text comes back under this key in `values`. |
| field `label` | Required, not blank. Cut to 80 chars. |
| field `value` | The initial text. Cut to 4000 chars. Only a `multiline` field's value may hold newlines and tabs. |
| field `placeholder` | Cut to 80 chars. |
| field `required` | Flick does not submit while this field is blank. |
| field `multiline` | A multi-line text field. |

Flick keeps the last good reply for each form. A form shows that reply at once, or empty with `Loading…` the first time, and sends `op: form` again each time it opens. A later reply changes only the cache, so it never replaces what the user typed. A form cannot be submitted while it loads, or after its `form` call failed.

On submit, the form stays open with `Saving…` until the reply comes. Then:

- a reply without `error` closes the form to root search, with the reply's `status` (or `Saved`) in the footer;
- an `error` shows under the fields, and the user can fix the text and submit again;
- if the user left the form or closed the launcher before the reply came, the status shows as a toast (see below). An error always toasts.

### `act` and `submit` replies

```json
{"status": "Hello from world", "refresh": true}
```

- `status`: a line for the user, cut to 500 chars. After `act`, it replaces the footer of the view the action ran in while that view shows. Else it shows as a toast: a card in the top right corner for 6 s, with the extension's name as its header. A newer toast of the same extension replaces it.
- `refresh`: `true` makes Flick send `root` again, and `open` for the extension's view on the screen.

An empty reply, `{}`, is a success with no status.

## Limits

| What | Limit | Past it |
|---|---|---|
| reply | 1 MiB | the reply is refused |
| items in a reply | 500 | the rest are dropped |
| actions on an item | 16 | the rest are dropped |
| fields in a form | 6 | the rest are dropped |
| keywords on an item | 16 | the rest are dropped |
| item key | 128 chars | the item is skipped (keys are never cut) |
| action, field, view and form names | 1-64 of `A-Z a-z 0-9 . _ -` | the action or field is skipped |
| title (item, action, view, form) | 200 chars | cut, ending in `…` |
| subtitle, status, confirm, error | 500 chars | cut |
| accessory, keyword, placeholder, label, submit label | 80 chars | cut (a keyword that is too long skips its item) |
| field value | 4000 chars | cut |
| SF Symbol name, icon file path | 100, 1024 chars | the icon is ignored |

Text never holds control characters (see [Errors](#errors)). One in a top-level key refuses the reply; one in an item, action or field skips it.

## Search

**`"filter": "ext"` views.** Each keystroke sends `op: query` with the search text. The view shows the items of the reply in the extension's order; Flick does not rank them. Until the reply comes, the view keeps the last answer to a query of that view, else the `open` items. An empty search text sends no query and shows the `open` items.

**Keywords.** `<keyword> <text>` in root search, with a word from the extension's `keywords` and text that is not blank, sends `op: keyword` with the text trimmed. The reply's items show above the ranked results.

**Newest wins.** A query waits 60 ms on the worker before it runs, and runs only if no newer query came. A newer query kills the process that answers an older one. An empty search text, another screen or a config reload stops the query that runs. Only the reply to the newest query shows.

Each query starts a new process. An interpreted program (Python, Node, Bun) can take 30-100 ms to start. For a list of a few hundred items or less, use `"filter": "flick"`: Flick ranks the items itself and starts no process per keystroke.

## Events

`subscribe` names the events an extension gets, as `{"v":1,"op":"event","event":"<kind>"}`:

| Event | When |
|---|---|
| `started` | Flick started. |
| `reloaded` | a config reload. |
| `wake` | the Mac woke from sleep. |
| `unlocked` | the screen unlocked. |
| `launcher_opened` | the launcher opened on root search (not on a hotkey view). |

Events are fire-and-forget. Flick ignores the reply, and logs a failed event call. An event waits behind every other call of that extension: it runs only when no other call is queued, so a user's action or search waits at most for the event that runs already. Events coalesce per kind: while one of a kind waits, another of that kind is dropped.

## Errors

Every error starts with the extension's name: `<name>: <why>`.

| Why | Cause |
|---|---|
| the last non-blank line of stderr | the program exited with a code other than 0 |
| `exit <n>` | the same, with nothing on stderr |
| `killed by a signal` | a signal stopped the program, with nothing on stderr |
| `no reply in <ms> ms; stopped` | the call ran out of time; Flick killed the process group |
| `<program>: <os error>` | the program could not start (not found, not executable) |
| `reply over 1048576 bytes` | stdout was over 1 MiB |
| `reply is not JSON: …`, `reply is not a JSON object` | stdout was not one JSON object |
| `<key> is not a string`, `<key> has a control character`, `items is not a list` | a top-level key had the wrong type |
| the reply's `error` text | the reply had a non-blank `error` |
| `that item is gone; try again` | an action ran on an item that is no longer in Flick's cache; Flick sends `root` again |

Control characters are C0 and C1 controls, DEL, U+200B-U+200F (zero-width and direction marks), U+2028-U+202E (line and paragraph separators, direction embeddings and overrides) and U+2066-U+2069 (direction isolates).

Flick logs every failed call to its stderr, as `flick: ext: <name>: <why>`. Run `~/Applications/Flick.app/Contents/MacOS/Flick` from a terminal to see the log. It also logs parse warnings, at most 5 for each reply and then a count. Where else an error shows:

| Call | Shows |
|---|---|
| `root` | a notification, *Flick extension failed* |
| `open` | the footer of the view while it shows, else a notification |
| `query` | the footer of its view while it shows; never a notification, since it can fail at each keystroke |
| `keyword` | the log only |
| `act`, and card actions | a toast, always (also after `after: hide`), and the footer of the view while it shows. Flick then sends `root` and `open` again, because the item may be gone. |
| `form` | a toast, and in the form while it shows |
| `submit` | a toast, and under the form's fields while it shows |
| `event` | the log only |

Notifications need permission: macOS asks the first time Flick posts one.

## Command line

```bash
flick ext ls                     # <name>\t<op>\tok|error\t<ms> ms[\t<error>]; <name>\t- before the first call
flick --json ext ls              # [{"name":"hello","last":{"op":"root","ok":true,"error":null,"ms":41}}]
flick ext refresh hello          # send root again, and open for its view on the screen
flick ext open hello greetings   # open view hello/greetings, as its hotkey does
flick ext call hello '{"v":1,"op":"root"}'
printf %s "$json" | flick ext call hello --stdin
```

- `ext ls` shows how each extension's last call went: its op (`call` for `ext call`), ok or the error, and the time from the spawn to the reply. It does not wait for a call that runs. A query that a newer one replaced does not count.
- `ext open` works like a press of a hotkey bound to the view: if the launcher shows that view already, it hides.
- `ext call` is for debugging. It sends the text as it is (it must be a JSON object; Flick adds nothing) on the extension's worker, behind its other calls. It prints the program's stdout without checking it, so it shows a reply that Flick would refuse. A failed exit or a timeout is an error. The verb waits at most 60 s.
- Over the network, a peer may send `ext ls` and `ext refresh`, but not `ext open` or `ext call`: a peer must not open the launcher on this Mac or run an extension with a request of its choosing.

## Example extension

This Python script answers every op. Save it as `~/bin/hello-ext`, run `chmod +x ~/bin/hello-ext`, add the `[[ext.extensions]]` table from [Set up](#set-up), and reload.

```python
#!/usr/bin/env python3
# A minimal Flick extension. Flick runs it once per call: one JSON request on stdin,
# one JSON reply on stdout, exit 0.
import json
import sys
import time

if "--slow" in sys.argv:
    time.sleep(2)  # for the manual checklist: a slow extension
if "--crash" in sys.argv:
    sys.exit("crashed on purpose")  # the last stderr line, exit 1

req = json.load(sys.stdin)
op = req["op"]


def greeting(key, name):
    return {
        "key": key,
        "title": f"Hello, {name}",
        "icon": "hand.wave",
        "tab": "copy",
        "actions": [
            {"key": "greet", "title": "Greet", "after": "stay"},  # do: ext (the default)
            {"key": "copy", "title": "Copy", "do": {"copy": f"Hello, {name}"}},
            {"key": "new", "title": "New Greeting…", "do": {"form": "new"}},
        ],
    }


if op == "root":
    reply = {"items": [{
        "key": "hello",
        "title": "Greetings",
        "subtitle": "Say hello from an extension",
        "icon": "hand.wave",
        "actions": [{"key": "open", "title": "Open", "do": {"push": "greetings"}}],
    }]}
elif op == "open":
    reply = {"title": "Greetings", "placeholder": "Search greetings…",
             "items": [greeting("world", "World"), greeting("flick", "Flick")]}
elif op == "keyword":
    reply = {"items": [greeting("typed", req["query"])]}
elif op == "act":
    reply = {"status": f"Hello from {req['item']}"}
elif op == "form":
    reply = {"title": "New Greeting", "submit": "Greet",
             "fields": [{"key": "name", "label": "Name", "required": True}]}
elif op == "submit":
    name = req["values"]["name"].strip()
    if name == "error":
        reply = {"error": "that name is taken"}
    else:
        reply = {"status": f"Hello, {name}"}
else:
    reply = {}  # event, or an op this script does not know: Flick ignores the reply

json.dump(reply, sys.stdout)
```

Try it from a shell first:

```bash
echo '{"v":1,"op":"open","view":"greetings"}' | ~/bin/hello-ext
flick ext call hello '{"v":1,"op":"form","form":"new"}'
```

Any language works the same way: read stdin to its end, parse one JSON object, print one JSON object, exit 0.

## Manual checklist

Unit tests cover the parser, the transport with a shell fixture (`tests/fixtures/ext-echo.sh`), the worker queue, the caches, actions, forms, queries and events with fake hooks. This checklist covers the real launcher, the HUD and notifications. Use the example above, on the installed Flick.app.

- [ ] **Root item.** Open the launcher and type `greet`: **Greetings** shows, with the hand icon. `flick ext ls` shows `hello	root	ok	<ms> ms`.
- [ ] **View.** ↵ on **Greetings** opens the view: the footer says `Greetings`, and the rows are **Hello, World** and **Hello, Flick**. Type `fli`: only **Hello, Flick** remains.
- [ ] **Hotkey and CLI.** The `hotkeys` entry opens the same view. `flick ext open hello greetings` opens it, and a second time hides the launcher.
- [ ] **Actions.** In the view, ↵ on **Hello, World** runs **Greet**: the launcher stays, the footer shows `Greet…` and then `Hello from world`. Tab copies `Hello, World` and hides the launcher. ⌘K lists Greet, Copy and New Greeting….
- [ ] **Toast.** Type `hi Ada` in root search: **Hello, Ada** shows above the ranked results. ↵ runs **Greet**: a `hello` card in the top right says `Hello from typed` and goes away after about 6 s.
- [ ] **Form.** ⌘K → **New Greeting…** opens the form (`Loading…` the first time). Type `Ada` and press ↵: `Saving…`, then root search with `Hello, Ada` in the footer.
- [ ] **Form error.** Submit the name `error`: the form stays, `hello: that name is taken` shows under the field, and a toast shows the same. Change the name and submit again: it saves.
- [ ] **Slow extension.** Set `argv = ["~/bin/hello-ext", "--slow"]` and reload. Open the launcher at once: it opens without delay and shows the cached items, and typing, ↑ ↓ and other modules work while the extension sleeps. Open **Greetings**: it shows its cached rows at once (or `Loading…` after a restart), and the new reply comes about 2 s later. Submit the form: `Saving…` shows for about 2 s, and the launcher keeps working.
- [ ] **Timeout.** With `--slow`, set `timeout_ms = 500` and reload. Open the launcher: a notification says `hello: no reply in 500 ms; stopped`, and `flick ext ls` shows `hello	root	error	… ms	hello: no reply in 500 ms; stopped`. `pgrep -f hello-ext` finds no process left.
- [ ] **Failed exit.** Set `argv = ["~/bin/hello-ext", "--crash"]` and reload. Open the launcher: a notification says `hello: crashed on purpose`. Open **Greetings** with its hotkey: the footer says the same, and no second notification shows.
- [ ] **Form left while saving.** With `--slow`, submit the form and press Esc at once: about 2 s later a toast says `Hello, <name>`.
- [ ] **Network.** From a peer: `flick --host my-macbook-pro ext ls` answers. `... ext open hello greetings` and `... ext call hello '{"v":1,"op":"root"}'` print `not allowed over the network` and exit 1.
