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
```

All keys are optional; the values above are the defaults except `name` (default `Messages`) and `hotkey`.

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

## Launcher

- **Messages** (or your `name`) in root search shows the latest message as its subtitle; Enter opens the list.
- In the list, newest first: Enter copies the message's text. ⌘K: **Show in Panel**, **Open Link** (with a link) and **Copy Message**.

## Commands

```bash
flick message post [--title t] [--url https://…] [--reply-to id] [--id id] [--pending] [--] <body...>
flick message ls [--limit n]     # <id>\t<time>\t<title: >body, newest first (--json: the records)
flick message show [id]          # show a message (default the latest) in the card again
flick message hide               # dismiss every card
```

`post` prints the message id (generated unless `--id`; ids are 1-64 of `A-Z a-z 0-9 . _ -`); with `--json` it answers `{"id":"…","replaced":true|false}`. The body is the words after the flags joined by spaces, at most 16 KiB; put `--` before a body that starts with `--`.

**Network.** Peers in `[remote] peers` may send every `message` verb: posting is the point of the module. A post can only show text and offer an http(s) link that you click.

## Manual tests

1. `flick message post "hello"`: a card shows top right without taking focus from the front app (keep typing in it), plays a sound and hides after 20 s.
2. `flick message post --url https://example.com "click me"`, click the card: the browser opens example.com and the card hides.
3. Post again and press Esc in another app: the card hides. Click a card's x: only that card closes. Post with `timeout_secs = 0` set and reload: it stays until dismissed.
4. `flick message post --pending --id t1 "question?"` then `flick message post --reply-to t1 "answer"`: the hourglass card is replaced by the answer with `Re: question?`; `flick message ls` lists only the answer.
5. Set `position = "bottom-left"`, `style = "both"`, reload, post: the card is bottom left and a notification shows.
6. Open **Messages** in the launcher, press Enter on a row: the footer says Copied message and the text is on the clipboard. ⌘K **Show in Panel** shows it again.
7. Post five messages with different `--id`s: four cards stack from the corner, newest nearest it, with a `+1 more` pill; close one and the hidden card appears. Post one of the ids again: that card redraws in place, no sound.
8. From a peer: `flick --host <this Mac> message post --title KOTA "from the server"` shows the card.
