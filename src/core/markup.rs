//! Markdown-lite: the small slice of markdown KOTA writes, parsed into blocks of styled spans
//! for the chat transcript and card text blocks (plan flick-3cdd), plus a plain-text form.
//!
//! Blocks, one source line at a time: `#` to `###` headings (`####`..`######` count as level 3),
//! `-` / `*` / `+` / `1.` bullets (two spaces of indent per nesting level), fenced code
//! (```` ```lang ````, running to the end when the fence never closes, so a streaming reply
//! shows its code as code), and paragraphs. A paragraph is consecutive text lines; its line
//! breaks are kept (`\n` inside the spans), and `gap` records a blank line before a block.
//!
//! Inline: `**bold**` / `__bold__`, `*italic*` / `_italic_` (not inside words for `_`),
//! `` `code` `` (any backtick run, closed by a run of the same length), `[text](url)`, bare
//! `http(s)://` URLs, and backslash escapes of ASCII punctuation. A marker that does not close
//! stays in the text as written. Nothing here panics, whatever the input. Links are kept as
//! written: a renderer decides which schemes it opens.

mod inline;
#[cfg(test)]
mod tests;

/// How a span is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

/// A run of text with one style and at most one link target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub link: Option<String>,
}

/// What a block is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    /// Level 1 to 3.
    Heading(u8),
    /// `depth` 0 is top level; `number` is `Some(n)` for an `n.` item.
    Bullet {
        depth: u8,
        number: Option<u32>,
    },
    /// One span with `code` set holds the lines between the fences, unchanged.
    Code {
        lang: String,
    },
}

/// One block: its kind, its spans (never an empty span), and whether a blank line came
/// before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub spans: Vec<Span>,
    pub gap: bool,
}

impl Block {
    /// The block's text without styles.
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// `md` as blocks.
pub fn parse(md: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut gap = false;
    let mut lines = md.lines();
    while let Some(raw) = lines.next() {
        let body = raw.trim();
        if body.is_empty() {
            gap = !out.is_empty();
            continue;
        }
        if let Some(lang) = body.strip_prefix("```").filter(|l| !l.contains('`')) {
            let code: Vec<&str> = lines.by_ref().take_while(|l| !fence_end(l)).collect();
            let mut spans = Vec::new();
            let style = Style { code: true, ..Style::default() };
            inline::push(&mut spans, &code.join("\n"), style, None);
            out.push(Block { kind: Kind::Code { lang: lang.trim().to_string() }, spans, gap });
            gap = false;
            continue;
        }
        let indent: usize = raw.chars().take_while(|c| c.is_whitespace()).map(tab_width).sum();
        let (kind, text) = line_kind(body, indent);
        let spans = inline::spans(text);
        match out.last_mut() {
            Some(last) if !gap && kind == Kind::Paragraph && last.kind == Kind::Paragraph => {
                inline::push(&mut last.spans, "\n", Style::default(), None);
                for s in spans {
                    inline::push(&mut last.spans, &s.text, s.style, s.link.as_deref());
                }
            }
            _ => out.push(Block { kind, spans, gap }),
        }
        gap = false;
    }
    out
}

/// `md` as plain text: markers dropped, bullets as `•` (or `n.`) indented two spaces per
/// level, links as their text, code as written, runs of blank lines as one, and no blank
/// lines at either end.
pub fn plain(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for b in parse(md) {
        if b.gap && lines.last().is_some_and(|l| !l.is_empty()) {
            lines.push(String::new());
        }
        let code = matches!(b.kind, Kind::Code { .. });
        let text = match b.kind {
            Kind::Bullet { depth, number } => {
                let mark = number.map_or_else(|| "•".to_string(), |n| format!("{n}."));
                format!("{}{mark} {}", "  ".repeat(depth.into()), b.text())
            }
            _ => b.text(),
        };
        let text = if code { text.trim_matches('\n') } else { &text };
        for l in text.split('\n') {
            if l.is_empty() && !code && lines.last().is_none_or(String::is_empty) {
                continue;
            }
            lines.push(l.to_string());
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// A closing code fence: only backticks, at least three.
fn fence_end(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("```") && t.trim_start_matches('`').is_empty()
}

/// Columns of leading whitespace: a tab is four.
fn tab_width(c: char) -> usize {
    if c == '\t' { 4 } else { 1 }
}

/// A trimmed, non-empty line's block kind and the text after its marker.
fn line_kind(body: &str, indent: usize) -> (Kind, &str) {
    let hashes = body.bytes().take_while(|&b| b == b'#').count();
    let after = &body[hashes..];
    if (1..=6).contains(&hashes) && (after.is_empty() || after.starts_with(' ')) {
        return (Kind::Heading(hashes.min(3) as u8), after.trim());
    }
    let depth = (indent / 2).min(8) as u8;
    if let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|m| body.strip_prefix(m)) {
        return (Kind::Bullet { depth, number: None }, rest.trim_start());
    }
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && let Some(rest) = body[digits..].strip_prefix(". ")
    {
        let number = body[..digits].parse().ok();
        return (Kind::Bullet { depth, number }, rest.trim_start());
    }
    (Kind::Paragraph, body)
}
