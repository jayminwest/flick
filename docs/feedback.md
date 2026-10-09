# Feedback

The `feedback` module keeps notes about Flick while you use it. **Add Feedback…** in the launcher opens a one-field form; ↵ saves. `fb <text>` in root search shows **Save feedback: <text>**; ↵ saves and closes the launcher. **Recent Feedback** lists the last 50 open notes, newest first; ↵ copies one, ⌘K **Mark Resolved** resolves it. From a terminal: `flick feedback add <text>`, `flick feedback ls [--all] [--limit n]` (`--json` for the entries; `--all` adds resolved ones), `flick feedback resolve <ts> [note]` (the entry's `ts` from `ls`; the note can name the issue that tracks it), `flick feedback path`.

Notes go to `feedback.jsonl` in the checkout this Flick was built from. The repo's `.gitignore` lists it, so git never commits it. The file is JSON Lines: one object per line, appended, never rewritten:

```json
{"ts":"2026-10-09T11:31:01-07:00","text":"the switcher is slow","build":"<sha>","app":"Safari","bundle_id":"com.apple.Safari","query":"add fee"}
```

`ts` is local time with its UTC offset, `build` the commit Flick was built from (`-dirty` for uncommitted changes, `dev` for `cargo build`), `app` and `bundle_id` the app in front when the launcher opened (not from the CLI), and `query` the root search text when **Add Feedback…** ran. Fields without a value are left out. Resolving appends a line `{"resolves":"<entry ts>","ts":"...","note":"..."}`; read open notes with `flick feedback ls`, or every line's text with `jq -r '.text // empty' feedback.jsonl`.

```toml
[feedback]
file = "~/notes/flick.jsonl"   # default: feedback.jsonl in the checkout; required for a prebuilt app
keyword = "fb"                 # "<keyword> <text>" saves in one step
hotkey = "cmd+ctrl+alt+shift+KeyF"  # opens a feedback field; unbound by default
```
