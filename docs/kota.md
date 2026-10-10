# KOTA

The `kota` module shows KOTA in the menu bar. KOTA is the always-on Claude Code session in a herdr pane on another Mac, here mbp-server. The menu bar item shows whether KOTA is thinking, waiting on an approval, idle, degraded or down, or whether this Mac is offline. It also shows how many cards are waiting on you. **Ask KOTA…** (a hotkey, the menu, or `flick kota ask`) sends KOTA a question. A pending KOTA card shows until the reply replaces it.

The module is off until `[kota]` sets at least one key. An empty `[kota]` table counts as none. Without a key, Flick runs no thread and no child process for this module, and shows no menu bar item. `flick kota refresh` still checks once on demand. The defaults below fit Jaymin's setup, so `hotkey` alone is enough to turn it on:

```toml
[kota]
hotkey = "cmd+ctrl+alt+shift+KeyO" # opens Ask KOTA; unbound by default
machine = "mbp-server"             # herdr's saved machine that runs KOTA; "local" (or this Mac's host name) is this Mac
cwd = "/Users/jaymin/kota"         # the KOTA pane is agent "claude" in this directory there
pane = ""                          # when several panes match, the one with this herdr pane name wins
herdr = "herdr"                    # the herdr CLI: a name on PATH (then /opt/homebrew/bin) or a path
dash = "https://mbp-server.tail1b7f44.ts.net:8310" # kota-dash; presence reads <dash>/ok
ssh = "jaymin@mbp-server"          # quick ask: the ssh target
kota_ask = ".dotfiles/home/.local/bin/kota-ask"    # kota-ask on it, relative to its home
poll_secs = 60                     # seconds between checks (15 s while KOTA works); 0: only on demand; else at least 15
fast_secs = 180                    # seconds of 15 s checks after an ask
status_item = true                 # show the menu bar item
notify_down = true                 # post a notification when KOTA goes down
```

```bash
flick kota status            # state, KOTA's task, failing checks, last check, errors, cards waiting, polling
flick --json kota status     # {"state","stale","since","pane","failing","errors","checked_at","pending","unread","polling"}
flick kota refresh           # check now (at most once per 10 s)
flick kota ask "what's on today?"   # send a question; answers once ssh is done, exit 1 with the reason if it failed
```

## Presence

Each check (a "round") runs two children in parallel. `herdr [--machine <machine>] agent list` has a 10 s budget. `curl -sS --fail --max-time 5 <dash>/ok` reads kota-dash's checks, a JSON object of booleans. The KOTA pane is the herdr pane with agent `claude` in `cwd`. When several panes match, the pane named `pane` wins, then the focused pane, then the lowest pane id.

| Glyph | State | Meaning |
|---|---|---|
| `K…` | thinking | The KOTA pane is `working`. |
| `K!` | blocked | The pane waits on you (an approval or a question). This outranks a failing check. |
| `K` | idle | The pane is `idle` or `done` and every `/ok` check passes. |
| `K~` | degraded | KOTA is there, but a `/ok` check is false or kota-dash did not answer. It is also degraded when herdr failed while kota-dash answered with `health` true: the server is up, but Flick cannot see the pane. |
| `K×` | down | The server answered and KOTA is not there. Either herdr answered with no KOTA pane, or herdr failed and kota-dash reports `health` false. |
| `K-` | offline | Neither herdr nor kota-dash answered. |
| `K?` | unknown, or stale | No round yet, or the pane has an unknown status. While stale, the data is from before a sleep or lock, and no round has confirmed it yet. |

The cards waiting on you and the posts you have not read follow the glyph, for example `K! 2`; the Inbox row splits them (`Inbox (1 waiting, 1 unread)`). The counts come from the `message` module (`Event::CardsPending`); see [unread posts](message.md#unread-posts).

**Down vs offline.** Down means the server answered and KOTA is not running there. The pane is gone, or kota-dash says it is unhealthy. Fix it on mbp-server. Offline means nothing answered. From the laptop, a dead mbp-server and a laptop with no network look the same, so offline is never called down, and it posts no notification. Check this Mac's network and Tailscale first.

**Debounce.** Down and offline take effect only on the second failing round in a row. The first failing round keeps the last state and retries in 15 s. Sleep and lock stop the timer and mark the state stale (`K?`, `(stale)` in the menu). Wake and unlock run a round after 5 s. Failing rounds within 30 s of a wake keep the last state, because the network may not be back yet.

**Cadence.** A round runs every `poll_secs` (60 s), every 15 s while KOTA is thinking or blocked, every 15 s for `fast_secs` after an ask, and every 15 s while the Ask KOTA view shows. With `poll_secs = 0`, rounds run only on demand: on `kota refresh`, when the launcher opens, and when the menu opens.

## Menu bar item

The item shows only after Flick starts, while `[kota]` sets a key and `status_item` is true. Its tooltip is the state line and KOTA's current task (the pane's terminal title). While it shows, the age in the state line (`· 4m`) updates every minute, also between checks. The menu, top to bottom:

- The state line (`KOTA: thinking · 4m`), KOTA's task, `Checks: <names> failing` (`herdr` or `dash` when that source failed), and `Checked 12:03`. The last row says `stale` while the state is stale.
- **Ask KOTA…** opens the Ask KOTA view.
- **Open Chat** opens the [KOTA chat window](message.md#chat), the one `[message] chat_hotkey` toggles, on the thread it showed last. Unlike the hotkey, it never hides the window.
- **Inbox** (`Inbox (2 waiting)`) opens the `message` module's cards view: its cards and the posts you have not read, newest first. Opening it reads those posts. The full history is the `message` module's message list.
- **Open Dashboard** opens `dash` in the browser.
- **Refresh Now** checks now.

Opening the menu also checks now, at most once per 10 s. When the state changes to down and `notify_down` is true, Flick posts one notification: `KOTA is down` and `herdr on mbp-server has no KOTA pane` (or `kota-dash on mbp-server reports health false`). Notifications need Flick.app. macOS asks for permission on the first launch.

## Quick ask

**Ask KOTA…** opens the launcher view `kota/ask`. Open it with the `hotkey` or from the menu bar item. Type the question; ↵ sends it, and the launcher hides. The footer shows KOTA's state. The **Open Chat** row is always in the view: it is first while the field is empty, so the hotkey then ↵ opens the [KOTA chat window](message.md#chat) on its last thread (the launcher hides). Use ↓ to get to it after you type. The text area lists the last 8 asks (memory only) with `sending…`, `sent` or `failed: <why>`. `flick kota ask <text>` does the same from a shell.

An ask:

1. Makes a request id (`k` plus the time in ms and a counter, base 36).
2. Posts a pending KOTA card with that id (`message post --pending --id <id> --title KOTA`).
3. Sends the text on stdin to `ssh -o BatchMode=yes -o ConnectTimeout=8 <ssh> <kota_ask> --id <id>` (15 s budget). The text is never in an argv.
4. If ssh fails, posts `KOTA ask failed: <reason>` as the reply to the pending card, so the hourglass does not stay.

KOTA's reply (`flick --host <laptop> message post --reply-to <id> ...`) replaces the pending card. A question can have at most 2000 characters. `ssh` needs key-based login to `ssh` (BatchMode never prompts).

## Network

`kota status` (cached, no I/O) and `kota refresh` (read-only and rate-limited) answer over the network. `kota ask` does not: no peer, KOTA included, can make this Mac ssh text to KOTA. A peer gets `flick: kota ask: not allowed over the network`.

## Moving off the `k` script command

Before this module, the dotfiles had a script command for asking KOTA:

```toml
[[script.commands]]
name = "Ask KOTA"
keyword = "k"
shell = "id=$(\"$HOME/Applications/Flick.app/Contents/MacOS/Flick\" message post --pending -- {query} 2>/dev/null); printf %s {query} | /usr/bin/ssh -o BatchMode=yes -o ConnectTimeout=8 mbp-server .dotfiles/home/.local/bin/kota-ask --id \"$id\""
```

Replace it once the laptop runs a Flick with this module (`flick kota status` answers instead of `unknown module`):

1. Add `[kota]` with a `hotkey` (and any key that differs from the defaults) to the laptop's config. When one config.toml is shared between Macs, put it in the laptop's [per-host overlay](configuration.md#per-host-overlay), so mbp-server does not poll too.
2. Remove the `[[script.commands]]` "Ask KOTA" table and run **Reload Flick Config**.
3. Ask KOTA with the hotkey, the menu bar item's **Ask KOTA…**, or `flick kota ask <text>` in a shell.

The module makes the request id itself, so kota-ask never gets an empty `--id`. The script got one when the `message post` call failed (seed flick-7b52). A failed ssh now replaces the pending card with the reason. Root search no longer takes `k <text>`. If you want the keyword back, a script command can call the verb: `shell = "\"$HOME/Applications/Flick.app/Contents/MacOS/Flick\" kota ask {query}"`.

## Manual smoke checklist

Unit tests cover the presence model, the menu model and the ask argv with fake children. This checklist covers the parts they cannot reach: the real herdr and kota-dash, ssh to mbp-server, the menu bar and notifications. Run it on the laptop, on the installed Flick.app, after a change to `src/modules/kota/` or `platform::status_item`.

- [ ] **Off without a key.** With no `[kota]` table (or an empty one), there is no K item in the menu bar, and `flick kota status` ends with `polling: off (set a key in [kota] to poll)`.
- [ ] **On with a key.** Add `hotkey = "cmd+ctrl+alt+shift+KeyO"` under `[kota]` and reload. Within a few seconds the menu bar shows `K` (or `K…` while KOTA works). `flick kota status` shows `KOTA: idle · <age>`, `Checked HH:MM` and `polling: every 60 s`.
- [ ] **Thinking and blocked.** Give KOTA a task. Within 60 s the glyph turns `K…` and the tooltip shows its task. When KOTA asks for an approval, the glyph turns `K!` within 15 s.
- [ ] **Pending count.** Have KOTA post a card that waits on you. The title shows `K 1` (or the current glyph plus 1), and the menu row reads `Inbox (1 waiting)`. **Inbox** opens the cards view, which lists that card.
- [ ] **Age.** Leave the menu bar alone for 2 minutes with `poll_secs = 0`. The tooltip's age (`· 2m`) moves without a check.
- [ ] **Menu.** Open the menu: it lists the state line, KOTA's task, `Checked HH:MM`, then **Ask KOTA…**, **Open Chat**, **Inbox**, **Open Dashboard** and **Refresh Now**. Opening it updates `Checked`. **Open Dashboard** opens kota-dash in the browser. **Open Chat** shows the chat window on its last thread; choose it again while the window shows, and the window stays.
- [ ] **Open Chat from the hotkey.** Press the hotkey: the first row is **Open Chat** and the footer says `↵ opens chat`. Press ↵: the launcher hides and the chat window shows on its last thread. Press the hotkey, type `hi`: the rows are **Ask KOTA: hi** then **Open Chat**, and the footer says `↵ sends`.
- [ ] **Ask from the hotkey.** Press the hotkey, type `smoke test: reply ok`, press ↵. A pending KOTA card appears at once. KOTA's reply replaces it. `flick kota status` polls fast (the glyph tracks KOTA within 15 s) for 3 minutes.
- [ ] **Ask from the CLI.** `flick kota ask "smoke test from the cli"` prints `Asked KOTA (k…): queued for KOTA (…)` (kota-ask's last line) and exits 0.
- [ ] **Ask failure.** Set `ssh = "jaymin@nonexistent-host"` and reload. `flick kota ask hi` exits 1 with ssh's error. The pending card turns into `KOTA ask failed: <reason>`. Restore `ssh`.
- [ ] **Degraded.** Make one kota-dash `/ok` check false (or stop kota-dash). Within two rounds the glyph is `K~`, and the menu shows `Checks: <name> failing` (`dash` when kota-dash is stopped). Restore it.
- [ ] **Down and its notification.** In a test herdr session, or with `cwd` pointed at a directory with no Claude pane, wait two rounds (about 15 s after the first). The glyph turns `K×`, and one notification says `KOTA is down` / `herdr on mbp-server has no KOTA pane`. No second notification comes while it stays down. Restore `cwd`.
- [ ] **Offline, no notification.** Turn off Wi-Fi (or quit Tailscale). After two rounds the glyph is `K-`, and no notification is posted. Turn the network back on and choose **Refresh Now**: the glyph returns.
- [ ] **Sleep and wake.** Sleep the laptop for a few minutes and wake it. The glyph shows `K?` (the menu says `stale`) until the first round, about 5 s after unlock, and then the real state. No false `K×` or `K-` appears during the first 30 s.
- [ ] **Network refusal.** From mbp-server: `flick --host jaymins-macbook-pro kota status` answers. `flick --host jaymins-macbook-pro kota ask hi` prints `flick: kota ask: not allowed over the network` and exits 1.
- [ ] **Hidden item.** Set `status_item = false` and reload: the item goes away, and `flick kota status` still answers. Set it back.
- [ ] **Migration.** After you remove the `k` script command and reload, `k hello` in root search no longer runs a command. Every ask path above still works.
