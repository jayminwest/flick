# Herdr: manual tests

The [README](../README.md#herdr) describes the `herdr` module and its settings. This file is
a manual test checklist for the parts that unit tests cannot reach: the real herdr server,
remote machines over SSH, the terminal and macOS notifications. Nothing here runs by itself:
do each step by hand, on the installed Flick.app, after a change to `src/modules/herdr/` or
`platform::notify`.

Use a test herdr session for any step that stops or changes herdr, never the one your agents
run in. To see what Flick knows, use the CLI:

```bash
flick herdr status                 # per machine: live/polled, last read, error; notifications
flick herdr ls                     # every agent, waiting first
flick --json herdr ls | jq '.ok.machines | keys'
```

## Checklist

1. **Lists every machine.** With herdr running here and on the saved machines, open
   **Herdr Agents**. Each agent from `herdr agent list` and from
   `herdr --machine <label> agent list` shows once, blocked first, then done, idle, working.
   `flick herdr status` shows `local` as `live` and the others as `polled`.
2. **Local status within 1 s.** Leave the agents view open. Make a local agent ask for an
   approval. Its row moves to the top within 1 s, without reopening the view.
3. **`pane.updated`.** While step 2 runs, note whether herdr also sends `pane.updated` (for
   example with `herdr api` tooling or a debug build that logs events). The module relies on
   per-pane `pane.agent_status_changed`; record what you see in the seed, so the next change
   can drop or keep that subscription.
4. **Remote status.** Make a remote agent ask for an approval. With the launcher open its row
   moves to the top within 15 s plus one SSH round trip; with `remote_refresh_secs = 60` it
   does so within 65 s with the launcher closed.
5. **Local jump.** ↵ on a local agent: the launcher hides, herdr focuses the agent's pane and
   WezTerm comes to the front.
6. **Remote focus follow.** ↵ on a remote agent while your herdr client is attached to that
   machine, then while it is attached to another. Note whether the local client switches to
   the remote pane. If it does not, record it in the seed: the module should then show a
   status line telling you to press prefix+w (not built yet).
7. **Unreachable machine.** Put one remote Mac to sleep (or set `machines` to include a
   label whose host is off). The view still opens at once. The machine shows as a row with
   subtitle `<error>, last read <age> ago` (the last good read); ↵ on it shows the same
   text in the status line. `flick herdr status` shows the same error. Other machines are
   not slowed down.
8. **herdr not running.** Stop the test herdr server. The view shows `local` with
   `herdr disconnected` (or the connect error). Start it again and open the launcher: the
   agents return without a Flick restart.
9. **Notification.** With the default `notify = ["blocked"]`, make a local agent ask for an
   approval while another app is in front. One notification shows: `<name> is waiting`,
   `<machine> · <cwd> · <title>`, no agent output. On the first launch macOS asks for
   permission; `flick herdr status` then prints `notifications: on (blocked)`.
10. **Click jumps.** Click the notification from step 9: herdr focuses that agent's pane and
    WezTerm comes to the front, as with ↵.
11. **No spam.** Run **Reload Flick Config**, quit and restart the herdr client, and open the
    launcher a few times: no notification for agents that were already blocked. An agent
    that stays blocked posts no second one. A remote agent that turns blocked posts one, on
    the next poll. With that agent's pane focused and WezTerm in front, a new block posts
    none.
12. **Done is opt-in.** An agent that finishes posts nothing until `notify` includes
    `"done"`; then it posts `<name> is done`. `notify = []` posts none and
    `flick herdr status` prints `notifications: off`.
13. **Nothing stored.** After **Show Output** on an agent, pick a word from its output:
    `sqlite3 ~/Library/Application\ Support/Flick/flick.db .dump | grep -c <word>` prints 0,
    and the word is not in Flick's log.
14. **Read-only.** `grep -rn 'agent.prompt\|send_keys\|pane.send' src/modules/herdr` finds
    nothing.
15. **Idle CPU.** With agents idle and the launcher closed, Flick stays at about 0% CPU over
    60 s in Activity Monitor.
16. **Off.** `[herdr] enabled = false` and a reload: **Herdr Agents** is gone, and no
    `herdr-*` thread remains (`sample Flick 1 | grep herdr` prints nothing).
