# Remote access

The `remote` module lets other Macs in your [Tailscale](https://tailscale.com) tailnet send Flick commands: `flick --host <this Mac> task ls` on another machine answers as `flick task ls` would here. It is off by default. It needs `peers` in `[remote]` and the switch on: run `flick remote on` or **Turn On Network Access** in the launcher. **Turn Off Network Access** or `flick remote off` closes the listener and every open connection. Only a local caller can turn it on or off.

```toml
[remote]
peers = ["mbp-server"]   # Tailscale machine names allowed to connect; [] means no listener
port = 7419              # default 7419
events = false           # true lets peers stream `flick --host <mac> events`
```

```bash
flick remote status            # on/off, listening addresses, peers, last connection and error
flick --json remote status | jq .ok.last
```

- **Who can connect.** Flick listens only on this Mac's Tailscale addresses (100.64.0.0/10 and fd7a:115c:a1e0::/48), never on a LAN or wildcard address. Before it reads a request, it asks `tailscale whois` for the caller's machine name and closes the connection unless that name is in `peers`. Tailscale ACLs still apply. `flick remote status` shows the last connection, allowed or refused, with the name Tailscale gave, so you can fix `peers`.
- **What peers can do.** Every network request runs as a remote caller (`--remote`), whatever the client sends. Peers cannot `reload`, `flick rebuild` or `cancel`, `keys fire`, `app uninstall`, `quicklink add` or `remove`, `capture` anything, `feedback resolve`, `task rm`, `script run`, `message card press` or `card focus`, any `dictation` or `llm` verb, `kota ask`, or `sys restart`, `sys tail` or `sys window`, and of `remote` only `remote status`. Activity data needs the same grant as a local agent (`flick activity remote allow`); a phone adds its app opens with `activity phone add` without it ([Phone events](#phone-events)). Everything else (tasks, herdr, `kota status` and `refresh`, `sys snapshot`, `services` and `fleet`, `feedback add` and `ls`, windows, app list and open, clipboard) answers.
- **Privacy.** With network access on, the peers you name can read what those commands return, clipboard history included. Name only machines you control. Nothing is sent anywhere: Flick only answers.

## Phone events

flick-ios (`../flick-ios`) sends one event per app open on the iPhone, so KOTA can read phone use through Flick next to the Mac's [activity](activity.md). The receiver is the Mac Pro (`jaymins-mac-pro`): it is always on. This section is the wire contract; flick-ios follows it.

Setup on the receiver: put the iPhone's Tailscale machine name in `[remote] peers` (`tailscale status` lists it, for example `jaymins-iphone`), reload, and run `flick remote on` once. Adding opens needs no activity grant and works with activity recording off. KOTA reads them like Mac activity: `flick --host jaymins-mac-pro activity today --json --remote`, with KOTA's Mac in `peers` and `flick activity remote allow` on the receiver.

**Transport.** TCP to `jaymins-mac-pro:7419` (the receiver's `[remote] port`). A request is one line: a JSON array of strings in UTF-8, then `\n`, at most 64 KiB. The reply is one line, `{"ok":...}` or `{"error":"..."}`. One connection may carry many requests: send one line, read its reply, then send the next. Flick closes a connection after 30 s without a request. A peer not in `peers` is closed before any reply.

**Request.**

```json
["activity","phone","add","--id","6F1C2B9E-6F0B-4C55-9F0A-2D5C3B7E8A10","--device","jaymins-iphone","--app","com.apple.mobilesafari","--at","1760112000","--json"]
```

Flags may come in any order, each once. `--json` is optional and must be the last word; without it the reply is a text line (`Stored phone open <id> from <device>` or `Already stored phone open <id> from <device>`).

| Flag | Required | Rule |
|---|---|---|
| `--id` | yes | The phone's id for this event, the same on every retry. 1 to 64 characters of `A-Z a-z 0-9 . _ - :`. A UUID string fits. |
| `--device` | yes | The phone's name: its Tailscale machine name by convention (`jaymins-iphone`). 1 to 64 characters of `A-Z a-z 0-9 . _ -`. Ids are unique per device. Flick does not check it against the peer's Tailscale name. |
| `--app` | yes | Bundle id or app name, as the phone knows it (`com.apple.mobilesafari`, `Instagram`). 1 to 128 characters, no control characters (tab, newline), no space at either end. |
| `--at` | yes | When the app opened: unix seconds, digits only (no sign, fraction or milliseconds). At most 30 days before the receiver's clock and 5 min after it. |
| `--reason` | no | The intention prompt's answer. 1 to 280 characters, the same rules as `--app`. |
| `--minutes` | no | Minutes the prompt granted, digits only, 1 to 1440. |

**Replies and retries.** The phone keeps each open in its queue until it gets one of these:

| Reply | Meaning | Phone |
|---|---|---|
| `{"ok":{"device":"jaymins-iphone","id":"6F1C...","stored":true}}` | Stored. | Drop it from the queue. |
| `{"ok":{...,"stored":false}}` | The same event (same device and id, same fields) was already stored: an earlier send got through but its reply did not. | Drop it. |
| `{"error":"activity phone: ..."}` | Refused for good: a bad field (`activity phone: bad --at: ...`), or the id is stored with other fields. A retry gives the same answer. | Drop it and log the error. |
| any other `{"error":...}` | The receiver cannot take it now: an older Flick (`activity: unknown command "phone"`), activity disabled (`unknown module "activity"`), a database error (`activity: could not store the phone open: ...`). | Keep it; retry later. |
| no reply (cannot connect, closed, timeout) | Network access off, Tailscale down, receiver asleep, not in `peers`. | Keep it; retry later. |

To test from a Mac peer: `flick --host jaymins-mac-pro activity phone add --id t1 --device test --app flick-test --at $(date +%s)`, then `flick activity forget app flick-test --yes` on the receiver.

## Peers for the fleet

The [fleet](sys.md) reads a `via = "flick"` machine through that Mac's Flick, so the network access works the other way round from the setup below. The Mac that shows the fleet (the laptop) is the peer, and the Mac it reads (mbp-server) is the host. On mbp-server:

```toml
[remote]
peers = ["jaymins-macbook-pro"]   # the laptop's Tailscale machine name (`tailscale status`)
```

Then reload and run `flick remote on` there once (the switch survives restarts). mbp-server's Flick must be new enough to have `sys snapshot`. Until then, or while its network access is off, the laptop falls back to ssh when the machine has `ssh` set. When one `config.toml` is shared between both Macs, list both names (`peers = ["mbp-server", "jaymins-macbook-pro"]`), or give each Mac its own `peers` in its [per-host overlay](configuration.md#per-host-overlay): naming itself does no harm, and network access stays off on each Mac until `flick remote on` there. A peer only reads the fleet's data: `sys tail` and `sys restart` are refused over the network, and the laptop runs them over ssh.

## Setup and manual tests

This section sets network access up between two Macs and lists the manual tests for the
parts that unit tests cannot reach: the real `tailscale` CLI, real peers and real sockets.
Nothing here runs by itself: do each step by hand, on the installed Flick.app, after a change
to `src/control/net.rs`, `src/control/tailscale.rs`, `src/modules/remote/` or `net_policy`.

Names below: **host** is the Mac that runs Flick with network access on (its MagicDNS name is
`<host>`); **peer** is the second Mac (here `mbp-server`), which needs only the `flick`
binary. Both must be in the same tailnet, and the Tailscale ACLs must let the peer reach the
host on the port (default 7419).

### Setup

On the host, in `~/.config/flick/config.toml`:

```toml
[remote]
peers = ["mbp-server"]
```

Then **Reload Flick Config**. The peer's name is its Tailscale machine name (`tailscale
status` on the host lists it). Case does not matter; `mbp-server` also matches
`mbp-server.tail1234.ts.net`.

To see what the host knows:

```bash
flick remote status           # on/off, listening, peers, last connection, error
flick --json remote status | jq .ok
lsof -nP -iTCP -sTCP:LISTEN | grep -i flick
```

Turning access on or off is async: it asks Tailscale on a background thread. Right after
`flick remote on`, `remote status` can still say `not listening` for a moment (up to about
2 s, longer when Tailscale is slow). Run it again before you call a step failed.

### Checklist

1. **Off by default.** On a fresh `flick.db` (or after `flick remote off`), with `peers` set:
   `flick remote status` says `network access: off` and `listening: not listening`, and
   `lsof -nP -iTCP | grep -i flick` shows nothing. The launcher shows **Turn On Network
   Access**.
2. **Enable.** On the host, run `flick remote on` (or **Turn On Network Access**). After a
   moment `flick remote status` shows `network access: on` and `listening:` with the host's
   Tailscale addresses on port 7419 (from `tailscale ip`), nothing else. `lsof -nP -iTCP
   -sTCP:LISTEN | grep -i flick` shows only those addresses, never `*:7419`, `0.0.0.0` or a
   LAN address. The launcher item now reads **Turn Off Network Access**.
3. **No peers, no listener.** Set `peers = []` and reload. `remote status` says
   `error: no peers: add Tailscale names to [remote] peers` and `lsof` shows no listener.
   Restore `peers` and reload; it listens again.
4. **Allowed request.** On the peer: `flick --host <host> task ls`, then
   `FLICK_HOST=<host> flick task ls` and `flick --host <host>:7419 --json herdr ls`. Each
   prints what a remote caller gets on the host (compare with
   `FLICK_REMOTE=1 flick task ls` there) and exits 0. On the host, `remote status` shows
   `last connection: mbp-server (<peer ip>) allowed <n>s ago`.
5. **Refused peer.** Remove `mbp-server` from `peers` (keep another name) and reload. On the
   peer, `flick --host <host> task ls` prints `flick: Flick closed the connection` and exits
   1; no reply comes. On the host, `remote status` shows `last connection: mbp-server (<ip>)
   refused ...: mbp-server is not in [remote] peers`. Restore `peers`.
6. **Refused verb.** From the peer, each of these prints `flick: <request>: not allowed over
   the network`, exits 1 and changes nothing on the host: `flick --host <host> reload`,
   `flick --host <host> remote off`, `flick --host <host> capture screen`,
   `flick --host <host> keys fire x`, `flick --host <host> quicklink add x https://x`,
   `flick --host <host> feedback resolve x`, `flick --host <host> task rm 1`,
   `flick --host <host> script run x`,
   `flick --host <host> flick rebuild`. Then
   `flick --host <host> events` prints `flick: events: not allowed over the network`; with
   `events = true` in `[remote]` (and a reload) it streams instead.
7. **Activity grant.** With activity recording on, from the peer: `flick --host <host>
   activity today` is refused with activity's `remote use not permitted` text. On the host,
   run `flick activity remote allow`; the same request from the peer now answers. Revoke the
   grant afterwards.
7a. **Phone events.** With no activity grant, from the peer: `flick --host <host> activity
   phone add --id t1 --device test --app flick-test --at $(date +%s)` prints `Stored phone open
   t1 from test`; the same request with the same `--at` number prints `Already stored ...`,
   with another `--at` it prints `... is already stored with other fields`; with `--at 1` it prints
   `flick: activity phone: bad --at: ...` and exits 1. On the host, `flick activity spans`
   lists `<time>\topen\tflick-test\tflick-test\ttest`. Clean up with `flick activity forget app
   flick-test --yes`.
8. **Toggle off closes the listener.** Keep `flick --host <host> events` streaming on the
   peer (with `events = true`). On the host, run `flick remote off` (or **Turn Off Network
   Access**). The stream on the peer ends; `lsof -nP -iTCP | grep -i flick` on the host
   shows no listener and no established connection; a new `flick --host <host> task ls`
   prints `can't reach Flick at <host>:7419 (...); is its network access on?` and exits 3.
9. **Tailscale down.** On the host, quit Tailscale (or `tailscale down`), then
   `flick remote on`. `remote status` shows `network access: on`, `not listening` and an
   error such as `no Tailscale address to listen on (is Tailscale up?)` or the CLI's error.
   Start Tailscale again, then open the launcher (or wake the Mac): Flick retries, and
   `remote status` shows it listening without a restart.
10. **Restart keeps the switch.** With access on, quit and start Flick. It listens again
    without `flick remote on`. With access off, it stays off.
11. **Restarts free the port.** With access on, run `flick remote off` and `flick remote on`
    three times, then change `peers` and reload. Each time `remote status` lists both
    Tailscale addresses (IPv4 and IPv6) with no error, and `lsof -nP -iTCP:7419
    -sTCP:LISTEN` shows one Flick listener per address.

### Address already in use

`remote status` names each address that did not bind, for example
`error: [fd7a:115c:a1e0::1]:7419: port 7419 is in use by Flick (pid 123): another Flick is
running`. Flick asks `lsof` what holds the port and says one of:

- `another Flick is running`: quit the other Flick (`pgrep -fl Flick`), then run `flick
  remote off` and `flick remote on`.
- `this Flick ...: an earlier listener did not close`: a bug; file it with the output of
  `lsof -nP -iTCP:7419`.
- another program's name: change `[remote] port`, or stop that program.
- `another process (another Flick running?)`: `lsof` could not tell; run
  `lsof -nP -iTCP:7419` yourself.

Flick retries a bind that finds the port in use for about half a second, so a Flick that is
quitting does not cause this error.
