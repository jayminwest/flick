//! An agent's last reply, cut from the terminal text that `agent.read` returns. herdr has
//! no transcript API, so this reads the screen as a person would (seen with Claude Code
//! and Codex):
//!
//! - The input box at the bottom starts at a full-width rule (`───`) in column 0; it and
//!   everything under it is chrome.
//! - A turn starts after a prompt line in column 0 (`❯ `, `> ` or `› ` and text). A prompt
//!   with no reply after it (Codex's input line) gives way to the one before it.
//! - In a turn, the reply is the last block that starts with a bullet in column 0 (`⏺`,
//!   `●` or `•`): earlier bullets are tool calls. No bullet: the whole turn.
//! - No prompt and no bullet (a shell): the last `fallback` non-empty lines.
//!
//! Pure: no I/O, no clock.

/// Chars per row of the reply block. The panel draws it in a 12 pt fixed-width font in a
/// 710 pt wide field, about 98 chars.
pub const WIDTH: usize = 94;
/// Rows of the reply block that fit under two list rows.
pub const ROWS: usize = 17;

const PROMPTS: [&str; 3] = ["❯ ", "> ", "› "];
const BULLETS: [char; 3] = ['⏺', '●', '•'];
/// Spinner and status lines an agent draws under its reply.
const STATUS: [char; 8] = ['✻', '✢', '✳', '✶', '✽', '✺', '✔', '⏵'];

/// Tabs to spaces, control characters dropped, trailing space trimmed.
fn clean(line: &str) -> String {
    let line: String = line.chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
    line.chars().filter(|c| !c.is_control()).collect::<String>().trim_end().to_string()
}

fn is_rule(line: &str) -> bool {
    line.chars().count() >= 20 && line.chars().all(|c| matches!(c, '─' | '━' | '═'))
}

fn is_prompt(line: &str) -> bool {
    PROMPTS.iter().any(|p| line.strip_prefix(p).is_some_and(|rest| !rest.trim().is_empty()))
}

fn is_status(line: &str) -> bool {
    line.trim_start().starts_with(STATUS)
}

/// The text after a column-0 bullet, or `None`.
fn bullet(line: &str) -> Option<&str> {
    let rest = line.strip_prefix(BULLETS)?;
    Some(rest.trim_start())
}

/// Where the input box starts: the last rule, or the rule above it when the two are at most
/// `BOX` lines apart (the box's top and bottom).
fn chrome_start(lines: &[String]) -> usize {
    const BOX: usize = 12;
    let Some(last) = lines.iter().rposition(|l| is_rule(l)) else { return lines.len() };
    match lines[..last].iter().rposition(|l| is_rule(l)) {
        Some(top) if last - top <= BOX => top,
        _ => last,
    }
}

/// `lines` dedented by the common indent of their non-blank lines.
fn dedent(lines: &[String]) -> Vec<String> {
    let indent = |l: &String| l.len() - l.trim_start_matches(' ').len();
    let common = lines.iter().filter(|l| !l.trim().is_empty()).map(indent).min().unwrap_or(0);
    lines.iter().map(|l| if l.trim().is_empty() { String::new() } else { l[common..].to_string() }).collect()
}

/// `lines` without blank or status lines at the ends and without runs of blank lines,
/// dedented.
fn tidy(lines: &[String]) -> Vec<String> {
    let mut end = lines.len();
    while end > 0 && (lines[end - 1].trim().is_empty() || is_status(&lines[end - 1])) {
        end -= 1;
    }
    let start = lines[..end].iter().position(|l| !l.trim().is_empty()).unwrap_or(end);
    let mut out: Vec<String> = vec![];
    for l in dedent(&lines[start..end]) {
        if !l.is_empty() || out.last().is_some_and(|p| !p.is_empty()) {
            out.push(l);
        }
    }
    out
}

/// The reply in `turn`: from its last bullet, the bullet dropped, or the whole turn.
fn reply_in(turn: &[String]) -> Vec<String> {
    let Some(at) = turn.iter().rposition(|l| bullet(l).is_some()) else { return tidy(turn) };
    let mut reply = vec![bullet(&turn[at]).unwrap_or_default().to_string()];
    let rest = &turn[at + 1..];
    let n = rest.len() - rest.iter().rev().take_while(|l| l.trim().is_empty() || is_status(l)).count();
    reply.extend(dedent(&rest[..n]));
    tidy(&reply)
}

/// The agent's last reply in `text` (escapes already stripped by herdr), as lines joined
/// with `\n`. Empty when there is none.
pub fn last_reply(text: &str, fallback: usize) -> String {
    let lines: Vec<String> = text.lines().map(clean).collect();
    let mut end = chrome_start(&lines);
    let mut prompts: Vec<usize> = (0..end).filter(|&i| is_prompt(&lines[i])).collect();
    // A prompt at the very bottom with at most two indented lines under it (a status bar)
    // is an input line (Codex).
    if let Some(&p) = prompts.last() {
        let under: Vec<&String> = lines[p + 1..end].iter().filter(|l| !l.trim().is_empty()).collect();
        if under.len() <= 2 && under.iter().all(|l| l.starts_with(' ')) {
            end = p;
            prompts.pop();
        }
    }
    for &p in prompts.iter().rev() {
        let reply = reply_in(&lines[p + 1..end]);
        if !reply.is_empty() {
            return reply.join("\n");
        }
        end = p;
    }
    let body = &lines[..end];
    if !prompts.is_empty() || body.iter().any(|l| bullet(l).is_some()) {
        return reply_in(body).join("\n");
    }
    let mut last: Vec<&str> =
        body.iter().rev().map(String::as_str).filter(|l| !l.trim().is_empty()).take(fallback).collect();
    last.reverse();
    last.join("\n")
}


/// `line` wrapped at spaces to rows of at most `width` chars (at least 2), keeping its
/// indent (up to half the width); a word longer than a row is split.
fn wrap(line: &str, width: usize) -> Vec<String> {
    let indent = " ".repeat((line.len() - line.trim_start_matches(' ').len()).min(width / 2));
    let mut rows = vec![];
    let mut row = indent.clone();
    let mut fresh = true;
    for word in line.split(' ').filter(|w| !w.is_empty()) {
        let mut word: Vec<char> = word.chars().collect();
        if !fresh && row.chars().count() + 1 + word.len() > width {
            rows.push(std::mem::replace(&mut row, indent.clone()));
            fresh = true;
        }
        if !fresh {
            row.push(' ');
        }
        while word.len() > width - row.chars().count() {
            let room = width - row.chars().count();
            row.extend(word.drain(..room));
            rows.push(std::mem::replace(&mut row, indent.clone()));
        }
        row.extend(word);
        fresh = false;
    }
    rows.push(row);
    rows
}

/// `reply` wrapped to `width` and cut to its last `rows` rows, with `…` as the first row
/// when cut.
pub fn tail(reply: &str, width: usize, rows: usize) -> String {
    let all: Vec<String> = reply.lines().flat_map(|l| wrap(l, width)).collect();
    if all.len() <= rows {
        return all.join("\n");
    }
    let keep = rows.saturating_sub(1);
    let mut out = vec!["…".to_string()];
    out.extend_from_slice(&all[all.len() - keep..]);
    out.join("\n")
}

#[cfg(test)]
mod tests;
