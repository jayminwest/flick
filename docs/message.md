# Messages

The `message` module shows short messages posted to this Mac, usually by an agent on another Mac in your tailnet: a card in a screen corner that does not take focus, a notification, or both. Every message is kept in a history list in the launcher.

![A reply card](screenshots/message-reply.png)

```bash
flick message post "Build finished"
flick --host my-laptop message post --title KOTA --url https://github.com/me/repo/pull/2 "PR is up"
```

## Settings

```toml
[message]
name = "KOTA"            # the launcher item, and the card header when a post has no --title
style = "panel"          # panel | notification | both | none (history only)
position = "top-right"   # top-right | top-left | bottom-right | bottom-left | top | bottom
width = 380              # points, 240 to 900
timeout_secs = 20        # 0 keeps the card until dismissed
max_cards = 4            # cards shown at once; older ones collapse into a "+N more" pill
max_history = 50         # messages kept
sound = true             # a short sound when a message (not a pending one) arrives
hotkey = "cmd+ctrl+alt+shift+KeyM"  # opens the list; unbound by default
action_command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "jaymin@mbp-server", ".dotfiles/home/.local/bin/kota-ask"]
pending_timeout_secs = 120  # a sent press waiting this long for KOTA shows "No update from KOTA"; 0 waits
card_timeout_secs = 0       # how long an open card with actions stays; 0 until acted on or closed
```

All keys are optional; the values above are the defaults except `name` (default `Messages`), `hotkey` and `action_command` (default empty: card presses that reply show an error naming the key).

## The card

- Each message is its own card, keyed by its id: posting (or `show`ing) an id that already shows redraws that card in place, without a sound; a new id stacks a new card nearest the `position` corner, older ones moving away from it. At most `max_cards` show; the rest wait in a `+N more` pill and come back as cards close. The stack sits on the screen under the pointer when its first card appeared, and stays there until it empties; it is above other windows and on every Space.
- It never becomes the key window and never activates Flick, so typing stays in the app you are using.
- **Click** opens the post's `--url` (http and https only) and dismisses the card; without a link, a click just dismisses it. The **x** in the top right corner just closes it. **Esc** dismisses every card from any app; that needs Accessibility for Flick (without it, use a click or the timeout). Each card hides after its own `timeout_secs`, later if the pointer rests on it.
- The body is plain text with light markdown cleanup: `**`, `__` and backticks dropped, `#` headings and `-`/`*` bullets as plain lines and `•`, `[text](url)` as `text (url)`. The card shows up to 18 lines; the list has the full text.
- `style = "notification"` or `"both"` also posts a macOS notification (needs notification permission; clicking it does nothing yet).

## Pending posts and replies

A post with `--pending` is a placeholder: an hourglass, a dimmed body, "Waiting for a reply…", no sound and no notification. A later post with `--reply-to <its id>` removes it from history and its card, and shows the reply with a `Re: <placeholder body>` line.

![A pending card](screenshots/message-pending.png)

`k <text>` (Ask KOTA) uses this: the script posts `--pending` locally, sends the text and the id to KOTA, and KOTA answers with `flick --host <this Mac> message post --reply-to <id> ...`. A `--reply-to` naming a message that is not pending keeps that message and quotes it.

## Cards

A card is a message with structure, posted as JSON (schema in `src/core/card.rs`; the full spec, `docs/cards.md`, is coming). It shows as a card with its title, blocks (text, key/value rows, lists, progress, choices, fields) and action buttons. It is stored in the same history as its plain text (the `message:<id>` row and `ls` show that text).

```bash
printf %s "$json" | flick --host my-laptop message card post --stdin   # or: message card post '<json>'
```

- Posting the same `id` again replaces the card in place. `reply_to` (a pending message id) replaces that placeholder, as `post --reply-to` does.
- A card in state `open` with an enabled action stays for `card_timeout_secs` (default: until closed; Esc leaves it); other cards hide after `timeout_secs`. A `pending` card shows the hourglass, without sound or notification.
- A card you dismissed (`card dismiss`, its x, Esc or its timeout) comes back only when re-posted as `open` or `error`; a `done` or `pending` update of it goes to history silently.

### Presses

- An action without `do` (or with `"reply": true`) replies to KOTA: Flick runs `action_command` with `--action --card <id> --action-id <action id>` appended and the card's field and choice values as a JSON object on stdin (`{"<input id>": "text" | "option id" | null | ["option id", …]}`), on a background thread, killed after 20 s. The card shows "Sent to KOTA…" with its actions off while it runs.
  - Exit 0: the card stays pending until KOTA posts the same id again (which redraws it and clears the pending state). If no update comes within `pending_timeout_secs`, the card shows "No update from KOTA" and its actions come back.
  - Exit 2: the card shows "KOTA rejected: <last stderr line>"; other exits, a timeout or a command that cannot start show an error line. Either way the actions come back, so you can retry.
  - Presses on a pending card are ignored, so a press is never sent twice. Without `action_command` a press shows "Set [message] action_command to send presses to KOTA".
- `"do": "dismiss"` closes the card (with `"reply": true`, after the press reaches KOTA). A `shell` action never runs on the first press: the card shows the exact command with Cancel and Run. Running it and the other local actions (`open_url`, `open_app`, `copy`, `script`, `flick`) come with a later step (flick-e244); for now they show an error line. A local action with `"reply": true` already sends its press.
- Pending, error and confirm states live in memory only.
- Invalid JSON, a card over 16 KiB, or a missing/invalid `id`, `title`, `v` or `state` is an error reply `invalid card: <reason>` (exit 1); nothing is shown or stored. Anything else is shown as well as it can be, with a warning.
- Cards posted over the network are marked remote: a `flick` action naming a verb peers may not send is shown disabled.

## Launcher

- **Messages** (or your `name`) in root search shows the latest message as its subtitle; Enter opens the list.
- In the list, newest first: Enter copies the message's text. ⌘K: **Show in Panel**, **Open Link** (with a link) and **Copy Message**.

## Commands

```bash
flick message post [--title t] [--url https://…] [--reply-to id] [--id id] [--pending] [--] <body...>
flick message ls [--limit n]     # <id>\t<time>\t<title: >body, newest first (--json: the records)
flick message show [id]          # show a message (default the latest) in the card again
flick message hide               # dismiss every card
flick message card post <json>|--stdin   # the card id, then one "warning: …" line per warning
flick message card get <id>      # the stored (normalized) card JSON
flick message card ls [--limit n]  # <id>\t<time>\t<state>\t<title>, newest first (--json: [{id,ts,remote,card}])
flick message card show <id>     # show the card again
flick message card dismiss <id>|--all  # remove the card (every card)
```

`card post --json` answers `{"id":"…","replaced":true|false,"warnings":["…"]}`; `replaced` is true when a card with that id existed or `reply_to` took a pending message. The client exits 1 when Flick answered with an error (fix the card) and 3 when it got no reply (Flick unreachable).

`post` prints the message id (generated unless `--id`; ids are 1-64 of `A-Z a-z 0-9 . _ -`); with `--json` it answers `{"id":"…","replaced":true|false}`. The body is the words after the flags joined by spaces, at most 16 KiB; put `--` before a body that starts with `--`.

**Network.** Peers in `[remote] peers` may send every `message` verb except `card press` and `card focus`: posting is the point of the module. A post can only show text and offer an http(s) link that you click.

## Manual tests

1. `flick message post "hello"`: a card shows top right without taking focus from the front app (keep typing in it), plays a sound and hides after 20 s.
2. `flick message post --url https://example.com "click me"`, click the card: the browser opens example.com and the card hides.
3. Post again and press Esc in another app: the card hides. Click a card's x: only that card closes. Post with `timeout_secs = 0` set and reload: it stays until dismissed.
4. `flick message post --pending --id t1 "question?"` then `flick message post --reply-to t1 "answer"`: the hourglass card is replaced by the answer with `Re: question?`; `flick message ls` lists only the answer.
5. Set `position = "bottom-left"`, `style = "both"`, reload, post: the card is bottom left and a notification shows.
6. Open **Messages** in the launcher, press Enter on a row: the footer says Copied message and the text is on the clipboard. ⌘K **Show in Panel** shows it again.
7. Post five messages with different `--id`s: four cards stack from the corner, newest nearest it, with a `+1 more` pill; close one and the hidden card appears. Post one of the ids again: that card redraws in place, no sound.
8. From a peer: `flick --host <this Mac> message post --title KOTA "from the server"` shows the card.
9. `printf %s '{"id":"c1","title":"Deploy?","blocks":[{"type":"text","md":"Ship **v2**"}],"actions":[{"id":"go","label":"Ship"}]}' | flick message card post --stdin`: a card "Deploy?" with "Ship v2" and "[Ship]" stays (Esc does not close it). Post it again with `"state":"done"`: it redraws in place and hides after `timeout_secs`. `flick message card post '{"id":"x"}'` prints `flick: invalid card: …` and exits 1.
10. Set `action_command = ["/bin/sh", "-c", "cat > /tmp/press.json; echo \"$@\" >> /tmp/press.json", "sh"]` and reload. Post a card with a field and a `Ship` action without `do`, type in the field and press Ship: the card shows "Sent to KOTA…", `/tmp/press.json` holds the values JSON and `--action --card <id> --action-id ship`. Re-post the card: it redraws with Ship enabled. With `pending_timeout_secs = 10`, press again and wait: "No update from KOTA" shows. With `exit 2` in the script (writing a line to stderr first), the card shows "KOTA rejected: <that line>".
