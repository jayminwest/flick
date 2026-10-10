# Command line

A running Flick listens on `~/Library/Application Support/Flick/flick.sock` (mode 0600; set `FLICK_SOCKET` to use another path). The `flick` binary is its client. `bundle.sh --install` and the in-app rebuild link it as `~/.local/bin/flick`; by hand: `ln -s ~/Applications/Flick.app/Contents/MacOS/Flick ~/.local/bin/flick`.

```bash
flick app list                 # <name>\t<path> per app
flick app open Safari
flick app quit Safari          # asks it to quit, like cmd+Q; force-quit forces it
flick app reveal Safari        # show the bundle in Finder
flick app uninstall Foo --dry-run  # <size>\t<path> for the bundle and its leftovers
flick app uninstall Foo --yes  # move exactly those to the Trash
flick clip list                # <id>\t<first line>, newest first
flick clip get 42              # one clip's full text
flick window left-half         # any action from `flick window list`
flick feedback add "Tab should complete paths"  # append to feedback.jsonl
flick message post --title KOTA "Done"  # show a message card (docs/message.md)
flick reload                   # reload config.toml
flick config example           # every config option, commented (no running Flick needed)
flick --json clip list         # the raw reply: {"ok":"..."} or {"error":"..."}
flick events | jq .            # app_activated, pasteboard_changed, wake, idle, ... as JSON lines
```

The protocol is one JSON array of strings per line, `["<module>","<verb>",args...]`, answered by one JSON line. Exit status: 0 for an ok reply, 1 for an error reply (Flick answered and refused: fix the request), 3 when no reply came (Flick not running, host unreachable, connection dropped: try another way), 2 for a usage mistake.

A request word that is exactly `--stdin` is replaced by everything the client reads from stdin, verbatim (no trimming), before the request is sent, locally or with `--host`. It keeps large or sensitive arguments off the command line: `printf %s "$json" | flick --host mac-studio message card post --stdin`. It may appear once, in any position; twice, or with stdin a terminal, is a usage error (exit 2). Empty stdin is an empty argument. More than 16 KiB (16384 bytes) or input that is not UTF-8 exits 1 without sending anything. Without the word, stdin is not read.

To ask the Flick on another Mac in your tailnet, put `--host <name[:port]>` first (MagicDNS name or Tailscale IP; port default 7419), or set `FLICK_HOST`; `--host` wins. That Mac must have [network access](remote.md) on with this Mac in its `peers`.

```bash
flick --host mac-studio task ls
FLICK_HOST=mac-studio:7419 flick --json herdr ls
flick --host mac-studio events # only with [remote] events = true there
```

Each module's commands are in its own page: [keys](keys.md), [activity](activity.md), [tasks](tasks.md), [herdr](herdr.md), [capture](capture.md), [feedback](feedback.md), [messages](message.md), [remote](remote.md), and [rebuild](install.md#rebuild-from-the-checkout).
