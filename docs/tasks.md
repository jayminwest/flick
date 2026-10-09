# Tasks: manual tests

The [README](../README.md#tasks) describes the `task` module and its settings. This file is a
manual test checklist for the parts that unit tests cannot reach: the launcher, sleep and
lock, quit, the hotkey and the link to activity recording. Nothing here runs by itself: do
each step by hand, on the installed app, after a change to `src/modules/tasks/`,
`src/core/track.rs`, `Event::TaskChanged` or the task attribution in `src/modules/activity/`.

To see what is stored, read the database with a second connection:

```bash
db=~/Library/Application\ Support/Flick/flick.db
sqlite3 "$db" 'select id, title, project, status from task_list order by id desc limit 10'
sqlite3 "$db" 'select id, task, start, end, end - start from task_time order by id desc limit 10'
sqlite3 "$db" 'select * from task_state'
```

Keep `flick events | grep task_changed` running in a second terminal for the whole list.

## Checklist

1. **Start from the CLI.** `flick task start "Write plan" --project flick` prints
   `Started Write plan #flick`. `flick events` shows one `{"event":"task_changed","task":<id>}`.
   `flick task ls` prints `Running: Write plan (...)` and marks the row with `▶`.
2. **Report.** Wait 2 min with input. `flick task report` shows the task's time within 5 s
   of the wall clock, under the task and under project `flick`.
   `flick --json task ls | jq .ok.running.id` and
   `flick --json task report today | jq .ok.total_secs` print numbers.
3. **Switch from the CLI.** `flick task switch "Review PR" --project kota` prints
   `Switched to Review PR #kota`. The `Write plan` row in `task_time` has its `end` set, a new
   row opens for `Review PR`, and `flick events` shows one `task_changed` with the new id.
4. **Stop from the CLI.** `flick task stop` prints `Stopped Review PR #kota`.
   `flick events` shows `{"event":"task_changed","task":null}`, `flick task ls` prints
   `No task running`, and the last `task_time` row's `end` no longer moves.
5. **Start from the launcher.** Open the launcher and run **Start Task**. The picker lists
   the tasks not done. Type `Fix login #kota`: the first row is
   **Start new task "Fix login"** with subtitle `#kota`, and the list shows only `kota`
   tasks. Press ↵: the launcher hides, and `flick task ls` shows `Fix login #kota` running.
6. **Root items while running.** Open the launcher. Root search shows
   **Stop Task: Fix login · 0:01** (time today) and **Switch Task**, and no **Start Task**.
   Typing `start` finds **Switch Task**.
7. **Switch from the launcher.** Run **Switch Task**. The running task is first; select
   another task and press ↵. `flick task ls` shows the new task running, and `flick events`
   shows one `task_changed`.
8. **⌘K actions.** In the picker, select a task that does not run and press ⌘K: the menu
   has **Start Task** and **Mark Done**. On the running task it has **Mark Done** and
   **Stop Task**. **Mark Done** on the running task stops it; the task leaves the picker and
   `flick task ls`, and still shows in `flick task report`.
9. **Stop from the launcher.** Start a task, then run **Stop Task: ...** from root search.
   The status line says `Stopped ...`; root search shows **Start Task** again.
10. **Tasks Today.** Run **Tasks Today**. It lists today's totals per task (the running task
    first, with `Running ·`) and per project (`#kota`, `No project`). ↵ on a task row starts
    it.
11. **Idle.** With a task running, leave the machine alone for 2+ min. The open `task_time`
    row ends about 60 s after the last input. The first input opens a new row for the same
    task.
12. **Lock.** With a task running, lock the screen (ctrl+cmd+Q) for 1+ min. The open row
    ends at once and its `end` does not move while locked. After unlock a new row opens.
13. **Sleep.** With a task running, sleep the Mac (Apple menu > Sleep) for 1+ min and wake
    it. The row before sleep ends at the sleep time; after wake (and unlock) a new row opens.
14. **Quit closes the row.** With a task running, note the open row's id, then run
    **Quit Flick**. The row's `end` is the quit time. Start Flick again: the task still runs
    (`flick task ls`), a new row opens, and the time Flick was down is not counted. Repeat
    with `kill <pid>` (SIGTERM): same result.
15. **Reload.** With a task running, run `flick reload`. The task still runs, and no time
    is lost or counted twice.
16. **Activity attribution.** Turn on activity recording (`flick activity on`). Use an app
    for 1 min with no task, then start a task, use two apps for 1 min each, and stop it.
    `flick activity today --by task` shows about 2 min under the task's id and the rest under
    no task. `flick --json activity spans --since today | jq '.ok[] | select(.task == <id>)'`
    lists only the spans between the start and the stop, with both apps.
17. **Hotkey.** Add `[task] hotkey = "cmd+ctrl+alt+shift+KeyT"` and run
    **Reload Flick Config**. The hotkey opens the task picker directly. Remove the line and
    reload: the hotkey does nothing.
18. **CPU.** With a task running and the launcher closed, Activity Monitor shows Flick near
    0% CPU over 60 s.
