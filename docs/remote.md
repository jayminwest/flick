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
- **What peers can do.** Every network request runs as a remote caller (`--remote`), whatever the client sends. Peers cannot `reload`, `flick rebuild` or `cancel`, `keys fire`, `app uninstall`, `quicklink add` or `remove`, `capture` anything or `feedback add` or `resolve`, and of `remote` only `remote status`. Activity data needs the same grant as a local agent (`flick activity remote allow`). Everything else (tasks, herdr, windows, app list and open, clipboard) answers.
- **Privacy.** With network access on, the peers you name can read what those commands return, clipboard history included. Name only machines you control. Nothing is sent anywhere: Flick only answers.

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
   `flick --host <host> feedback add x`, `flick --host <host> flick rebuild`. Then
   `flick --host <host> events` prints `flick: events: not allowed over the network`; with
   `events = true` in `[remote]` (and a reload) it streams instead.
7. **Activity grant.** With activity recording on, from the peer: `flick --host <host>
   activity today` is refused with activity's `remote use not permitted` text. On the host,
   run `flick activity remote allow`; the same request from the peer now answers. Revoke the
   grant afterwards.
8. **Toggle off closes the listener.** Keep `flick --host <host> events` streaming on the
   peer (with `events = true`). On the host, run `flick remote off` (or **Turn Off Network
   Access**). The stream on the peer ends; `lsof -nP -iTCP | grep -i flick` on the host
   shows no listener and no established connection; a new `flick --host <host> task ls`
   prints `can't reach Flick at <host>:7419 (...); is its network access on?` and exits 1.
9. **Tailscale down.** On the host, quit Tailscale (or `tailscale down`), then
   `flick remote on`. `remote status` shows `network access: on`, `not listening` and an
   error such as `no Tailscale address to listen on (is Tailscale up?)` or the CLI's error.
   Start Tailscale again, then open the launcher (or wake the Mac): Flick retries, and
   `remote status` shows it listening without a restart.
10. **Restart keeps the switch.** With access on, quit and start Flick. It listens again
    without `flick remote on`. With access off, it stays off.
