# Tasks

The `task` module keeps a short task list (title, optional project, status `todo`, `doing` or `done`) and times the one task that runs. It has no subtasks, estimates, due dates or sync. Tasks and their time stay in `flick.db` on this Mac.

In the launcher, **Start Task** opens a picker of the tasks not done. Type to filter; ↵ starts the selected task. If no task has the typed title, the first row is **Start new task "<title>"**. A trailing `#word` sets the project of the new task and filters the list to that project: `Review PR #kota`. While a task runs, root search shows **Stop Task: Review PR · 0:42** (time today) and **Switch Task**. **Tasks** lists every task: open ones first (the running one on top), then done ones, each with its project, status and time today. Type to search; a trailing `#word` filters by project; ↵ starts the selected task (a done task opens again). **Tasks Today** shows today's totals per task and per project. On a task row in any of these views, ⌘K has **Start Task** (**Stop Task** on the running one), **Mark Done** (**Reopen Task** on a done one), **Rename Task** (a form with title and project) and **Delete Task**. Delete asks first and needs ⌘↵; it removes the task and its tracked time.

```toml
[task]
hotkey = "cmd+ctrl+alt+shift+KeyT" # opens the task picker; unbound by default
```

```bash
flick task start "Write plan" --project flick  # a title that matches no task creates it
flick task switch 3                # same as start: the running task stops, task 3 runs
flick task stop
flick task ls                      # todo and doing tasks, ▶ marks the running one; --all adds done
flick task add "Review PR" --project kota
flick task done 3                  # stops it if it runs; done tasks leave ls but stay in reports
flick task reopen 3                # back to todo
flick task rename 3 "Review PR 12" --project kota  # --project "" clears it; without --project it stays
flick task rm 3                    # deletes the task and its time (stops it if it runs); refused over the network
flick task report week             # also today (the default), 2026-10-01, 2026-10-01..2026-10-07; --project P
flick --json task ls | jq .ok.running.id
flick --json task report today | jq .ok.total_secs
```

`<id|title>` is a task id, an exact title, or a case-insensitive prefix that matches only one task. A prefix that matches several tasks is an error that lists them. Report days are local days; a week is Monday to Sunday.

- **Timer.** At most one task runs. It keeps running through `flick reload` and a restart; the time Flick was not running does not count. Idle (60 s without input) ends the task's time at the last input, and the next input starts it again. Sleep and screen lock pause it at once; wake and unlock resume it. A crash loses the time since the last event. Quit (**Quit Flick**, SIGTERM) ends the open time row. There is no ticking timer: durations are computed when a list or command reads them.
- **Activity.** Task timing works with activity recording off. While recording is on, each activity span carries the id of the task that ran: `flick activity today --by task` gives totals per task id. For the apps used per task, filter the spans: `flick --json activity spans --since today | jq '.ok[] | select(.task == 3)'`.
- **Events.** Each start, switch and stop, and the task restored at startup, is an event: `flick events` prints `{"event":"task_changed","task":3}` (`"task":null` after a stop).

## Manual tests

This checklist covers the parts that unit tests cannot reach: the launcher, sleep and
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

### Checklist

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
   has **Start Task**, **Mark Done**, **Rename Task** and **Delete Task**. On the running task
   it has **Stop Task** in place of **Start Task**. **Mark Done** on the running task stops
   it; the task leaves the picker and `flick task ls`, and still shows in
   `flick task report`.
9. **Stop from the launcher.** Start a task, then run **Stop Task: ...** from root search.
   The status line says `Stopped ...`; root search shows **Start Task** again.
10. **Tasks.** Run **Tasks**. It lists every task: the running one first (`Running ·`),
    then the other open tasks, then done ones (`Done ·`, check mark icon). Type `#kota`: only
    `kota` tasks stay. ↵ on a done task starts it and hides the launcher; `flick task ls`
    shows it running and `doing`.
11. **Manage from Tasks.** In **Tasks**, ⌘K on a done task has **Start Task**,
    **Reopen Task**, **Rename Task** and **Delete Task**. **Reopen Task** puts it back in the
    picker. **Rename Task** opens a form with Title and Project; save `Inbox` / `#admin`:
    the status line says `Renamed Inbox #admin` and `flick task ls` shows the new title.
    A title and project another task has stays in the form with an error. **Delete Task**
    asks first: ↵ does not delete, ⌘↵ does. Deleting the running task stops it
    (`flick events` shows `task_changed` with `null`), and its `task_time` rows are gone.
12. **Manage from the CLI.** `flick task done 3`, `flick task reopen 3`,
    `flick task rename 3 "New title" --project p` and `flick task rm 3` each print one line
    (`Reopened 3: ...`, `Renamed 3: ...`, `Deleted 3: ...`). From a remote peer,
    `flick --host <host> task rm 3` is refused.
13. **Tasks Today.** Run **Tasks Today**. It lists today's totals per task (the running task
    first, with `Running ·`) and per project (`#kota`, `No project`). ↵ on a task row starts
    it.
14. **Idle.** With a task running, leave the machine alone for 2+ min. The open `task_time`
    row ends about 60 s after the last input. The first input opens a new row for the same
    task.
15. **Lock.** With a task running, lock the screen (ctrl+cmd+Q) for 1+ min. The open row
    ends at once and its `end` does not move while locked. After unlock a new row opens.
16. **Sleep.** With a task running, sleep the Mac (Apple menu > Sleep) for 1+ min and wake
    it. The row before sleep ends at the sleep time; after wake (and unlock) a new row opens.
17. **Quit closes the row.** With a task running, note the open row's id, then run
    **Quit Flick**. The row's `end` is the quit time. Start Flick again: the task still runs
    (`flick task ls`), a new row opens, and the time Flick was down is not counted. Repeat
    with `kill <pid>` (SIGTERM): same result.
18. **Reload.** With a task running, run `flick reload`. The task still runs, and no time
    is lost or counted twice.
19. **Activity attribution.** Turn on activity recording (`flick activity on`). Use an app
    for 1 min with no task, then start a task, use two apps for 1 min each, and stop it.
    `flick activity today --by task` shows about 2 min under the task's id and the rest under
    no task. `flick --json activity spans --since today | jq '.ok[] | select(.task == <id>)'`
    lists only the spans between the start and the stop, with both apps.
20. **Hotkey.** Add `[task] hotkey = "cmd+ctrl+alt+shift+KeyT"` and run
    **Reload Flick Config**. The hotkey opens the task picker directly. Remove the line and
    reload: the hotkey does nothing.
21. **CPU.** With a task running and the launcher closed, Activity Monitor shows Flick near
    0% CPU over 60 s.
