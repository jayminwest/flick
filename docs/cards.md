# Cards: the KOTA spec (schema v1)

A card is a message with structure: a title, body blocks and buttons, posted to a Mac's Flick as JSON and drawn in the corner HUD without taking focus. A press either runs a small local action on that Mac or goes back to KOTA, which answers by re-posting the card. This page is the contract; Flick prints it with `flick --host <mac> message card spec`. User-side settings are in [message.md](message.md). The code wins where this page and the code disagree: `src/core/card.rs`, `src/core/card/`, `src/modules/message/`.

## Post a card

```sh
printf %s "$json" | flick --host jaymins-macbook-pro message card post --stdin
printf %s "$json" | flick --host jaymins-macbook-pro --json message card post --stdin
```

- `--stdin` is replaced by stdin, verbatim (use `printf %s`, not `echo`), at most 16384 bytes; it keeps the JSON off argv. It may appear once; stdin that is a terminal is refused.
- Text reply (exit 0): the card id, then one `warning: <text>` line per warning.
- `--json` reply (exit 0): `{"ok":{"id":"c1","replaced":false,"warnings":[]}}`. `replaced` is true when a card with that id was already stored or `reply_to` took a pending message. An error is `{"error":"<message>"}` on stdout, exit 1.

| Exit | Meaning | What to do |
|---|---|---|
| 0 | Flick took the card (it may still carry warnings) | read the warnings once, fix the generator |
| 1 | Flick answered with an error: `invalid card: <reason>`, a refusal (`<module> <verb>: not allowed over the network`), `No card <id>`, or `--stdin` input over 16 KiB / not UTF-8 | fix the request; do not retry as is |
| 2 | usage: a bad command line, `--stdin` twice, stdin a terminal | fix the call |
| 3 | no reply: Flick not running, host unreachable, connection dropped | fall back (iMessage) |

## Schema v1

Top-level keys; any other key is ignored with a warning.

| Key | Type | Rule |
|---|---|---|
| `v` | integer | schema version, default 1. Not a positive integer: error. Above 1: read as v1 with a warning. |
| `id` | string | required; 1-64 of `A-Z a-z 0-9 . _ -`. The message id: posting the same id again replaces the card. |
| `title` | string | required, not blank; trimmed, cut to 120 chars with `…` (warning). |
| `state` | string | `open` (default), `pending`, `done`, `error`. Anything else: error. |
| `thread` | string | optional conversation id, id charset. Invalid: dropped with a warning. Stored with the card; nothing groups by it yet (reserved for the chat window). |
| `reply_to` | string | optional id of the message this card answers (see [reply_to](#reply_to-and-pending-placeholders)). Invalid: dropped with a warning. |
| `blocks` | array | body, top to bottom, at most 24. Not an array: ignored with a warning. |
| `actions` | array | buttons, left to right (wrapping, right-aligned), at most 6. Not an array: ignored with a warning. |

### Blocks

Each block is an object with a `type`.

| `type` | Keys | Shows |
|---|---|---|
| `text` | `md` (string, ≤ 4000 chars) | markdown-lite: `#`-`###` headings, `-`/`*`/`1.` bullets, fenced code, `**bold**`, `*italic*`, `` `code` ``, `[text](url)`. Tall text is cut at 240 pt. |
| `kv` | `items`: `[{"key":…,"value":…}]` (≤ 20) | key/value rows; number and boolean values become strings |
| `list` | `items`: `["…"]` (≤ 20), `ordered` (bool) | `•` rows, or `1.` rows when ordered; numbers and booleans become strings |
| `progress` | `value` (0-1, optional), `label` | a bar with `label  40%`; without `value` it is indeterminate (`label…`). Out of range is clamped (warning). |
| `choice` | `id`, `label`, `options` (≤ 12), `multi` (bool), `selected` | an input. An option is a string (its own id and label) or `{"id":…,"label":…}`. `selected` is an option id or a list of them. Checkboxes when `multi`, radio buttons up to 4 options, else a pop-up. |
| `field` | `id`, `label`, `placeholder`, `value` (≤ 2000), `multiline` (bool) | a text input, at most 4 per card |

Input ids (`choice` and `field`) use the id charset and must be unique on the card. Their values go back to KOTA with a press, keyed by id.

### Hard errors and degrade rules

A hard error is the verb's error (`invalid card: <reason>`, exit 1); nothing shows and nothing is stored:

- over 16384 bytes of JSON; not JSON; not an object;
- `id` missing or invalid; `title` missing, not a string or blank;
- `v` not a positive integer; `state` not one of the four.

Everything else degrades and adds a warning:

- An unknown `type`, a block that is not an object, or a block with bad keys (a missing `md`, a non-list `items`, an option without an id, a duplicate input id, …) becomes a text block `[<type>] <its md or text, else a JSON preview of ≤ 300 chars>`.
- Over-cap text (`title`, `md`, field `value`, action `label`) is cut with `…`. A `kv` or `list` over 20 items keeps 19 and ends in `…N more`. Past 24 blocks, the first 23 stay and the last is `…N more`. Extra options are dropped; fields past the 4th are dropped.
- `selected` ids that are not options are dropped; a single choice keeps only the first.
- An action that is not an object, or has a bad `id`, no `label`, or a duplicate id, is dropped. Past 6 actions the rest are dropped.
- An action with an unknown, malformed or refused `do` stays on the card, **disabled**, its reason as the tooltip. An unknown `style` is `default`; a non-boolean `reply` is false.

`flick message card get <id>` returns the normalized JSON Flick stored (what it understood).

## Actions

```text
{"id": "go", "label": "Ship", "style": "primary", "do": {…}, "reply": false}
```

- `id`: id charset, unique on the card; it comes back as `--action-id`. `label`: required, ≤ 40 chars.
- `style`: `default`, `primary` (⌘↵ presses the first enabled primary), `destructive`.
- No `do`: the press goes to KOTA. `do` without `reply`: a local action only. `do` with `"reply": true`: the local action first, then, if it worked, the press goes to KOTA (if it failed, the card shows the error and nothing is sent).

### The `do` vocabulary

`do` is an object with exactly one key (`dismiss` may also be the bare string `"dismiss"`). Field values are never templated into a `do`.

| `do` | Runs on the user's Mac | Card line |
|---|---|---|
| `{"open_url":"https://…"}` | opens an http or https link (≤ 2048 chars, no spaces or control chars) | Opened the link |
| `{"open_app":"Safari"}` | opens an app by name or bundle id (≤ 256 chars). A path (`/`, `~`) is refused. | Opened Safari |
| `{"copy":"text"}` | puts the literal text on the clipboard | Copied |
| `{"script":{"name":"deploy","query":"prod"}}` | runs the user's `[[script.commands]]` entry `name` (`query` optional) as `flick script run <name> [query]`, 30 s limit | the reply's last line, e.g. `ran deploy: …` |
| `{"flick":["task","start","x"]}` | one flick request (module, verb, args), 30 s limit | the reply's last line |
| `{"shell":"make deploy"}` | `/bin/sh -c` in the home folder after an in-card confirm, 60 s limit (the process group is killed) | the last output line |
| `{"dismiss":true}` | closes the card | (closes) |

While `script`, `flick` or `shell` runs, the card shows a spinner and `Running script deploy…` / `Running flick task start x…` / `Running command…`. A failure is `<label> failed (exit n): <last stderr line>`, `<label> took over N s; stopped`, or `<label> was killed by a signal`. Output is never stored.

### Security model

- Every card posted over the network (`--host`) is **remote**, and Flick stores that origin with it. KOTA's cards are remote.
- `flick` actions: the first word must be a module (not `-…`, not `events`, not `snapshot`, `import-raycast`, `help` or `config`, which the CLI runs in-process), and no word may be `--host`, `--host=…`, `--json`, `--remote` or `--stdin`. On a remote card the words must also pass the network policy, so a card can never launder a verb peers may not send: `reload`, `flick rebuild|cancel`, `keys fire`, `app uninstall`, `quicklink add|remove`, `capture`, `feedback resolve`, `task rm`, `script run`, `message card press|focus`, `dictation`, `kota ask`, `llm`, `sys restart|tail|window`, and every `remote` verb but `status` show **disabled** (`flick: script run: not allowed over the network`).
- The child process has no `$FLICK_HOST` (it talks to this Mac's Flick) and has `$FLICK_REMOTE=1` for a remote card, so module remote guards apply too.
- `script` is how a remote card runs one of the user's own scripts: it names an entry the user configured, and runs only when the user presses it.
- `shell` is **always** confirmed, whatever the origin: the first press replaces the buttons with the exact command and Cancel / Run; only Run executes it. A command over 2000 chars is refused (not cut), since the confirm must show exactly what runs.
- `open_url` takes http and https only; `open_app` takes names and bundle ids only.
- Every `do` is checked again at press time with the card's stored origin. Nothing runs without a press: a post never runs anything.

## States and updates

| `state` | Shows | Actions |
|---|---|---|
| `open` | speech-bubble symbol | enabled |
| `pending` | hourglass, spinner, "Sent to KOTA…" | disabled; presses are ignored |
| `done` | check mark, "Done" | disabled |
| `error` | warning symbol, "KOTA reported an error" | enabled (retry) |

- **Update a card by re-posting the same `id`** with the full card (a post replaces, it never merges). It redraws in place, keeps its slot, plays no sound, and clears Flick's own press state (pending, error line, confirm, running). What the user typed or picked survives the redraw unless the update changed that input's initial `value`/`selected`.
- An `open` card with an enabled action stays until acted on or closed (`card_timeout_secs`, default 0 = forever; Esc from another app does not close it). A `pending` card stays until updated. Other cards hide after `timeout_secs` (default 20 s) unless the pointer rests on them.
- A card you post as `pending` stays up with no timeout (and no watchdog) until you re-post it, so the final `done` update of a long job shows in its place; the user can still close it.
- A card the user dismissed (x, Esc, `card dismiss`) comes back only when re-posted as `open` or `error`; a `done` or `pending` re-post of it goes to history silently. A card that only timed out is not dismissed: any re-post of it shows again.
- `pending` posts make no sound and no notification.
- Error details belong in a `text` block; the `error` state's own line is generic.

## reply_to and pending placeholders

`k <text>` on the laptop posts a `--pending` placeholder message and hands its id to KOTA. A card with `"reply_to": "<that id>"` takes its place: the placeholder leaves history and its corner card closes, and `replaced` is true. A `reply_to` naming a message that is not pending keeps that message and records it as the card's context. To answer a `k` question with a card, post the card with `reply_to` set; later updates of the card re-post its own `id` (keeping `reply_to` is harmless).

## Presses reach KOTA

The user's Mac config holds the command (unset, a press shows "Set [message] action_command to send presses to KOTA"):

```toml
[message]
action_command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "jaymin@mbp-server", ".dotfiles/home/.local/bin/kota-ask"]
pending_timeout_secs = 120
```

A press of a reply action runs, on a background thread:

```text
<action_command...> --action --card <card id> --action-id <action id>
stdin: {"env":"prod","note":"ship it","tags":["a","b"]}   then EOF
```

- Stdin is the values JSON: one key per input, a string for a `field`, an option id or `null` for a single `choice`, a list of option ids for a `multi` choice; `{}` without inputs. At most 4000 chars (a larger press is not sent); key order is not meaningful. Stdout is ignored.
- The card shows "Sent to KOTA…" with its actions off.
- **Exit 0**: accepted. The card stays pending until KOTA re-posts the same `id` (any state). After `pending_timeout_secs` (default 120 s; 0 waits forever) without one, the card shows "No update from KOTA" and its actions come back. Re-posting before the command exits is fine.
- **Exit 2**: rejected. The last non-empty stderr line (≤ 200 chars) shows as `KOTA rejected: <line>`; actions come back.
- **Anything else** (another code, a signal, a spawn failure): `Sending to KOTA failed (exit n): <last stderr line>`; actions come back.
- **20 s budget** from spawn to exit, then the command is killed: "No answer from KOTA in 20 s". Queue the work and exit; do the work after.
- A press on a `pending` or `done` card, or on one whose press still runs, is ignored (`card press` answers `Card <id> is done: its actions are off`), so a press is never sent twice and a finished card never runs its actions again. A card dismissed while its press runs drops the result.

The KOTA side (`kota-ask --action`) should: read the values, queue a tick that names the card and action, exit 0 at once, then re-post the card (`done` with the outcome, `open` with the next question, or `error` with what failed).

## Commands

```sh
flick message card post <json>|--stdin   # id + warning lines; --json {"id","replaced","warnings"}
flick message card get <id>              # the stored, normalized card JSON
flick message card ls [--limit n]        # id<TAB>time<TAB>state<TAB>title, newest first (default 20); --json [{id,ts,remote,card}]
flick message card show <id>             # show it again (also undoes a dismissal)
flick message card dismiss <id>|--all    # remove it (every card) from the screen; history stays
flick message card spec                  # this page
flick message card press <id> <action> [values-json]   # press as a click does (this Mac only)
flick message card focus                 # move the keyboard into the newest card (this Mac only)
```

Peers may send every card verb except `press` and `focus` (refused: `message card press: not allowed over the network`, exit 1). An unknown id is `No card <id>` (exit 1). `card press` defaults the values to the card's initial inputs; on a `shell` action the first press shows the confirm and a second is Run; `press <id> :cancel` is Cancel.

## Keyboard (user side)

A card takes the keyboard only through the user's `card_hotkey`, `card focus` or a click in a field; the front app stays active.

| Key | Does |
|---|---|
| Tab / Shift-Tab | next / previous field, choice or button |
| ⌘1 … ⌘6 | press action 1 … 6 |
| ⌘↵ | press the first enabled `primary` action |
| ⌥↑ / ⌥↓ | move to the card above / below |
| Esc | give the keyboard back; the card stays |

Any press gives the keyboard back. So: put the likely answer first and mark it `primary`.

## Examples

Every `json` block on this page is a complete card; a unit test parses each one so the page cannot drift from the parser.

Approval with buttons. "Approve" and "Deny" go to KOTA; "Open PR" only opens the link.

```json
{"v": 1, "id": "pr-128", "title": "Merge PR #128?", "reply_to": "k-7f3",
 "blocks": [
   {"type": "text", "md": "**flick#128** keyboard reach for cards. CI green, 9/9 gates."},
   {"type": "kv", "items": [{"key": "Branch", "value": "feat/cards-keyboard"}, {"key": "Diff", "value": "+412 -38"}]}
 ],
 "actions": [
   {"id": "approve", "label": "Approve", "style": "primary"},
   {"id": "deny", "label": "Deny", "style": "destructive"},
   {"id": "open", "label": "Open PR", "do": {"open_url": "https://github.com/jayminwest/flick/pull/128"}}
 ]}
```

A press of Approve runs `kota-ask --action --card pr-128 --action-id approve` with `{}` on stdin. KOTA then re-posts:

```json
{"id": "pr-128", "title": "Merged PR #128", "state": "done",
 "blocks": [{"type": "text", "md": "Merged into main as `77aaab6`."}]}
```

Choice and field form. Values come back as `{"slot":"tue-10","note":"…","with":["sam"]}`.

```json
{"id": "dentist", "title": "Book the dentist?", "thread": "errands",
 "blocks": [
   {"type": "choice", "id": "slot", "label": "Slot", "selected": "tue-10",
    "options": [{"id": "tue-10", "label": "Tue 10:00"}, {"id": "wed-14", "label": "Wed 14:00"}, {"id": "fri-9", "label": "Fri 9:00"}]},
   {"type": "choice", "id": "with", "label": "Remind", "multi": true, "options": ["sam", "alex"]},
   {"type": "field", "id": "note", "label": "Note", "placeholder": "Anything to tell them"}
 ],
 "actions": [
   {"id": "book", "label": "Book", "style": "primary"},
   {"id": "skip", "label": "Not now", "do": "dismiss", "reply": true}
 ]}
```

Progress. Re-post the same id as it moves; finish with `open` or `error` if the user must see the end.

```json
{"id": "upload-3", "title": "Uploading episode 12", "state": "pending",
 "blocks": [{"type": "progress", "value": 0.4, "label": "Uploading"}, {"type": "list", "items": ["Render done", "Upload 40%"]}]}
```

```json
{"id": "upload-3", "title": "Episode 12 is up", "state": "open",
 "blocks": [{"type": "text", "md": "Published, scheduled for Fri 9:00."}],
 "actions": [
   {"id": "view", "label": "Open", "style": "primary", "do": {"open_url": "https://studio.youtube.com/"}},
   {"id": "close", "label": "Close", "do": {"dismiss": true}}
 ]}
```

Local actions. Each runs on the laptop, only when pressed; `shell` asks first.

```json
{"id": "standup", "title": "Standup in 5 min", "state": "open",
 "blocks": [{"type": "text", "md": "Notes copied from yesterday's log."}],
 "actions": [
   {"id": "zoom", "label": "Open Zoom", "style": "primary", "do": {"open_app": "us.zoom.xos"}},
   {"id": "copy", "label": "Copy notes", "do": {"copy": "- shipped cards\n- next: chat window"}},
   {"id": "timer", "label": "Start timer", "do": {"flick": ["task", "start", "standup"]}},
   {"id": "prep", "label": "Run prep", "do": {"script": {"name": "standup-prep"}}},
   {"id": "pull", "label": "git pull", "do": {"shell": "cd ~/Projects/flick && git pull --ff-only"}}
 ]}
```

An error with a retry.

```json
{"id": "pr-128", "title": "Merge failed", "state": "error",
 "blocks": [{"type": "text", "md": "Branch protection: 1 review required."}],
 "actions": [{"id": "approve", "label": "Retry", "style": "primary"}]}
```

## Manual end-to-end check

Run by Jaymin with the laptop awake, `[remote]` on there with mbp-server in `peers`, and `action_command` set as above.

1. On mbp-server: `flick --host jaymins-macbook-pro message card spec | head -1` prints this page's title (exit 0).
2. On mbp-server: post the approval example with `printf %s "$json" | flick --host jaymins-macbook-pro --json message card post --stdin`. The reply is `{"ok":{"id":"pr-128","replaced":false,"warnings":[]}}`; the card shows top right on the laptop without taking focus; `flick --host jaymins-macbook-pro message card ls` lists it as `open`.
3. Post `{"id":"x"}` the same way: exit 1, `invalid card: card needs a non-empty title string`, nothing shows. Stop Flick or use a wrong host: exit 3.
4. On the laptop, click **Open PR**: the browser opens, the card says "Opened the link" and stays.
5. Press **Approve** (or `card_hotkey` then ⌘↵): the card shows "Sent to KOTA…". On mbp-server, `kota-ask` got `--action --card pr-128 --action-id approve` and `{}` on stdin, and KOTA got a tick.
6. KOTA re-posts the `done` card: it redraws in place, "Done", no sound, and hides after 20 s.
7. Post the form example; pick Wed, tick sam, type a note, press **Book**: stdin holds `{"note":"…","slot":"wed-14","with":["sam"]}`.
8. Press a reply action and have KOTA not answer: after 120 s the card shows "No update from KOTA" and the buttons come back. Make `kota-ask` print a reason to stderr and exit 2: the card shows `KOTA rejected: <reason>`.
9. Post a card with `{"do":{"flick":["script","run","x"]}}` from mbp-server: its button is disabled with `flick: script run: not allowed over the network`. A `shell` action shows the exact command and runs only on Run.
10. `flick --host jaymins-macbook-pro message card press pr-128 approve` is refused (exit 1); `message card dismiss --all` clears the corner.
