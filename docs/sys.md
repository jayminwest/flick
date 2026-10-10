# System and fleet

The `sys` module reports this Mac's health and the services it checks. It also shows a fleet of Macs in one launcher view, here the laptop, mbp-server and the Mac Pro. **Fleet** in root search lists each machine with its load, memory, disk, battery and age, and under it each service with ok, warn or fail. ⌘K on a machine opens Screen Sharing or its dashboard. ⌘K on a service tails its log or restarts it after a confirm.

Nothing runs in the background by default. A check runs when you ask (`flick sys ...`), when the launcher opens, on wake, and every 15 s while the Fleet view shows. With `refresh_secs`, the fleet is also polled on a timer.

## This Mac

```bash
flick sys snapshot           # host, uptime, CPU load, memory, disks, battery, thermal, services summary
flick sys services           # one line per [[sys.service]]: ok|warn|fail|unknown, name, kind, reason, age
flick --json sys snapshot    # the JSON a fleet on another Mac reads (schema 1)
```

The facts come from one run of stock tools: `sysctl`, `df`, `pmset`. Flick uses no private APIs and needs no permission. Both verbs answer from a cache and start a refresh. The first call with an empty cache waits for it, at most about 2 s.

A service check is a `[[sys.service]]` table:

```toml
[[sys.service]]
name = "mail-backlog"            # shown in the views; unique
kind = "command"                 # http | tcp | launchd | process | command
target = ["/bin/sh", "-c", "wc -l < ~/.local/state/kota-mailwatch/pending.jsonl"]
warn = 10                        # optional thresholds: ms for http and tcp, the number for command
fail = 50
log = "~/Library/Logs/kota-mailwatch.log"  # optional: Tail Log reads its last 100 lines
restart = false                  # launchd only: lets the dashboard restart it after a confirm
domain = "gui"                   # launchd only: gui (your agents, default) | system (daemons, via sudo -n)
```

| kind | target | ok when |
|---|---|---|
| `http` | an `http(s)://` URL | curl (3 s) gets a 2xx or 3xx |
| `tcp` | `host:port` | it accepts a connection (3 s) |
| `launchd` | a label in your GUI domain (`gui/<uid>`), or `system` with `domain = "system"` | it runs, or its last exit was 0 |
| `process` | an exact process name | `pgrep -x` finds it |
| `command` | an argv, run without a shell (5 s) | it exits 0. With `warn` or `fail`, the first number it prints is compared |

Higher is worse, unless `fail` is below `warn`.

## The fleet

Each `[[sys.machine]]` table adds a machine to **Fleet**:

```toml
[sys]
refresh_secs = 0                 # 0 (default): no background polling; else at least 30

[[sys.machine]]
name = "mac-pro"                 # shown in the views; unique
via = "ssh"                      # local | flick | ssh
host = "mac-pro"                 # via = "flick": its Flick's name[:port]; default: name
ssh = "jaymin@mac-pro"           # an ssh destination: via = "ssh", and the fallback for via = "flick"
vnc = "vnc://mac-pro"            # optional: Screen Sharing in its ⌘K menu
dash = "http://mac-pro:8080/"    # optional: Open Dash in its ⌘K menu
```

- **`via = "local"`** is this Mac: its own snapshot and its `[[sys.service]]` checks.
- **`via = "flick"`** asks the Mac's Flick for `sys snapshot --json` over Tailscale, as `flick --host <host>` does. That Mac reports its own services, so it can check services that listen only on its loopback. Its network access must be on, with this Mac in its `[remote] peers` ([Remote access](remote.md)). When its Flick is too old for `sys snapshot`, or does not answer, Flick falls back to ssh if `ssh` is set, and `flick sys fleet` notes why.
- **`via = "ssh"`** runs the same probe over `ssh -o BatchMode=yes -o ConnectTimeout=5 <ssh> /bin/sh -s`. That Mac needs no Flick, only key-based ssh: BatchMode never asks for a password. Its checks are the `[[sys.machine.service]]` tables under it. `launchd` and `process` checks run inside the ssh call; `http` and `tcp` run from this Mac. `command` checks are not allowed there.

A `[[sys.machine.service]]` under a `via = "flick"` machine that has `ssh` is checked only when the machine falls back to ssh. It also tells this Mac what it may tail and restart there, over ssh (see Actions). Match its `name` to the name the machine's own Flick reports.

```bash
flick sys fleet              # per machine: fresh|stale|down|pending, name, via, metrics, age; then errors and services not ok
flick --json sys fleet       # {"schema","stale_after_secs","machines":[...]}
```

States: **fresh**; **stale** when the data is older than three times the cadence (45 s, or 3 × `refresh_secs`), or kept after a failed read; **down** when a read failed with nothing to keep; **pending** before the first read. A machine that does not answer keeps its last snapshot, marked stale, with the error.

## Views and actions

**Fleet** (root item `sys:fleet`) shows only while at least one `[[sys.machine]]` exists. It lists a row per machine (status icon, metrics, age) and a row per service of each, then **Herdr Agents**, which opens the [herdr](herdr.md) agents view. ↵ on a machine or service opens the machine view: its state, each probe fact and its services. ↵ on a service there shows its status line.

⌘K on a row:

- **Screen Sharing** (machines with `vnc`) opens the `vnc://` URL.
- **Open Dash** (machines with `dash`) opens the dashboard.
- **Tail Log** (services with `log`) shows the last 100 lines. **Tail Again** reads them again.
- **Restart…** (launchd services with `restart = true`) asks first. The confirm shows the exact command and needs ⌘↵. Locally it runs `launchctl kickstart -k gui/<uid>/<label>`; for a machine it runs `ssh <ssh> launchctl kickstart -k gui/$(id -u)/<label>`. The result also shows as a HUD card in the top right corner for a few seconds (`Restarted …`, or why it failed). `flick sys restart --yes` answers in the terminal instead.

Tail and restart act only on services that this Mac's config defines: its own `[[sys.service]]` (run here) or a `[[sys.machine.service]]` of a machine with `ssh` (run over ssh). A service that only a peer's Flick reports offers neither, because a label, a path or a command never comes from a peer's reply.

**System daemons.** A launchd service is a launch agent in your GUI domain (`gui/<uid>`) unless it says `domain = "system"`: a launch daemon, run as root, such as a Homebrew service started with `sudo brew services`. Its check reads `launchctl print system/<label>`. Its restart runs `sudo -n launchctl kickstart -k system/<label>`, here or over ssh, and the confirm says `launchd system · sudo -n`. Its tail runs plain `tail` first; only when the log is not readable (`Permission denied`) does it run `sudo -n tail` instead.

`sudo -n` never asks for a password, and Flick never types one. When sudo would need one, the restart fails with *sudo needs a password here; Flick never types one…* and nothing runs. To allow it, add a NOPASSWD rule for exactly that command on the Mac it runs on, with `sudo visudo -f /etc/sudoers.d/flick`:

```
jaymin ALL=(root) NOPASSWD: /bin/launchctl kickstart -k system/homebrew.mxcl.ollama
jaymin ALL=(root) NOPASSWD: /usr/bin/tail -n 100 -- /opt/homebrew/var/log/ollama.log
```

Without a rule, restart it in Terminal.

**Fleet window.** ⌘K on **Fleet** → **Open Fleet Window**, or `flick sys window`, shows the fleet in a floating window that stays up while you work: one bubble per machine (name, state, how it was read, age, metrics, one line per service). The dot by the title turns red when a machine is down or a service fails, orange while machines are being read. It polls every 15 s while it shows and stops once hidden. Type in its field and press ↵ to filter machines (name, state, `via`) or services (name, kind, status); ↵ on an empty field shows everything. ⌘R reads every machine now: the notice line says **Refreshing…** while it runs, then **Refreshed** for a few seconds. Esc or ⌘W hides it. `flick sys window --snapshot <png>` draws it into a PNG without showing it. The bubbles sit at the top of the window, as in a dashboard.

⌘K in the window shows a card under each machine with the same actions as ⌘K in the launcher: **Screen Sharing**, **Open Dash**, **Tail <service>** and **Restart <service>…**. ⌘K again hides the cards. A tail shows under the machine's card. **Restart…** asks first on the card: it shows the exact command with **Cancel** and **Run**. The notice line says `Restarting …`, then the result (also a HUD card), or why a press did nothing. With no `vnc`, `dash`, `log` or `restart` set, the notice says so. Hiding the window drops the cards.

```toml
[sys]
hotkey = "cmd+ctrl+alt+shift+KeyW"   # optional: shows the fleet window, or hides it when it has the keyboard
```

```bash
flick sys tail [<machine>] <service>             # the last 100 lines
flick sys restart [<machine>] <service>          # prints the command, runs nothing
flick sys restart [<machine>] <service> --yes    # restarts it
```

## Network

`sys snapshot`, `sys services` and `sys fleet` are read-only, so peers may ask them. That is how a `via = "flick"` fleet reads another Mac. A remote `sys fleet` gets the cache and starts no round, so a peer never makes this Mac ssh. `sys tail` and `sys restart` are refused over the network (`flick: sys restart: not allowed over the network`): they run code and read files. So is `sys window`: it pops a window. The fleet's own actions never go through a peer's Flick. They run here, or over ssh from here.

## Fleet example: laptop, mbp-server, Mac Pro

The laptop shows the fleet. mbp-server runs Flick and checks its own KOTA services, many of them on loopback. The Mac Pro has no Flick and is read over ssh. Tailscale names: `jaymins-macbook-pro`, `mbp-server`, and the Mac Pro at `100.118.223.57`.

`[[sys.machine]]` and `[[sys.service]]` belong to one Mac. If home-manager links one `config.toml` to every Mac, put them in each Mac's [per-host overlay](configuration.md#per-host-overlay) (`config.<host>.toml`), not in the shared file. Otherwise mbp-server would show the laptop's fleet with itself as `via = "local"`, and the laptop would check mbp-server's loopback ports. `[remote] peers` can be shared (list both Macs) or go in the overlays.

**Laptop** (`jaymins-macbook-pro`): the fleet.

```toml
[[sys.machine]]
name = "mbp"
via = "local"

[[sys.machine]]
name = "mbp-server"
via = "flick"
host = "mbp-server"
ssh = "jaymin@mbp-server"        # fallback while its Flick is old or off, and for tail/restart
vnc = "vnc://mbp-server"
dash = "https://mbp-server.tail1b7f44.ts.net:8310"

# What the laptop may tail and restart on mbp-server, over ssh. Same names as mbp-server's
# own [[sys.service]] below, so the rows its Flick reports get these actions.
[[sys.machine.service]]
name = "kota-dash"
kind = "launchd"
target = "org.nix-community.home.kota-dash"
log = "~/Library/Logs/kota-dash.log"
restart = true

[[sys.machine.service]]
name = "kota-memory"
kind = "launchd"
target = "org.nix-community.home.kota-memory"
log = "~/Library/Logs/kota-memory.log"
restart = true

[[sys.machine.service]]
name = "kota-mailwatch"
kind = "launchd"
target = "org.nix-community.home.kota-mailwatch"
log = "~/Library/Logs/kota-mailwatch.log"
restart = true

[[sys.machine.service]]
name = "kota-life-ops"
kind = "launchd"
target = "org.nix-community.home.kota-life-ops"
log = "~/Library/Logs/kota-life-ops.log"
restart = true

[[sys.machine]]
name = "mac-pro"
via = "ssh"
ssh = "jaymin@100.118.223.57"
vnc = "vnc://100.118.223.57"

[[sys.machine.service]]
name = "ollama"
kind = "launchd"
target = "com.ollama.ollama"
log = "~/.ollama/logs/server.log"
restart = true

[[sys.machine.service]]
name = "syncthing"
kind = "launchd"
target = "homebrew.mxcl.syncthing"
log = "/opt/homebrew/var/log/syncthing.log"
restart = true

[[sys.machine.service]]
name = "mlx-serve"
kind = "tcp"
target = "100.118.223.57:11234"
warn = 200
fail = 1000

[[sys.machine.service]]
name = "ollama-api"
kind = "tcp"
target = "100.118.223.57:11434"
warn = 200
fail = 1000

[remote]
peers = ["mbp-server", "jaymins-macbook-pro"]
```

**mbp-server**: its own checks, reported to the laptop through its Flick.

```toml
[[sys.service]]
name = "kota-dash"
kind = "launchd"
target = "org.nix-community.home.kota-dash"
log = "~/Library/Logs/kota-dash.log"
restart = true

[[sys.service]]
name = "kota-dash-ok"
kind = "http"
target = "http://127.0.0.1:8310/ok"
warn = 500
fail = 2000

[[sys.service]]
name = "kota-memory"
kind = "launchd"
target = "org.nix-community.home.kota-memory"
log = "~/Library/Logs/kota-memory.log"
restart = true

[[sys.service]]
name = "kota-memory-port"
kind = "tcp"
target = "127.0.0.1:8300"

[[sys.service]]
name = "kota-mailwatch"
kind = "launchd"
target = "org.nix-community.home.kota-mailwatch"
log = "~/Library/Logs/kota-mailwatch.log"
restart = true

[[sys.service]]
name = "kota-life-ops"
kind = "launchd"
target = "org.nix-community.home.kota-life-ops"
log = "~/Library/Logs/kota-life-ops.log"
restart = true

[[sys.service]]
name = "life-ops-port"
kind = "tcp"
target = "127.0.0.1:8700"

[[sys.service]]
name = "mail-backlog"
kind = "command"
target = ["/bin/sh", "-c", "wc -l < ~/.local/state/kota-mailwatch/pending.jsonl"]
warn = 10
fail = 50

[[sys.service]]
name = "syncthing"
kind = "launchd"
target = "homebrew.mxcl.syncthing"
log = "/opt/homebrew/var/log/syncthing.log"

[remote]
peers = ["mbp-server", "jaymins-macbook-pro"]
```

mbp-server then needs a Flick built with the `sys` module, its config reloaded, and `flick remote on` once (the switch persists). **Mac Pro**: nothing to install. The laptop needs key-based ssh to `jaymin@100.118.223.57` (`ssh -o BatchMode=yes jaymin@100.118.223.57 true` must succeed without a prompt).

## Manual checklist

Unit tests cover the probe parser, the check verdicts, the fleet model, the views and the action commands with fake children. This checklist covers real machines, ssh, the remote transport and launchd. Run it on the laptop, on the installed Flick.app, after a change to `src/modules/sys/` or the peer client.

- [ ] **Local snapshot.** `flick sys snapshot` prints the host, uptime, load, memory, disks, battery and thermal. `flick sys services` lists each `[[sys.service]]` with a verdict (or `no services (...)`).
- [ ] **No fleet, no item.** With no `[[sys.machine]]`, root search has no **Fleet**, and `flick sys fleet` says `no machines (add [[sys.machine]] tables to config.toml)`.
- [ ] **mbp-server via Flick.** On mbp-server: rebuild Flick, set `[remote] peers` to include `jaymins-macbook-pro`, reload, `flick remote on`. On the laptop: `flick --host mbp-server --json sys snapshot` answers. `flick sys fleet` shows `fresh  mbp-server  flick  load ...`.
- [ ] **Fallback to ssh.** On mbp-server, `flick remote off`. On the laptop, open **Fleet**: mbp-server still reads, now via `ssh`, and `flick sys fleet` shows a `note:` line with the reason. Turn it back on.
- [ ] **Mac Pro via ssh.** `ssh -o BatchMode=yes jaymin@100.118.223.57 true` succeeds with no prompt. **Fleet** shows mac-pro `fresh` with load, memory and disk, and its services: ollama, syncthing, mlx-serve, ollama-api.
- [ ] **Unreachable machine.** Sleep or disconnect the Mac Pro. The Fleet view still opens at once. mac-pro turns `stale` with its last metrics and the ssh error (`down` if it never answered). Other machines are not slowed down.
- [ ] **Polling cadence.** With `refresh_secs = 0` and the launcher closed, `flick sys fleet` ages grow (no background reads). With the Fleet view open, ages stay under about 15 s.
- [ ] **Failing service.** Add a check that cannot pass (`name = "probe-fail"`, `kind = "tcp"`, `target = "127.0.0.1:1"`) to this Mac's `[[sys.service]]` and reload. Its row turns fail with a reason, and `flick sys fleet` lists it under the machine. Remove it.
- [ ] **Screen Sharing and Open Dash.** ⌘K on mbp-server: **Screen Sharing** opens Screen Sharing to mbp-server, and **Open Dash** opens kota-dash. ⌘K on mac-pro: **Screen Sharing** connects to 100.118.223.57.
- [ ] **Tail Log.** ⌘K on kota-mailwatch under mbp-server → **Tail Log** shows the last 100 lines of `~/Library/Logs/kota-mailwatch.log` there. **Tail Again** refreshes them. `flick sys tail mbp-server kota-mailwatch` prints the same.
- [ ] **Restart with confirm.** ⌘K on syncthing under mac-pro → **Restart…** shows `ssh jaymin@100.118.223.57 launchctl kickstart -k gui/$(id -u)/homebrew.mxcl.syncthing`. Esc cancels, and nothing restarts. ⌘↵ restarts it, and the footer says `Restarted syncthing on mac-pro`. Also try ollama: if launchctl answers that the service cannot be found in the GUI domain, ollama runs as a system daemon: set its `domain = "system"` (see **System daemons**).
- [ ] **CLI restart needs --yes.** `flick sys restart mac-pro syncthing` prints the command and restarts nothing. `--yes` restarts it.
- [ ] **No actions for peer-only services.** A service that only mbp-server's Flick reports (for example `kota-dash-ok`) has no **Tail Log** or **Restart…** in ⌘K.
- [ ] **Network refusal.** From mbp-server: `flick --host jaymins-macbook-pro sys fleet` answers from the cache. `flick --host jaymins-macbook-pro sys restart mac-pro syncthing --yes` and `... sys tail mbp-server kota-dash` print `not allowed over the network` and exit 1.
- [ ] **Herdr Agents.** The last row of **Fleet** opens the herdr agents view.
- [ ] **Fleet window.** ⌘K on **Fleet** → **Open Fleet Window**: the launcher hides and a window titled Fleet appears under the pointer with a bubble per machine; the app you were in stays active. Ages stay under about 15 s while it shows. Type `pro` + ↵: only mac-pro remains and a notice line shows the filter; ↵ on an empty field brings all back. ⌘R shows Refreshing… then Refreshed in the notice line, ahead of the filter. Esc hides it; reopen it and it keeps its size and place. `flick --host jaymins-macbook-pro sys window` from mbp-server prints `not allowed over the network`.
- [ ] **Fleet window actions.** With few machines, the bubbles sit at the top. ⌘K: a card shows under mbp-server and mac-pro. **Screen Sharing** on mac-pro opens Screen Sharing. **Tail kota-mailwatch** shows its last lines under mbp-server's card. **Restart syncthing…** on mac-pro shows the command with Cancel and Run; Cancel restarts nothing. ⌘K again hides the cards.
- [ ] **Restart toast.** Restart a service from the launcher's ⌘K or the window's card: a `Fleet` HUD card in the top right says `Restarted …` (or why not) and hides after about 6 s. `flick sys restart … --yes` shows no card.
- [ ] **System daemon without NOPASSWD.** Set `domain = "system"` on a launch daemon whose command has no sudoers rule. Its row shows its state. **Restart…** shows `sudo -n launchctl kickstart -k system/<label>` with `launchd system · sudo -n`; Run fails at once with *sudo needs a password here…*; no password prompt appears and nothing restarts.
- [ ] **System daemon with NOPASSWD.** Add the rule from **System daemons** for one daemon. Restart it: `Restarted …`. Tail a root-only log of it: the lines show, and the command shown is `sudo -n tail …`.
- [ ] **Fleet window hotkey.** Set `[sys] hotkey` and reload. The hotkey shows the window; pressed again while the window has the keyboard, it hides it.
- [ ] **Sleep.** Sleep the laptop with the Fleet view open, then wake it. One round runs after wake, and no stale round from before sleep overwrites it.
