# Local model chat

The `llm` module chats with OpenAI-compatible model servers on your tailnet, such as mlx-serve or ollama on the Mac Pro. You type in a floating chat window and the reply streams in. Normal chats are kept in flick.db unless you turn history off. The private chat keeps nothing: it lives in memory, talks only to a server you mark `private = true`, and is wiped when its window closes.

Nothing runs until `[[llm.servers]]` lists a server. Without one there is no launcher item, the chat hotkey is not bound, and no request is made. (A set `private_hotkey` is still bound, only to say that there is no private server.)

## Set up

Add a server and, if you like, a hotkey to `config.toml`, then run **Reload Flick Config**:

```toml
[llm]
hotkey = "cmd+ctrl+alt+shift+KeyL" # shows or hides the chat window; unbound by default
default_server = ""                # "" for the first normal server
default_model = ""                 # "" for the first model the server lists
system_prompt = ""                 # sent first in every chat; "" for none
history = true                     # keep normal chats in flick.db
max_threads = 100                  # the most chats kept; saving one past it drops the oldest (1 to 10000)
max_tokens = 0                     # token cap per reply; 0 for the server's default
timeout_secs = 300                 # seconds one reply may take (1 to 3600)

[[llm.servers]]
name = "mlx"
url = "https://mac-pro.example.ts.net:11234" # with or without /v1

[[llm.servers]]
name = "ollama"
url = "http://100.64.0.2:11434"
```

Check a server from a shell with `flick llm ping mlx`. `flick llm models mlx` lists its models.

A server with `private = true` is reserved for the private chat. The normal chat never uses it, and it is left out of the model list.

## Chat

- Open the window with the `hotkey` or **Local Model Chat** in the launcher. The hotkey hides it again while it has the keyboard. Flick does not take focus from the app you were in; that app gets the keyboard back when the window hides.
- Type and press Return to send. ⇧Return or ⌥Return adds a line. The reply streams into the window. While a reasoning model thinks, its bubble says "Thinking…". The reasoning text itself is not shown or kept.
- ⌘. stops the reply and keeps what had arrived (marked "stopped"). ⌘N starts a new chat. ⌘W or Esc hides the window. One reply runs at a time; a prompt sent while one streams goes back into the input.
- A failed reply shows red with the server's reason. The header dot is orange while a reply or a model list is on its way and red after a failure.
- Each prompt sends the whole chat so far, after `system_prompt`.
- A kept chat whose server is no longer in the config (or is now `private`) refuses to send and says why. Pick a model to move it to another server.

## Picking a model

⌘K on **Local Model Chat**, then **Choose Model…**, lists every normal server's models. The lists are fetched again each time the view opens. Enter on a model makes the open chat use it from the next prompt on, and new chats too, until Flick restarts. A server that did not answer shows its error; Enter on that row asks it again.

Without a pick, a chat uses `default_server` and `default_model`. When those are empty, it uses the first normal server and the first model that server lists. The list is fetched when you send the first prompt.

## History

With `history = true` (the default), each prompt and each finished reply is written to flick.db, in tables `llm_threads` and `llm_messages`. A finished reply is one that is done, stopped or failed. Only the newest `max_threads` chats are kept; saving a chat past that drops the oldest along with its messages. ⌘K on **Local Model Chat**, then **Chat History**, lists the kept chats, newest first. Enter reopens one. On launch, the window shows the newest kept chat.

With `history = false`, nothing is written. The chat lives in memory until ⌘N or quit, and **Chat History** is empty. Chats kept earlier stay in flick.db and are not shown.

A reply still streaming when Flick quits is not kept.

## Privacy of the normal chat

- Requests go through `/usr/bin/curl`, one process per request. The prompt goes over curl's stdin, never in its arguments, so `ps` does not show it.
- Flick never logs prompts or replies. Error lines carry at most 200 characters of a server's error message.
- Normal chats are stored in plain text in flick.db when `history` is on. The server may keep its own logs; mlx-serve logs the start of every prompt by default.
- Every `flick llm` command is refused over the network: another Mac cannot make this one send requests.

## Private chat

The private chat is a separate window that keeps nothing. Open it with **Private Model Chat** in the launcher or with `private_hotkey`:

```toml
[llm]
private_hotkey = "cmd+ctrl+alt+shift+KeyP" # shows or hides the private chat; unbound by default

[[llm.servers]]
name = "vault"
url = "https://mac-pro.example.ts.net:11235"
private = true
```

- It talks only to a server with `private = true` (the first one listed). With none, the launcher item and the hotkey say "no private server" and nothing is sent; it never falls back to a normal server. `private = true` is your word that the server keeps no logs or caches; Flick cannot check it.
- The model is `default_model` if the private server lists it, else the first model it lists. Each prompt sent before a model is known fetches the list again, so a prompt sent after the server starts works. If the list fails, the prompt goes back into the input and the notice says why (see [Troubleshooting](local-llm/private-server.md#troubleshooting)). The first reply after a while may wait while the server loads the model ("Waiting for the model…").
- The window says **Private - nothing is saved** in a banner and has its own outline. It is left out of screenshots, screen recording and screen sharing. Spell check, autocorrect, text completion, predictions, Writing Tools and undo are off in its input, and its bubbles cannot be selected.
- Keys: Return sends, ⌘. stops the reply, ⌘N clears the chat, ⌘W or Esc hides and clears it. The hotkey hides (and clears) it while it has the keyboard.
- ⌘C copies the input's selection, or, with nothing selected, the last reply. Either copy is marked concealed and transient, so Flick's clip history and other clipboard managers skip it. Pasting it elsewhere is up to you.
- It is cleared when the window hides, when the screen locks, when the Mac sleeps, when the config reloads, and when Flick quits. Reopening shows an empty window.
- Nothing of it is written to flick.db, logs, notifications or feedback. No `flick` command reads it, and no event carries its text.

What Flick cannot clear: copies that macOS keeps inside the window's text views until they are replaced, curl's and the kernel's buffers, and memory pages that macOS swapped out (swap is encrypted). "Nothing is saved" means Flick writes nothing, not that the memory is forensically clean. The server is the other half: see [Private server](#private-server) and [Verify private mode](#verify-private-mode) below.

## Commands

```bash
flick llm ping [server]            # does it answer, how fast, how many models
flick llm ping private [server]    # the same for a private server: models list only, no prompt
flick llm models [server] [--json] # its models: id, state, context length, capabilities
```

## Manual tests

Run these in the app; unit tests cover the logic with a fake curl and a fake window.

1. With no `[[llm.servers]]`: no **Local Model Chat** item, and the hotkey does nothing.
2. Add a server and a hotkey, reload. Press the hotkey from another app: the window shows under the pointer with the caret in the input, and the other app still looks active.
3. Send "hello". The reply streams in word by word. The header shows the server and model, and the dot is orange, then grey.
4. Ask for something long and press ⌘. midway: the reply stops and is marked "stopped".
5. Press ⌘N, send another prompt, then quit and relaunch Flick. The hotkey shows the newest chat; **Chat History** lists both, and Enter on the older one opens it.
6. **Choose Model…** lists each server's models. Pick another model and send: the reply's header names the new model.
7. Point a server at a dead port and send. With `default_model` empty, the notice line gives the error and the prompt returns to the input. With a `default_model`, the reply shows red with the error.
8. Set `history = false`, reload, and chat. Check that `sqlite3 ~/Library/Application\ Support/Flick/flick.db 'select count(*) from llm_messages'` does not grow.
9. Press Esc and ⌘W: the window hides, and the app you were in gets the keyboard back.

Private chat:

10. With no `private = true` server: **Private Model Chat** says "no private server" in the launcher, the private hotkey shows the same, and no request reaches the normal server.
11. Add a private server and reload. The private hotkey shows a window titled "Private Chat" with the "Private - nothing is saved" banner and an outlined frame. Send a prompt: the reply streams in.
12. Take a screenshot (⇧⌘3, ⇧⌘5 window capture) and start a screen recording: the private window is missing from both.
13. Type a misspelled word in its input: no red underline, no autocorrect, no completion popup. Right-click: no context menu.
14. ⌘C with nothing selected, then check Flick's clip history (and any clipboard manager): the reply is not listed, but it pastes into another app.
15. Each of these leaves an empty window with a "Cleared when ..." notice: Esc then reopen, ⌘N, lock the screen (⌃⌘Q) and unlock, sleep and wake, **Reload Flick Config**. After quit and relaunch the window is empty too (with no notice).
16. After a private exchange, `sqlite3 ~/Library/Application\ Support/Flick/flick.db .dump | grep <a word you sent>` finds nothing.

## Private server

Private chat (flick-c325) only sends to a server with `private = true`. On the Mac Pro, that is a second mlx-serve on `127.0.0.1:11235`. It keeps no log, no prompt cache and no core dumps, and `tailscale serve` publishes it to the tailnet only, with no API key. [`local-llm/private-server.md`](local-llm/private-server.md) has the LaunchDaemon plist, install, checks and rollback. You install it by hand; Flick never changes the Mac Pro.

`private = true` is your claim about a server. Only the :11235 kit is checked by the steps below. A server such as ollama keeps its own logs.

## Verify private mode

[`local-llm/verify-private.sh`](local-llm/verify-private.sh) makes a random canary, waits while you send it through the private chat, then searches both Macs for it. Run it on the laptop:

```bash
docs/local-llm/verify-private.sh                 # laptop, then the Mac Pro over ssh (host mac-pro)
docs/local-llm/verify-private.sh --host <ssh-host>
docs/local-llm/verify-private.sh --reuse         # check again for the last canary (asked for, hidden)
```

Each location gets a line: PASS (no canary), FAIL (found, with the files), SKIP (location absent) or INCOMPLETE (some files unreadable). The exit code is 0 when all pass, 1 on any FAIL, and 2 when nothing failed but something was unreadable.

| Laptop | Mac Pro |
| --- | --- |
| `~/Library/Application Support/Flick` (flick.db with its -wal and -shm, clip history) | `~/.mlx-serve` |
| clip history: the `clips` table, queried | `/tmp` |
| Flick's saved window state | `/var/log`, rotated .gz/.bz2 included |
| `~/Library/Logs`, `~/Library/Caches` | `/Library/Logs`, incl. DiagnosticReports |
| `$TMPDIR`, `/tmp` | `~/Library/Logs` |
| the feedback file (`flick feedback path`) | |
| unified log: flick, curl, tailscale | unified log: mlx-serve, tailscaled |

The canary never goes into a file or a command line: grep and sqlite3 read it from a pipe, and ssh gets it on stdin. Your terminal shows it, so do not paste it anywhere else. Terminal's own saved state is not searched.

Over ssh, the Mac Pro scan runs as your user, so root-only files under `/var/log` come back INCOMPLETE. For a complete scan, copy the script to the Mac Pro and run it there with `sudo bash verify-private.sh --here mac-pro`; it asks for the canary. On the laptop, an INCOMPLETE `~/Library/Caches` usually means the terminal lacks Full Disk Access.

Run it after installing the private server, after each mlx-serve or Flick update that touches private chat, and after any change to logging on either Mac. Note the date and result on flick-6519.

### Manual checklist

1. The private server is installed ([private-server.md](local-llm/private-server.md)). On the Mac Pro, `sudo lsof -nP -iTCP:11235 -sTCP:LISTEN` shows only `127.0.0.1:11235`, and `~/.mlx-serve/logs/` has no `mlx-serve-11235.log`.
2. Flick's config has the `private = true` server, and **Reload Flick Config** has run.
3. Run `verify-private.sh` on the laptop. Send `Repeat this word back exactly: <canary>` in the private chat, wait for the reply, copy it with ⌘C, close the window, and press Return in the terminal.
4. Every line is PASS or SKIP, and the result is `RESULT overall: PASS`. On INCOMPLETE, run the sudo scan on the Mac Pro or grant Full Disk Access, then run `--reuse`.
5. While a private reply streams, `ps -axww | grep curl` on the laptop shows the URL but no prompt text.
6. Paste on the laptop right after step 3: the reply pastes, but Flick's **Clipboard History** does not list it.
7. Take a screenshot (⇧⌘3) with the private window open: the window is missing from the image.
8. Reopen the private chat after closing it, after locking the screen, and after **Reload Flick Config**: it is empty each time.
9. Remove the `private = true` server and reload. Private chat refuses to send and says no private server is set; it does not fall back to :11234.
