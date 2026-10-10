//! Inline markup inside one line: emphasis, code, links, bare URLs and escapes. Scanning is
//! by byte, and text is only ever cut at ASCII marker bytes, so every slice is on a char
//! boundary.

use super::{Span, Style};

/// What a marker at some position turned out to be.
enum Tok<'a> {
    /// An escaped character, shown as is.
    Lit(&'a str),
    Code(&'a str),
    /// Shown text and target.
    Link(&'a str, &'a str),
    /// Bold when `true`, else italic; the text between the markers.
    Emph(bool, &'a str),
    Url(&'a str),
}

/// `s` as spans.
pub(super) fn spans(s: &str) -> Vec<Span> {
    let mut out = Vec::new();
    run(s, Style::default(), None, &mut out);
    out
}

/// Append `text` to `out`, joining it to the last span when that has the same style and link.
pub(super) fn push(out: &mut Vec<Span>, text: &str, style: Style, link: Option<&str>) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.style == style
        && last.link.as_deref() == link
    {
        last.text.push_str(text);
        return;
    }
    out.push(Span { text: text.to_string(), style, link: link.map(str::to_string) });
}

/// Parse `s` in `style` (inside `link` when set) onto `out`.
fn run(s: &str, style: Style, link: Option<&str>, out: &mut Vec<Span>) {
    let (mut i, mut lit) = (0, 0);
    while i < s.len() {
        match token(s, i, link.is_some()) {
            Err(skip) => i += skip,
            Ok((tok, end)) => {
                push(out, &s[lit..i], style, link);
                match tok {
                    Tok::Lit(t) => push(out, t, style, link),
                    Tok::Code(t) => push(out, t, Style { code: true, ..style }, link),
                    Tok::Link(text, url) => run(text, style, Some(url), out),
                    Tok::Emph(true, t) => run(t, Style { bold: true, ..style }, link, out),
                    Tok::Emph(false, t) => run(t, Style { italic: true, ..style }, link, out),
                    Tok::Url(u) => push(out, u, style, Some(u)),
                }
                (i, lit) = (end, end);
            }
        }
    }
    push(out, &s[lit..], style, link);
}

/// The token at byte `i` and the byte after it, or how many bytes to keep as plain text.
fn token(s: &str, i: usize, in_link: bool) -> Result<(Tok<'_>, usize), usize> {
    match s.as_bytes()[i] {
        b'\\' => match s[i + 1..].chars().next() {
            Some(c) if c.is_ascii_punctuation() => Ok((Tok::Lit(&s[i + 1..i + 2]), i + 2)),
            _ => Err(1),
        },
        b'`' => code(s, i).map(|(t, end)| (Tok::Code(t), end)),
        b'[' if !in_link => link(s, i).ok_or(1),
        b'*' | b'_' => emph(s, i),
        b'h' if !in_link => url(s, i).ok_or(1),
        _ => Err(s[i..].chars().next().map_or(1, char::len_utf8)),
    }
}

/// How many bytes equal to `b[i]` start at `i`.
fn run_len(b: &[u8], i: usize) -> usize {
    b[i..].iter().take_while(|&&x| x == b[i]).count()
}

/// The character before byte `i`.
fn before(s: &str, i: usize) -> Option<char> {
    s[..i].chars().next_back()
}

/// A code span opened by the backtick run at `i`: its text and end, or the run's length.
fn code(s: &str, i: usize) -> Result<(&str, usize), usize> {
    let ticks = run_len(s.as_bytes(), i);
    let mut from = i + ticks;
    while let Some(off) = s[from..].find('`') {
        let at = from + off;
        let close = run_len(s.as_bytes(), at);
        if close == ticks {
            return Ok((&s[i + ticks..at], at + close));
        }
        from = at + close;
    }
    Err(ticks)
}

/// `[text](url)` at `i`; an empty text shows the url. No `[` in the text, no space in the url.
fn link(s: &str, i: usize) -> Option<(Tok<'_>, usize)> {
    let after = &s[i + 1..];
    let close = after.find(']')?;
    let text = &after[..close];
    let target = after[close + 1..].strip_prefix('(')?;
    let url = &target[..target.find(')')?];
    if text.contains('[') || url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    let shown = if text.is_empty() { url } else { text };
    Some((Tok::Link(shown, url), i + close + url.len() + 4))
}

/// A bare `http(s)://` URL at `i`, not inside a word, without trailing punctuation.
fn url(s: &str, i: usize) -> Option<(Tok<'_>, usize)> {
    let rest = &s[i..];
    let scheme = ["https://", "http://"].into_iter().find(|p| rest.starts_with(p))?;
    if before(s, i).is_some_and(char::is_alphanumeric) {
        return None;
    }
    let raw = &rest
        [..rest.find(|c: char| c.is_whitespace() || "<>`\"".contains(c)).unwrap_or(rest.len())];
    let u =
        raw.trim_end_matches(|c: char| ".,;:!?'*_".contains(c) || (c == ')' && !raw.contains('(')));
    (u.len() > scheme.len()).then_some((Tok::Url(u), i + u.len()))
}

/// Emphasis opened by the `*` or `_` run at `i` (two markers bold, one italic): the first
/// closer of the same marker that follows non-space, skipping code spans and escapes. `_`
/// neither opens nor closes inside a word.
fn emph(s: &str, i: usize) -> Result<(Tok<'_>, usize), usize> {
    let bytes = s.as_bytes();
    let (mark, open) = (bytes[i], run_len(bytes, i));
    let width = open.min(2);
    let word = |ch: Option<char>| mark == b'_' && ch.is_some_and(char::is_alphanumeric);
    let opens = s[i + width..].chars().next().is_some_and(|ch| !ch.is_whitespace());
    if !opens || word(before(s, i)) {
        return Err(open);
    }
    let mut at = i + width;
    while at < bytes.len() {
        if bytes[at] == b'`' {
            at = code(s, at).map_or_else(|skip| at + skip, |(_, end)| end);
        } else if bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at] == mark {
            let close = run_len(bytes, at);
            let fits = close == width || (width == 2 && close > 2);
            let next = s[at + width..].chars().next();
            if fits && !before(s, at).is_some_and(char::is_whitespace) && !word(next) {
                return Ok((Tok::Emph(width == 2, &s[i + width..at]), at + width));
            }
            at += close;
        } else {
            at += 1;
        }
    }
    Err(open)
}
