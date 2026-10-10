# Private mlx-serve on the Mac Pro

Flick's private chat only talks to servers marked `private = true`. This page sets one up: a second mlx-serve LaunchDaemon, `com.jaymin.mlx-serve-private`, on `127.0.0.1:11235`, published to the tailnet with `tailscale serve`. It is separate from the shared `com.jaymin.mlx-serve` on :11234. The shared instance logs the start of every prompt and keeps a prefix cache, and mailwatch and kota-memory depend on it, so it stays as it is.

Flick does not install this. You run the steps below by hand on the Mac Pro. Then [`verify-private.sh`](verify-private.sh) checks that nothing was written ([checklist](../local-llm.md#verify-private-mode)).

## What the plist does

[`com.jaymin.mlx-serve-private.plist`](com.jaymin.mlx-serve-private.plist):

| Setting | Why |
| --- | --- |
| `--host 127.0.0.1 --port 11235` | Loopback only. The tailnet reaches it through `tailscale serve`, never on the LAN. |
| `--log-file off --log-level error` | No request log. Without these, mlx-serve writes `~/.mlx-serve/logs/mlx-serve-11235.log` with the start of every prompt. |
| `--prefix-cache-entries 0 --tokenize-cache-entries 0` | No prompt prefixes or token ids kept between requests. The disk prefix cache is off by default, and the plist does not pass `--prefix-cache-disk`. |
| no `--lan-share`, no `--api-key-env` | Tailnet only, no API key. Jaymin decided this on 2026-10-10. Any device on the tailnet can reach :11235. |
| `--max-resident-models 1 --idle-evict-secs 900` | Holds one model, and only while in use. It frees the memory 15 minutes after the last request. The first private prompt after that waits while the model loads. |
| `StandardOutPath`/`StandardErrorPath` `/dev/null` | Anything mlx-serve prints is discarded, not written to a file. |
| `Core` limit 0 (soft and hard) | A crash writes no core dump of process memory. |
| `Umask` 077, `WorkingDirectory` `/var/empty` | A file it does write is readable only by its user, and nothing lands in the working directory. |

## Before you install: fill in the placeholders

The shared daemon's plist is in no repo, so two values in the kit are placeholders. Read them off the shared daemon:

```bash
sudo launchctl print system/com.jaymin.mlx-serve | grep -E 'program|arguments|username' -A12
```

1. Replace `__MLX_SERVE_BINARY__` with the `program` path.
2. Replace the `<string>__MODEL_ARGS__</string>` line with the shared daemon's model arguments, one `<string>` per argument (for example its model or model-dir flag and value, and `--mtp` if used). If it has none, delete the line. Leave out its port, log, cache and LAN-share flags; the kit sets its own.
3. If the shared daemon passes host and port with other flag names, use those names. `mlx-serve --help` lists them.
4. If its `username` is not `jaymin`, change `UserName` and `HOME` to match. The private instance then reads the same model cache.

Do steps 1 to 4 in a copy, not in the repo:

```bash
cp docs/local-llm/com.jaymin.mlx-serve-private.plist /tmp/mlx-private.plist
$EDITOR /tmp/mlx-private.plist
plutil -lint /tmp/mlx-private.plist
grep -c '<string>__' /tmp/mlx-private.plist   # must print 0
```

## Install

On the Mac Pro:

```bash
# 1. The plist, owned by root, not writable by others.
sudo cp /tmp/mlx-private.plist /Library/LaunchDaemons/com.jaymin.mlx-serve-private.plist
sudo chown root:wheel /Library/LaunchDaemons/com.jaymin.mlx-serve-private.plist
sudo chmod 644 /Library/LaunchDaemons/com.jaymin.mlx-serve-private.plist
rm /tmp/mlx-private.plist

# 2. Delete the stale log from the Sep 3 experiment on :11235.
rm -f ~/.mlx-serve/logs/mlx-serve-11235.log

# 3. Start it.
sudo launchctl bootstrap system /Library/LaunchDaemons/com.jaymin.mlx-serve-private.plist
sudo launchctl print system/com.jaymin.mlx-serve-private | grep -E 'state|pid|last exit'

# 4. Publish it to the tailnet (HTTPS, tailnet only; same as :11234).
tailscale serve --bg --https=11235 http://127.0.0.1:11235
tailscale serve status
```

If `tailscale serve` says access denied, run it with `sudo`.

## Check it

On the Mac Pro:

```bash
sudo lsof -nP -iTCP:11235 -sTCP:LISTEN     # only 127.0.0.1:11235, never *:11235
curl -s http://127.0.0.1:11235/v1/models   # lists the model
ls ~/.mlx-serve/logs/                      # no mlx-serve-11235.log
```

From the laptop:

```bash
curl -s https://<mac-pro>.<tailnet>.ts.net:11235/v1/models
```

Then add the server to Flick's `config.toml` on the laptop and run **Reload Flick Config**:

```toml
[[llm.servers]]
name = "private"
url = "https://mac-pro.example.ts.net:11235"
private = true
```

Run [`verify-private.sh`](verify-private.sh) next ([checklist](../local-llm.md#verify-private-mode)).

## Troubleshooting

- **`state = not running`, or `last exit code` keeps changing:** the daemon discards its own output, so start the same command in a shell to see the error. Use the plist's arguments, but `--log-level info` and no `--log-file off` are fine for a minute. Then delete any log it wrote: `rm -f ~/.mlx-serve/logs/mlx-serve-11235.log`.
- **The private chat says `error: 502 (the proxy reached no model server behind it ...)`:** `tailscale serve` is set up, but nothing listens on `127.0.0.1:11235`. The daemon is not installed or not running: run the checks in [Check it](#check-it).
- **It says `(7) Failed to connect ...`:** nothing answers on the tailnet port. Run `tailscale serve status` on the Mac Pro and look for `:11235`.
- **It fails on a relative path:** the working directory is `/var/empty`, which it cannot write to. Pass absolute paths in the model arguments.
- **After an mlx-serve upgrade:** run `verify-private.sh` again. A new version may log somewhere new.

## Roll back

```bash
tailscale serve --https=11235 off
sudo launchctl bootout system/com.jaymin.mlx-serve-private
sudo rm /Library/LaunchDaemons/com.jaymin.mlx-serve-private.plist
rm -f ~/.mlx-serve/logs/mlx-serve-11235.log
```

Then remove the `private = true` server from Flick's config. The shared `com.jaymin.mlx-serve` on :11234 is not touched by install or rollback.
