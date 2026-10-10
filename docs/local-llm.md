# Local model chat

The `llm` module chats with OpenAI-compatible model servers on your tailnet, such as mlx-serve or ollama on the Mac Pro. You type in a floating chat window and the reply streams in. Normal chats are kept in flick.db unless you turn history off. A private mode that stores nothing is planned (flick-c325) and is not in this build.

Nothing runs until `[[llm.servers]]` lists a server. Without one there is no launcher item, the hotkey is not bound, and no request is made.

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

## Commands

```bash
flick llm ping [server]            # does it answer, how fast, how many models
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
