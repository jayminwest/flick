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
max_history = 50         # messages kept outside chat threads
chat_history = 200       # messages kept per chat thread
chat_threads = 20        # chat threads kept (the one with the oldest last message goes first)
sound = true             # a short sound when a message (not a pending one) arrives
hotkey = "cmd+ctrl+alt+shift+KeyM"  # opens the list; unbound by default
card_hotkey = "cmd+ctrl+alt+shift+KeyK"  # moves the keyboard into the newest card (again: back); unbound by default
action_command = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "jaymin@mbp-server", ".dotfiles/home/.local/bin/kota-ask"]
pending_timeout_secs = 120  # a sent press waiting this long for KOTA shows "No update from KOTA"; 0 waits
card_timeout_secs = 0       # how long an open card with actions stays; 0 until acted on or closed
chat_hotkey = "cmd+ctrl+alt+shift+KeyJ"  # shows or hides the KOTA chat window; unbound by default
kota_host = "jaymin@mbp-server"          # where a chat question goes (ssh target)
kota_ask = ".dotfiles/home/.local/bin/kota-ask"  # kota-ask on that host, relative to its home
```

All keys are optional; the values above are the defaults except `name` (default `Messages`), `hotkey`, `card_hotkey`, `chat_hotkey` (unbound: no chat) and `action_command` (default empty: card presses that reply show an error naming the key).

## The card

- Each message is its own card, keyed by its id: posting (or `show`ing) an id that already shows redraws that card in place, without a sound; a new id stacks a new card nearest the `position` corner, older ones moving away from it. At most `max_cards` show; the rest wait in a `+N more` pill and come back as cards close. The stack sits on the screen under the pointer when its first card appeared, and stays there until it empties; it is above other windows and on every Space.
- It never activates Flick and takes the keyboard only when you click one of its text fields or use `card_hotkey` (see [Keyboard](#keyboard)), so typing stays in the app you are using.
- **Click** opens the post's `--url` (http and https only) and dismisses the card; without a link, a click just dismisses it. The **x** in the top right corner just closes it. **Esc** dismisses every card from any app (except while a card has the keyboard: then Esc only gives it back); that needs Accessibility for Flick (without it, use a click or the timeout). Each card hides after its own `timeout_secs`, later if the pointer rests on it.
- The body is plain text with light markdown cleanup: `**`, `__` and backticks dropped, `#` headings and `-`/`*` bullets as plain lines and `•`, `[text](url)` as `text (url)`. The card shows up to 18 lines; the list has the full text.
- `style = "notification"` or `"both"` also posts a macOS notification (needs notification permission; clicking it does nothing yet).

## Pending posts and replies

A post with `--pending` is a placeholder: an hourglass, a dimmed body, "Waiting for a reply…", no sound and no notification. A later post with `--reply-to <its id>` removes it from history and its card, and shows the reply with a `Re: <placeholder body>` line.

![A pending card](screenshots/message-pending.png)

`k <text>` (Ask KOTA) uses this: the script posts `--pending` locally, sends the text and the id to KOTA, and KOTA answers with `flick --host <this Mac> message post --reply-to <id> ...`. A `--reply-to` naming a message that is not pending keeps that message and quotes it.

## Cards

A card is a message with structure, posted as JSON. The schema, the action vocabulary, the press round trip and examples are in [cards.md](cards.md), the spec KOTA reads with `flick --host <mac> message card spec`; this section is the user side. It shows as a card with its title, blocks (text, key/value rows, lists, progress, choices, fields) and action buttons. Text blocks render markdown-lite styled: bold, italic, `code`, fenced code, headings, bullets and links. It is stored in the same history as its plain text (the `message:<id>` row and `ls` show that text).

```bash
printf %s "$json" | flick --host my-laptop message card post --stdin   # or: message card post '<json>'
```

- Posting the same `id` again replaces the card in place. `reply_to` (a pending message id) replaces that placeholder, as `post --reply-to` does.
- A card in state `open` with an enabled action stays for `card_timeout_secs` (default: until closed; Esc leaves it); a `pending` card (the hourglass, without sound or notification) stays until it is updated or closed; other cards hide after `timeout_secs`.
- A card you dismissed (`card dismiss`, its x or Esc) comes back only when re-posted as `open` or `error`; a `done` or `pending` update of it goes to history silently. A card that only timed out shows any update.

### Presses

- An action without `do` (or with `"reply": true`) replies to KOTA: Flick runs `action_command` with `--action --card <id> --action-id <action id>` appended and the card's field and choice values as a JSON object on stdin (`{"<input id>": "text" | "option id" | null | ["option id", …]}`), on a background thread, killed after 20 s. The card shows "Sent to KOTA…" with its actions off while it runs.
  - Exit 0: the card stays pending until KOTA posts the same id again (which redraws it and clears the pending state). If no update comes within `pending_timeout_secs`, the card shows "No update from KOTA" and its actions come back.
  - Exit 2: the card shows "KOTA rejected: <last stderr line>"; other exits, a timeout or a command that cannot start show an error line. Either way the actions come back, so you can retry.
  - Presses on a pending or `done` card are ignored, so a press is never sent twice and a finished card never runs an action again; `open` and `error` cards take presses. Without `action_command` a press shows "Set [message] action_command to send presses to KOTA".
- `"do": "dismiss"` closes the card (with `"reply": true`, after the press reaches KOTA). Other local actions run on this Mac when pressed, checked again at press time against where the card came from (a card from a peer may not run a `flick` verb peers may not send):
  - `open_url`, `open_app` (a name or bundle id) and `copy` run at once; the card shows a short line ("Opened the link", "Opened Safari", "Copied").
  - `script` (`flick script run <name> [query]`) and `flick` (any request, e.g. `["task","start","x"]`) run Flick's own binary as a command line client in the background, killed after 30 s; the card shows "Running …", then the last line of the reply, or an error line. For a peer's card the request is marked `--remote`.
  - `shell` never runs on the first press: the card shows the exact command with Cancel and Run. Run executes it with `/bin/sh -c` in your home folder, killed (with what it started) after 60 s; the card shows the last output line or "command failed (exit n): <last stderr line>". Output is never stored.
  - With `"reply": true` the local part runs first; if it works, the press is sent to KOTA as above. If it fails, the card shows the error and nothing is sent.
- Pending, error and confirm states live in memory only.
- Any press (a click, Space on a focused button, ⌘1..⌘6, ⌘↵) gives the keyboard back to the app you were in, also after typing in one of the card's fields.
- Invalid JSON, a card over 16 KiB, or a missing/invalid `id`, `title`, `v` or `state` is an error reply `invalid card: <reason>` (exit 1); nothing is shown or stored. Anything else is shown as well as it can be, with a warning.
- Cards posted over the network are marked remote: a `flick` action naming a verb peers may not send is shown disabled.

### Keyboard

`card_hotkey` (or `flick message card focus`) moves the keyboard into the newest card without activating Flick: the app you are in stays active (its menu bar stays), only the typing goes to the card, its first field or button focused. Clicking a field in a card does the same for that card.

| Key | In a card that has the keyboard |
| --- | --- |
| Tab / Shift-Tab | next / previous field, choice or button (wraps) |
| Space | press the focused button, toggle the focused choice |
| ⌘1 … ⌘6 | press action 1 … 6 (a disabled one does nothing) |
| ⌘↵ | press the primary action (the first enabled `"style": "primary"`) |
| ⌥↑ / ⌥↓ | move the keyboard to the card above / below |
| Esc | give the keyboard back; the card stays (Esc never dismisses a card that has the keyboard, even one Esc would dismiss from another app) |
| ⌘X ⌘C ⌘V ⌘A ⌘Z | edit the focused field |

`card_hotkey` pressed again also gives the keyboard back, as does any press and closing the card. In a shell confirm step ⌘n and ⌘↵ do nothing: Tab to Run and press Space, or click it.

## Launcher

- **Messages** (or your `name`) in root search shows the latest message as its subtitle; Enter opens the list.
- In the list, newest first: Enter copies the message's text. ⌘K: **Show in Panel**, **Open Link** (with a link) and **Copy Message**.
- ⌘K on the root item: **Chat Threads**, the chat threads newest first; Enter opens the chat window on one.

## Chat

`chat_hotkey` shows a floating chat window with KOTA on the screen under the pointer, the caret in its input; the app you were in stays active, and Esc (or ⌘W, or the hotkey again) hides the window and gives that app the keyboard back. Return sends (⇧Return adds a line); your question shows at once with "Thinking…" under it, and goes to KOTA as `printf %s <question> | ssh <kota_host> <kota_ask> --id <req> --thread <t>` (20 s). If that fails the question is marked "Not sent · ⌘R retries" and the reason shows under the transcript; ⌘R sends it again. KOTA answers in the thread with `message post --thread <t> --reply-to <req> --id <x> [--partial]`; streaming re-posts update one bubble. While the window shows a thread, posts to it show no corner card and play no sound. ⌘N starts a thread, ⌘[ and ⌘] move to the older and newer one. Full chat docs: flick-3c60.

## Commands

```bash
flick message post [--title t] [--url https://…] [--reply-to id] [--id id] [--thread t] [--pending|--partial] [--] <body...>
flick message ls [--limit n]     # <id>\t<time>\t<title: >body[ (pending|partial|failed)], newest first (--json: the records)
flick message threads [--limit n]  # <thread>\t<time of last>\t<n> messages\t<first body>, newest first (--json: [{id,messages,first_ts,last_ts,first_body}])
flick message thread <t> [--limit n]  # the thread's newest n messages, oldest first, as ls prints them (--json: the records)
flick message chat [--thread t] [--snapshot <png>]  # show the chat window (on thread t); --snapshot draws it into a PNG (this Mac only)
flick message ask [--thread t] <text...>  # ask KOTA in the chat (default: the window's thread); answers once ssh is done (this Mac only)
flick message show [id]          # show a message (default the latest) in the card again
flick message hide               # dismiss every card
flick message card post <json>|--stdin   # the card id, then one "warning: …" line per warning
flick message card get <id>      # the stored (normalized) card JSON
flick message card ls [--limit n]  # <id>\t<time>\t<state>\t<title>, newest first (--json: [{id,ts,remote,card}])
flick message card show <id>     # show the card again
flick message card dismiss <id>|--all  # remove the card (every card)
flick message card spec          # print the card spec (docs/cards.md, built in)
flick message card press <id> <action> [values-json]  # press a button as a click does (this Mac only)
flick message card focus         # move the keyboard into the newest card, as card_hotkey does (this Mac only)
```

`card press` takes the same path as a click: values default to the card's initial field and choice values, and it answers what happened (`Sent go to KOTA…`, `Running script deploy…`, `Copied`, `Closed c1`) or the refusal. On a `shell` action the first `card press` shows the confirm on the card (and prints the command); a second `card press` of the same action is Run, and `card press <id> :cancel` is Cancel.

`card post --json` answers `{"id":"…","replaced":true|false,"warnings":["…"]}`; `replaced` is true when a card with that id existed or `reply_to` took a pending message. The client exits 1 when Flick answered with an error (fix the card) and 3 when it got no reply (Flick unreachable).

`post` prints the message id (generated unless `--id`; ids are 1-64 of `A-Z a-z 0-9 . _ -`); with `--json` it answers `{"id":"…","replaced":true|false}`. The body is the words after the flags joined by spaces, at most 16 KiB; put `--` before a body that starts with `--`.

**Threads and streaming** (for the chat window, plan pl-75d3). `--thread <t>` (an id as above) files the post in conversation `t`; a card's `thread` does the same. `--partial` marks a reply that is still streaming: post the same `--id` again with the text so far, and once more without `--partial` when it is complete. A partial post redraws its card in place with no sound and no notification; the final one plays the sound and notifies as any post. `--partial` and `--pending` together are an error. Re-posting an id that is in a thread or partial keeps its original time and place (and its thread, if the re-post has no `--thread`); re-posting any other id makes it new, as before. Unthreaded history keeps `max_history` messages, each thread `chat_history`, and `chat_threads` threads; chat never evicts unthreaded history. `--json` messages carry `thread`, `role` (`"me"` for what you typed in the chat) and `state` (`"partial"` or `"failed"`) only when set. `thread <t>` for a thread with no messages is the error `No thread <t>` (exit 1).

**Network.** Peers in `[remote] peers` may send every `message` verb (including `card spec`, `threads` and `thread`) except `card press`, `card focus`, `chat` and `ask` (a peer must not open a window that takes this Mac's keyboard or make this Mac ssh): posting is the point of the module. A post can only show text and offer an http(s) link that you click.

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
11. Post a local card with actions `{"do":{"open_url":"https://example.com"}}`, `{"do":{"open_app":"Calculator"}}`, `{"do":{"copy":"hello"}}`, `{"do":{"flick":["task","ls"]}}` and `{"do":{"shell":"echo hi; sleep 1; echo bye"}}`. Each press works and leaves a short line; the flick one shows the request's last reply line; the shell one shows the command, runs only on Run, shows "Running command…" for a second and then "bye". Typing in the front app keeps working throughout. A shell `sleep 90` shows "command took over 60 s; stopped" and leaves no `sleep` process.
12. From a peer, post a card with `{"do":{"flick":["reload"]}}`: the button is disabled with "flick: reload: not allowed over the network"; `flick message card press <id> <action>` prints the same refusal.
13. Set `card_hotkey`, reload, post a card with a field, a choice and two actions (one `"style":"primary"`) from a terminal, then click into another app (e.g. TextEdit) and type. Press `card_hotkey`: the menu bar still shows TextEdit, the card's field has the cursor. Tab/Shift-Tab walk the field, the choice and the buttons; ⌘2 presses the second action; ⌘↵ the primary. Press `card_hotkey` again (or Esc): typing goes to TextEdit again and the card stays. With two cards, ⌥↓ (top corner) moves the keyboard to the older card.
14. Click into the card's field (TextEdit stays active in the menu bar), type, press Esc: the card stays (an action card, and also a card without actions) and typing goes back to TextEdit. Click the field again, type, then click a button: the press happens and typing goes back to TextEdit without another click.
