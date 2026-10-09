# Activity: manual tests

The [README](../README.md#activity) describes the `activity` module and its settings. This
file is a manual test checklist for the parts that unit tests cannot reach: the menu bar
indicator, Accessibility title changes, sleep and lock, and quit. Nothing here runs by
itself: do each step by hand, on the installed app, after a change to `src/modules/activity/`,
`src/core/track.rs`, `platform::axwatch`, `platform::status_item` or `events::on_session`.

To see what is stored, read the database with a second connection:

```bash
db=~/Library/Application\ Support/Flick/flick.db
sqlite3 "$db" 'select id, start, end, end - start, app, title from activity_spans order by id desc limit 10'
sqlite3 "$db" 'select * from activity_state'
```

## Checklist

1. **Off by default.** On a database with no `activity_state` rows, switch apps for a
   minute. `select count(*) from activity_spans` prints 0, `flick activity status` prints
   `recording: off`, and the menu bar has no `●`.
2. **Indicator.** Run **Start Activity Recording** from the launcher. The `●` shows at once;
   its tooltip reads "Flick is recording activity".
3. **Indicator does not steal focus.** With recording on, open the launcher and type: the
   text goes to the launcher, and the app under it stays active (its menu bar stays). Click
   the `●` and close its menu with Escape: the app you were in stays frontmost, and no
   Flick window or Dock icon shows. Open the launcher again: it still takes keys at once.
   Clipboard paste, the window switcher, window keys and the desktop toggle work as before.
4. **Spans.** Switch Safari -> WezTerm -> Mail for about 1 min each. `flick activity today`
   lists the three apps, each within 5 s of the real time. `flick activity spans` lists one
   line per switch. A quick alt-tab (under `merge_secs`) leaves no row.
5. **Titles off.** With `titles = false` (the default), every new row has `title` NULL, and
   `flick activity status` prints `titles: off`.
6. **Titles on, permission message.** Set `titles = true` and run **Reload Flick Config**.
   With Accessibility granted, `flick activity status` prints `titles: on`, and switching
   browser tabs or terminal windows makes separate spans with titles. Remove Flick from
   System Settings > Privacy & Security > Accessibility: status prints
   `titles: no Accessibility permission`, and new spans are per app with no title. Grant it
   again afterwards.
7. **CPU with a spinner title.** With recording on and `titles = true`, put a terminal in
   front whose title changes several times a second (a working coding agent's spinner).
   Over 60 s, Activity Monitor shows Flick near 0% CPU, and `activity_spans` gets no row
   per spinner glyph (the title is the same after the glyph is stripped). Repeat with an idle
   desktop: near 0% CPU.
8. **Idle.** Leave the machine alone for 2+ min. The open span ends about 60 s after the
   last input, `flick activity today` shows no time for the idle stretch, and the first
   input opens a new span.
9. **Lock.** Lock the screen (ctrl+cmd+Q). The open span ends at once: its `end` does not
   move while locked, and `flick activity status` shows `open span: screen locked or
   asleep`. After unlock a new span opens for the front app.
10. **Sleep.** Sleep the Mac (Apple menu > Sleep) for 1+ min and wake it. The span before
    sleep ends at the sleep time; after wake (and unlock), a new span opens.
11. **Excluded app.** Bring 1Password to the front for 30 s. No row names it, and
    `flick activity status` shows `open span: excluded app in front`.
12. **Quit closes the span.** With recording on, note the open row's id, then run **Quit
    Flick**. The row's `end` is the quit time (not the time of the last app switch). Start
    Flick again, and repeat with `kill <pid>` (SIGTERM): same result. Recording is still on
    after the restart, and the `●` shows again.
13. **Stop.** Choose **Stop Recording** from the `●` menu. The indicator hides, the open
    span ends, and `flick activity status` prints `recording: off`.
14. **Forget.** `flick activity forget today` without `--yes` deletes nothing.
    `flick activity forget today --yes` deletes today's rows, and `forget all --yes` empties
    both tables and turns recording off.
