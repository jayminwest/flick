# Command line

A running Flick listens on `~/Library/Application Support/Flick/flick.sock` (mode 0600; set `FLICK_SOCKET` to use another path). The `flick` binary is its client:

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
flick reload                   # reload config.toml
flick --json clip list         # the raw reply: {"ok":"..."} or {"error":"..."}
flick events | jq .            # app_activated, pasteboard_changed, wake, idle, ... as JSON lines
```

The protocol is one JSON array of strings per line, `["<module>","<verb>",args...]`, answered by one JSON line. An error reply exits with status 1.

To ask the Flick on another Mac in your tailnet, put `--host <name[:port]>` first (MagicDNS name or Tailscale IP; port default 7419), or set `FLICK_HOST`; `--host` wins. That Mac must have [network access](remote.md) on with this Mac in its `peers`.

```bash
flick --host mac-studio task ls
FLICK_HOST=mac-studio:7419 flick --json herdr ls
flick --host mac-studio events # only with [remote] events = true there
```

Each module's commands are in its own page: [keys](keys.md), [activity](activity.md), [tasks](tasks.md), [herdr](herdr.md), [capture](capture.md), [feedback](feedback.md), [remote](remote.md), and [rebuild](install.md#rebuild-from-the-checkout).
