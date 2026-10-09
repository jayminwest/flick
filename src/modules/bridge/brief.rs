//! The system prompt appended to every agent session: what Flick is, how to read its state,
//! what the agent may and may not do, and a snapshot of the context verbs' output.

use super::agent::{DENY, clip};
use std::fmt::Write as _;

/// Most bytes of one context verb's output in the snapshot.
pub const VERB_LIMIT: usize = 4 * 1024;
/// Most bytes of the whole snapshot.
pub const TOTAL_LIMIT: usize = 16 * 1024;

/// What the brief is built from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BriefInput {
    /// Local date and time, as the user would say it (`Fri 9 Oct 2026 14:05`).
    pub now_local: String,
    /// Flick verbs the agent may run (`[bridge] allow`).
    pub allow: Vec<String>,
    /// Each context verb (`task ls`) with its `--json` output or its error text.
    pub context: Vec<(String, Result<String, String>)>,
    /// An interactive session, which may also watch `flick events`.
    pub interactive: bool,
}

/// The system prompt for `input`.
pub fn brief(input: &BriefInput) -> String {
    let mut s = String::from(
        "You are working for the user of this Mac. Flick is their local launcher; it also \
         records their context (tasks, coding agents, app activity) and serves it on the \
         command line.\n\n\
         Rules:\n\
         - Read Flick's state only with `flick <module> <verb> --json`, with `--json` last.\n\
         - Never read flick.db or any file under ~/Library/Application Support/Flick, and \
         never work around Flick's commands.\n\
         - If a command answers 'not permitted', stop and tell the user. They can grant \
         access themselves (for activity: `flick activity remote allow`, or 'Allow Agents \
         to Read Activity' in Flick). Never try to grant it.\n\
         - Never send input to herdr agents or panes; only read their status.\n",
    );
    let deny: Vec<String> = DENY.iter().map(|d| format!("`flick {d}`")).collect();
    let _ = writeln!(s, "- Never run {}.", deny.join(" or "));
    if input.interactive {
        s.push_str(
            "- To watch for changes, run `flick events` (one JSON event per line) and stop \
             it when done.\n",
        );
    }
    let allow: Vec<String> = input.allow.iter().map(|a| format!("`flick {a}`")).collect();
    let allow = if allow.is_empty() { "none".to_string() } else { allow.join(", ") };
    let _ = write!(s, "\nCommands you may run: {allow}.\n\nNow: {}.\n", input.now_local);
    if !input.context.is_empty() {
        s.push_str("\nContext snapshot, fetched just before this session:\n");
        s.push_str(&snapshot(&input.context));
    }
    s
}

/// Each verb's output under a `## flick <verb> --json` heading, cut to `VERB_LIMIT`, and
/// verbs past `TOTAL_LIMIT` named without their output.
pub fn snapshot(context: &[(String, Result<String, String>)]) -> String {
    let mut s = String::new();
    let mut used = 0;
    for (verb, out) in context {
        let _ = writeln!(s, "\n## flick {verb} --json");
        let body = match out {
            Ok(text) => text.trim(),
            Err(e) => {
                let _ = writeln!(s, "error: {}", clip(e.trim(), VERB_LIMIT));
                continue;
            }
        };
        let room = VERB_LIMIT.min(TOTAL_LIMIT - used);
        if room == 0 {
            s.push_str("(omitted: snapshot limit reached; run the command)\n");
            continue;
        }
        let cut = clip(body, room);
        used += cut.len();
        s.push_str(cut);
        s.push('\n');
        if cut.len() < body.len() {
            s.push_str("(cut; run the command for the rest)\n");
        }
    }
    s
}

#[cfg(test)]
mod tests;
