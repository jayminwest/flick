# Activity

The `activity` module records which app is in front, as time spans, and reports where your day went. It replaces trackers such as Rize, and it keeps the data on this Mac.

Recording is off until you turn it on: run **Start Activity Recording** from the launcher, `flick activity on`, or the optional hotkey. While it records, a `●` shows in the menu bar; its menu has **Stop Recording**. The on/off state survives reloads and restarts. **Activity Today** in the launcher shows today's totals per category, project, app and (with `urls = true`) web domain.

```toml
[activity]
titles = false                     # the default; true also stores window titles
urls = false                       # the default; true also stores the browser's front tab URL
remote_titles = false              # agent sessions see window titles only when true
remote_urls = false                # agent sessions see URLs and domains only when true
exclude = ["com.1password.1password", "com.agilebits.onepassword7", "com.apple.keychainaccess"]
hotkey = "cmd+ctrl+alt+shift+KeyR" # toggles recording; unbound by default
merge_secs = 2                     # a span shorter than this is dropped when focus moves on (alt-tab)

[[activity.rules]]                 # report-time grouping; the first match wins
app = "wezterm|zed"                # regex on the bundle id or app name, any case
title = "flick"                    # optional regex on the window title
project = "flick"
category = "code"
```

```bash
flick activity status              # recording, titles, urls, the agents' grant, the open span
flick activity today               # totals by category, project, app, title and domain; `week` for 7 days
flick activity today --by task     # totals per running task id (see `flick task ls`)
flick activity spans --since 2026-10-01   # <start>\t<duration>\t<app>\t<bundle id>\t<title>[\t<url>]
flick --json activity today | jq .ok.by_app
flick activity forget today --yes  # also: forget all --yes, forget app <bundle id> --yes
flick activity remote allow 30     # agents may read reports for 30 min; also: allow always, deny, status
```

Privacy:

- **What.** Each span is a start, an end, the app's bundle id and name and, only with `titles = true`, the focused window's title (spinner glyphs stripped, at most 256 characters). With `titles = false` Flick never reads a title and installs no Accessibility observer. Titles need Accessibility; without it `flick activity status` says `titles: no Accessibility permission` and spans stay per app.
- **URLs.** Only with `urls = true`, a span in Brave, Chrome, Edge or Chromium also stores the front tab's URL (at most 2048 characters), and reports list the top domains. Flick asks the browser through AppleScript when it comes to the front or its window or title changes (the window is followed as for titles, so tab switches need Accessibility too). A private (incognito) window gives no URL. Safari and Arc are not supported: Flick cannot tell their private windows, so it never asks them. Safari's AppleScript dictionary has no private-window property; the only workaround reads a localized Window menu label through System Events, which needs extra permissions, works only while Safari is active and can guess wrong. Arc's dictionary accepts `incognito` when a script makes a window, but no documentation shows it as a property a script can read back, and Arc windows have no `mode`. Each browser asks once for Automation permission; if you deny it, `flick activity status` says `urls: no Automation permission for Brave Browser`, the Activity Today footer names the browser, and its spans have no URL (allow it again in System Settings > Privacy & Security > Automation). URLs keep their query strings, which can hold tokens; `exclude` the browser or leave `urls` off if that matters.
- **Not recorded.** Apps in `exclude` (matched by bundle id or name) leave a gap, not a row. Setting `exclude` replaces the default list, so keep the password managers in it. Idle time (60 s without input; the span ends at the last input), a locked screen and sleep are not recorded either.
- **Agents.** An agent session (`--remote`, or a peer over [remote access](remote.md)) reads `today`, `week` and `spans` only while you allow it: `flick activity remote allow [<minutes>|always]` (60 min without an argument) or **Allow Agents to Read Activity (1 h)** in the launcher. `flick activity remote deny`, or the same item, revokes it. `flick activity status` shows the grant as `remote: off`, `remote: on · 42 min left` or `remote: on · until revoked`; `flick activity remote status` also gives the last read. Agents never turn recording on or off, forget, or change the grant. Their replies leave out titles unless `remote_titles = true`, and URLs and domains unless `remote_urls = true`. The grant guards against accidents, not against a hostile agent: `--remote` is a word the caller sends.
- **Where.** Tables `activity_spans` and `activity_state` in `~/Library/Application Support/Flick/flick.db`. The module has no network code. The only way in from outside is the mode-0600 control socket.
- **Delete.** `flick activity forget today|all|app <bundle id> --yes`. `forget all` empties both tables, turns recording off and runs `VACUUM`, so the rows leave the file. Data stays until you delete it.
- **Rules** apply when a report runs and are never stored, so a rule edit regroups past spans too.

Limits: idle detection runs every 5 s, so it can lag by that much, and a long video with no input counts as idle. A crash loses the time since the last event. Quit (**Quit Flick**, SIGTERM from launchd or `kill`) closes the open span. [docs/activity.md](activity.md) has a manual test checklist.

## Manual tests

This checklist covers the parts that unit tests cannot reach: the menu bar
indicator, Accessibility title changes, browser tab URLs and their Automation prompt, sleep
and lock, and quit. Nothing here runs by
itself: do each step by hand, on the installed app, after a change to `src/modules/activity/`,
`src/core/track.rs`, `platform::axwatch`, `platform::browser`, `platform::status_item` or
`events::on_session`.

To see what is stored, read the database with a second connection:

```bash
db=~/Library/Application\ Support/Flick/flick.db
sqlite3 "$db" 'select id, start, end, end - start, app, title, url from activity_spans order by id desc limit 10'
sqlite3 "$db" 'select * from activity_state'
```

### Checklist

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
6a. **URLs off.** With `urls = false` (the default), switch Brave tabs for a minute: every
   new row has `url` NULL, `flick activity status` prints `urls: off`, and System Settings >
   Privacy & Security > Automation lists no browser under Flick.
6b. **URLs on, Automation prompt.** Set `urls = true` and run **Reload Flick Config**, with
   recording on. Bring Brave to the front: macOS asks once whether Flick may control
   "Brave Browser". The launcher and other apps stay responsive while the prompt is up.
   Allow it. Within about a second the open row's `url` is the front tab's URL. Switch tabs
   (each for 5+ s): each tab gets its own row with its URL; a quick tab flip (under
   `merge_secs`) leaves no row. `flick activity today` ends with a `Domains` section
   (hosts only, `www.` dropped), and `flick --json activity spans | jq '.ok[].url'` lists the
   URLs.
6c. **Private window.** Open a private window in Brave (shift+cmd+N) and load a page. The
   rows for that window have `url` NULL; back in a normal window, URLs return.
6d. **Denied.** In System Settings > Privacy & Security > Automation, turn off Brave
   Browser under Flick, then bring Brave to the front. `flick activity status` prints
   `urls: no Automation permission for Brave Browser`, and new Brave rows have no URL.
   Turn it on again: after the next tab switch, status prints `urls: on`. To see the
   prompt again from scratch: `tccutil reset AppleEvents com.jayminwest.flick`.
6e. **Other browsers and exclude.** Chrome gets URLs the same way (its own prompt).
   Safari and Arc rows never have a URL and cause no prompt. With Brave in `exclude`, no
   Brave row is stored and no prompt appears.
6f. **Agents.** `flick activity remote allow`: `flick activity status` prints
   `remote: on · 60 min left`. Then `flick activity spans --remote --json`:
   every `url` is null and `flick activity today --remote --json` has empty `top_domains`.
   With `remote_urls = true` (and a config reload) both show.
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
