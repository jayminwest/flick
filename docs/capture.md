# Capture

The `capture` module takes screenshots, opens them in a small annotation editor, and draws on the screen. It replaces Shottr and Presentify. Shots go through macOS's own `screencapture`, so the selection is the native one: a crosshair, Space for a window, Esc to cancel, every display, Retina pixels.

| Launcher item | What it does |
|---|---|
| **Capture Area** | Drag to select; Space picks a window instead; Esc cancels and saves nothing. |
| **Capture Window** | Click a window. With `shadow = true` the PNG keeps the window shadow. |
| **Capture Screen** | The whole display under the mouse. |
| **Capture Area and Annotate** | Capture Area, then the editor opens on the shot. |
| **Draw on Screen** / **Stop Drawing on Screen** | Draw over every app and every display. |
| **Highlight Cursor** / **Stop Highlighting Cursor** | A ring follows the pointer and pulses on a click. |
| **Clear Drawing** | Remove the shapes from the screen. Shows only while shapes are there. |
| **Recent Captures** | Your shots, newest first. ↵ opens one. ⌘K: **Copy Image**, **Annotate**, **Show in Finder**, **Copy Path**, **Move to Trash**. |

Editor and draw keys: `a` arrow, `r` rectangle, `p` pen, `h` highlighter, `t` text, `x` redact (a solid black box), `1` to `5` the colors, ⌘Z undo, ⇧⌘Z redo, Delete clears.

- **Editor.** ↵ writes the annotated PNG at full pixel size, and copies it when `copy = true`. ⌘C writes and copies; ⌘S writes and does not copy. Esc or the close button writes nothing. The app you came from is in front again after the editor closes. **Capture Area and Annotate** writes over the shot; **Annotate** in Recent Captures and `flick capture annotate` write `<name> annotated.png` beside the file, and that copy shows in Recent Captures.
- **Draw on Screen.** Drawing starts with the pen. ↵ stops drawing and keeps the shapes on the screen; clicks then pass through to the apps under them. Esc stops drawing and clears the shapes. With `fade_secs` above 0, each shape fades after that many seconds.

```toml
[capture]
dir = "~/Pictures/Flick"           # created on the first save
name = "Flick {date} at {time}.png"  # also {kind}: area, window, screen, display, rect
copy = true                        # put each shot on the clipboard
save = true                        # false: clipboard only (save and copy cannot both be off)
sound = true                       # the shutter sound
cursor = false                     # include the mouse pointer
shadow = true                      # window shadows in Capture Window
history = 200                      # rows in Recent Captures; trimming never deletes files
colors = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff"]  # keys 1-5; #rrggbb or #rrggbbaa
width = 4.0                        # pen width in points
fade_secs = 0.0                    # draw on screen: 0 keeps shapes until Esc or Clear Drawing
halo_color = "#ffcc00"
halo_radius = 28.0
area_hotkey = "cmd+ctrl+alt+shift+Digit4"  # area, window, screen, annotate, draw and cursor
draw_hotkey = "cmd+ctrl+alt+shift+KeyD"    # hotkeys are all unbound by default
```

```bash
flick capture area                 # starts the selection and answers "Select an area" at once
flick capture area --annotate      # also: window, window --annotate
flick capture screen               # the display under the mouse; prints the file path
flick capture display 2            # display 2, main display first
flick capture rect 0,0,400,300 --no-copy   # x,y,w,h in points from the top left
flick capture screen --out /tmp/shot.png   # --out needs an absolute path
flick --json capture screen | jq .ok.path  # {"path","width","height","copied"}
flick capture ls --limit 5         # <id>\t<path>\t<w>x<h>, newest first; --json: rows
flick capture last                 # the newest file
flick capture draw on              # on|off|toggle|clear
flick capture cursor toggle        # on|off|toggle
flick capture annotate ~/Desktop/shot.png  # opens the editor; prints the copy's path
```

`screen`, `display` and `rect` answer when the file is written. `area` and `window` answer at once; the shot lands in Recent Captures when you finish the selection.

- **Screen Recording.** Flick needs the Screen Recording permission (System Settings → Privacy & Security → Screen & System Audio Recording). Without it, macOS gives back only the wallpaper and Flick's own windows, so Flick does not capture: the first try shows `Allow Screen Recording for Flick in System Settings` and opens the system prompt. Quit and restart Flick after you allow it. macOS 15 and later also asks again from time to time whether Flick may keep recording the screen; that prompt is from macOS, and Flick keeps working when you allow it. An ad hoc signed rebuild can lose the grant, as with Accessibility.
- **Where files go.** `dir`, named from `name`; a taken name gets ` (2)`, ` (3)` and so on. `--out <path>` writes there instead. With `save = false` the shot is a file in `$TMPDIR/flick-capture`, is on the clipboard, and does not show in Recent Captures; Flick does not delete those files, macOS clears the temp folder.
- **What is stored.** Table `capture_shots` in `flick.db` holds each shot's path, kind, size and time, never the image. A row whose file is gone drops out of Recent Captures.
- **Trash.** **Move to Trash** asks first (⌘↵ confirms), moves the file to the macOS Trash (Finder's Put Back restores it) and removes the row. Nothing else in Flick deletes a screenshot.

## Manual tests

This checklist covers the parts that unit tests cannot reach: the Screen
Recording permission, `screencapture` itself, the clipboard, the annotation editor window,
the draw overlay and the cursor halo. Nothing here runs by itself: do each step by hand, on
the installed Flick.app (`scripts/bundle.sh --install`), after a change to
`src/modules/capture/`, `src/platform/capture.rs`, `src/platform/ink/` or
`pasteboard::set_png`.

Agents: do not run these steps. They take screenshots, need the user's hotkeys and change
the user's screen.

To see what Flick knows, use the CLI:

```bash
flick capture ls --limit 5         # <id>\t<path>\t<w>x<h>, newest first
flick --json capture last | jq .ok # {"id","path","kind","width","height","taken"}
osascript -e 'clipboard info'      # «class PNGf» and TIFF after a copy
```

### Setup

- Use the default `[capture]` table (no table), unless a step says otherwise. Run
  **Reload Flick Config** after each config change.
- Bind the hotkeys for the hotkey steps, for example `area_hotkey`, `draw_hotkey` and
  `cursor_hotkey` from the commented example in the default config.
- Note your displays: one Retina display at least; one non-Retina display for steps 11 and
  19; three or more for step 26.

### Checklist

#### Permission

1. **Denied.** Reset the grant: `tccutil reset ScreenCapture <bundle id of Flick.app>`.
   Run **Capture Area**. The footer shows `Allow Screen Recording for Flick in System
   Settings`, the system prompt opens, no crosshair shows, and no file or row is added (the
   wallpaper is never captured). `flick capture screen` answers
   `capture: Allow Screen Recording ...` and exits 1.
2. **Granted.** Allow Flick in System Settings → Privacy & Security → Screen & System Audio
   Recording, then quit and restart Flick. **Capture Area** now shows the crosshair.
3. **Reconfirmation (macOS 15+).** Over the following weeks macOS asks again whether Flick
   may keep recording the screen. Allow: captures keep working without a restart. Record
   the prompt's wording and how often it came in the seed, so [Capture](#capture) stays right.

#### Shots

4. **Capture Area.** From the launcher: the panel hides first, then the native crosshair
   shows (a crosshair cursor, not the arrow). Drag a region. The PNG is in
   `~/Pictures/Flick` (created if it was missing) as `Flick <date> at <time>.png`, and pasting
   into Preview (File → New from Clipboard) shows the same image. The shutter sound plays.
5. **Cancel.** Start **Capture Area** and press Esc. No file, no row, the clipboard does not
   change, and the next capture works (the worker is not stuck: no `A capture is already
   running`).
6. **Space for a window.** Start **Capture Area**, press Space, click a window: the PNG is
   that window.
7. **Capture Window.** Click a window: the PNG has the window shadow. Set `shadow = false`,
   reload, repeat: no shadow. Set `sound = false`: no shutter sound. `cursor = true` with
   **Capture Screen**: the pointer is in the PNG.
8. **Capture Screen and the 150 ms hide wait.** With the launcher open over an app, run
   **Capture Screen** on each display in turn. Each PNG is the display under the mouse and
   none shows the launcher panel, or a half-faded one. If one does, the 150 ms `HIDE_WAIT`
   in `src/modules/capture/mod.rs` is too short on this Mac: note it in the seed.
9. **CLI.** `flick --json capture screen | jq .ok.path` prints a path that exists, and the
   reply has `"copied":true`. `flick capture rect 0,0,400,300 --no-copy` writes a 400×300 pt
   file (800×600 px on Retina) and leaves the clipboard unchanged (copy some text first and
   paste it after). `flick capture screen --out /tmp/fk-shot.png` writes there;
   `--out shot.png` fails with `capture: --out needs an absolute path`.
10. **Async verbs.** `flick capture area` prints `Select an area` at once and the terminal is
    free; after the drag, `flick capture last` prints the new file.
11. **Retina pixels.** `flick --json capture last | jq .ok` on a Retina display shows twice
    the point size; on a non-Retina display, the point size.
12. **Name collision.** Set `name = "fixed"` and take two shots: `fixed.png` and
    `fixed (2).png`.
13. **save = false.** Set `save = false`: a shot goes to the clipboard, a file appears in
    `$TMPDIR/flick-capture`, and Recent Captures does not list it. `save = false` with
    `copy = false` is refused on reload with `[capture]: save and copy are both off ...`.

#### Recent Captures

14. **List.** **Recent Captures** lists newest first with size, folder and age. ↵ opens a
    shot in Preview. Delete a file in Finder and reopen the view: its row is gone.
15. **Actions.** ⌘K on a row: **Copy Image** (paste in Preview), **Show in Finder** (the file
    is selected), **Copy Path** (paste in a terminal). **Move to Trash** asks first; plain ↵
    only shows `Press ⌘↵ to Move to Trash`; ⌘↵ moves the file to the Trash (Put Back
    restores it) and removes the row.

#### Annotation editor

16. **Opens and takes keys.** Run **Capture Area and Annotate**. The editor window opens at
    the image's point size (fitted to the screen if larger), centered, in front, and keys
    reach it at once: press `r` and drag, without clicking the window first. On macOS 14+
    this relies on cooperative activation (`NSApplication::activate`): if keys go to the
    previous app instead, note it in the seed.
17. **Tools and colors.** Draw with each tool (`a`, `r`, `p`, `h`, `t`, `x`) in each color
    (`1` to `5`). The Text tool opens a field at the click; ↵ places the text, Esc drops it.
    ⌘Z undoes, ⇧⌘Z redoes, Delete clears.
18. **Save, copy, cancel.** ↵ writes over the shot and copies it: the file and the pasted
    image both show the shapes, at full pixel size (same width and height as before). Annotate
    a shot again and press ⌘S: written, not copied. Again and press Esc (and again with the
    close button): the file does not change. Each time, the app that was in front before is
    in front again, with its key focus.
19. **Mixed DPI.** Repeat step 16 for a shot from a non-Retina display while a Retina display
    is the main one, and the other way round. The editor shows the image at its point size,
    and the saved file keeps its pixel size.
20. **Annotate a copy.** In Recent Captures, ⌘K **Annotate** on a row, draw, ↵: a new row
    `<name> annotated.png` shows beside the source, which does not change.
    `flick capture annotate ~/Desktop/<file>.png` prints the copy's path and opens the
    editor. A second Annotate while the editor is open brings that editor to the front.

#### Draw on screen and cursor halo

21. **Draw.** Run **Draw on Screen**. The pen draws over every app, the menu bar and the Dock,
    on every display, and in a full-screen space (make a window full screen and switch to
    it). Keys reach the overlay although it does not activate Flick: `r`, `2`, ⌘Z, Delete.
    The app that was in front keeps its title bar active.
22. **Stop and Esc.** ↵ stops drawing and keeps the shapes; clicks now pass through to the
    apps under them, and the app that was in front still has key focus (type into it).
    **Clear Drawing** removes the shapes. Draw again and press Esc: drawing stops and the
    shapes are gone. With drawing off and no shapes, clicks reach every app, including the
    launcher panel opened over the overlay area.
23. **Fade.** `fade_secs = 3`: each shape fades 3 s after you finish it.
24. **Halo.** Run **Highlight Cursor**. A ring follows the pointer on every display and
    pulses on a click; it never blocks a click. Run **Stop Highlighting Cursor**: it is gone.
    `flick capture cursor toggle` and the `cursor_hotkey` do the same.
25. **Displays change.** While shapes are on the screen, plug in or unplug a display (or change
    the arrangement in System Settings). The overlay covers the new layout; displays that
    kept their frame keep their shapes.
26. **Three or more displays.** With 3+ displays, run **Capture Screen** with the mouse on
    each one, and `flick capture display 1`, `2`, `3`. Each PNG is the expected display
    (screencapture's `-D` numbering is assumed to follow `NSScreen` order, main display
    first). If one is wrong, note which order screencapture uses in the seed.

#### Cost and neighbours

27. **Idle CPU.** With capture enabled and nothing on the screen, Flick stays at about 0% CPU
    over 60 s in Activity Monitor. The same with the halo on and the mouse still.
28. **Neighbours.** After the steps above, the launcher, clipboard history (a copied image
    adds no text clip), the window switcher and activity recording work as before.
29. **Off.** `[capture] enabled = false` and a reload: the capture items and hotkeys are
    gone, and `flick capture ls` fails.
