# Script commands

The `script` module runs shell commands from root search. It is the first step toward the script commands on the roadmap: commands live in `config.toml` for now, not in script files with metadata.

```toml
[[script.commands]]
name = "Ask KOTA"
keyword = "k"
shell = "printf %s {query} | ssh -o BatchMode=yes -o ConnectTimeout=5 mbp-server .local/bin/kota-ask"

[[script.commands]]
name = "Sleep Display"
shell = "pmset displaysleepnow"
```

- **Run.** Each command is a root item. Enter runs `shell` with `/bin/sh -c` and closes the launcher. The command runs on a background thread with no input; its output is dropped.
- **Argument.** A command whose `shell` holds `{query}` takes an argument. Type `<keyword> <text>` in root search (`k pick up milk`) and press Enter, or select the command and press Enter or Tab to type the text. Flick replaces each `{query}` with the text in single quotes, so the text is one shell word: quotes, `$`, `;` and backticks in it are not run. The text is trimmed and may be at most 4000 bytes.
- **Failure.** A command that exits non-zero, or cannot start, posts a notification with the last line of its stderr, and the Flick log has the same line. Notifications need permission (herdr asks on first launch).
- **Rules.** Names must be unique: `script:<name>` is the item id and keeps its usage history. Keywords must be one word and unique among commands. A quicklink with the same keyword shows next to the command; give them different keywords. `shell` must not be empty.
- **Environment.** Flick's own, as launchd or the shell that started it set it: a launchd-started Flick has a short `PATH`, so use full paths for tools outside `/usr/bin` and `/bin`.

`flick script run <name> [query]` runs a command by name from this Mac, with the same rules: the query words are joined with spaces, trimmed, quoted as one shell word and capped at 4000 bytes. A command with `{query}` needs a query; one without refuses it. The reply says what ran (`ran <name>: <command line>`) or why nothing ran (unknown name, missing or extra argument, too long) and exits 1. The command starts in the background, so a later failure is the notification above, not the reply. Network peers are refused (`script run: not allowed over the network`): a peer cannot run scripts on this Mac.

## Manual tests

1. Add the `Sleep Display` command above, run **Reload Flick Config**, type `sleep`, press Enter: the launcher closes and the display sleeps.
2. Add `name = "Echo"`, `keyword = "e"`, `shell = "echo {query} >> /tmp/flick-echo"`. Type `e it's $HOME; date` and press Enter. `/tmp/flick-echo` ends with the literal line `it's $HOME; date`.
3. Select **Echo**, press Tab, type `hi`: the row shows the command line `echo 'hi' >> /tmp/flick-echo`. Enter runs it.
4. Set `shell = "echo nope >&2; exit 3"` on a command and run it: a notification `<name> failed` says `nope`.
