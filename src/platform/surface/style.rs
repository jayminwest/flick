//! The pure side of `render`: markdown-lite (`core::markup`) as a flat list of styled runs,
//! each with its font, paragraph indents and spacing, and a link only for `http(s)` URLs.
//! `render` turns the runs into an `NSAttributedString`. No `AppKit`.

use crate::core::markup::{self, Kind};

/// A run's font, relative to nothing: `size` is in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
}

/// The paragraph a run belongs to: first-line and wrapped-line indents, space before it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Para {
    pub first: f64,
    pub rest: f64,
    pub before: f64,
}

/// A run of text drawn one way.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub font: Font,
    /// Code: a shaded background.
    pub code: bool,
    /// An `http(s)` URL the run opens.
    pub link: Option<String>,
    pub para: Para,
}

/// Indent per bullet level, in body-size units.
const LEVEL_INDENT: f64 = 1.2;
/// Rough width of one marker character, in body-size units, for wrapped bullet lines.
const MARK_CHAR: f64 = 0.55;

/// `url` when it is an `http` or `https` URL; nothing else becomes a link.
pub fn safe_link(url: &str) -> Option<&str> {
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https")).then_some(url)
}

/// A heading's font size for body `size`.
fn heading(level: u8, size: f64) -> f64 {
    size + match level {
        1 => 4.0,
        2 => 2.0,
        _ => 1.0,
    }
}

/// A bullet's marker and its paragraph's indents for body `size`.
fn bullet(depth: u8, number: Option<u32>, size: f64) -> (String, f64, f64) {
    let mark = number.map_or_else(|| "•\u{a0}".to_string(), |n| format!("{n}.\u{a0}"));
    let first = f64::from(depth) * LEVEL_INDENT * size;
    let rest = first + mark.chars().count() as f64 * MARK_CHAR * size + 1.0;
    (mark, first, rest)
}

/// `md` as runs at body `size`. Blocks are separated by a `\n` run that ends the block
/// before (so it takes that block's paragraph); a blank line, a heading or a code block adds
/// space before a block. Code is one point smaller than the text around it, except in a
/// heading.
pub fn runs(md: &str, size: f64) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let plain = Font { size, bold: false, italic: false, mono: false };
    let mut last = Para::default();
    for (i, b) in markup::parse(md).into_iter().enumerate() {
        let spaced = b.gap || matches!(b.kind, Kind::Heading(_) | Kind::Code { .. });
        let before = if i > 0 && spaced { (size * 0.6).round() } else { 0.0 };
        let mut para = Para { before, ..Para::default() };
        let (text_size, code_size, bold) = match b.kind {
            Kind::Heading(level) => (heading(level, size), heading(level, size), true),
            _ => (size, size - 1.0, false),
        };
        if i > 0 {
            out.push(Run { text: "\n".into(), font: plain, code: false, link: None, para: last });
        }
        if let Kind::Bullet { depth, number } = b.kind {
            let (mark, first, rest) = bullet(depth, number, size);
            para = Para { first, rest, before };
            out.push(Run { text: mark, font: plain, code: false, link: None, para });
        }
        for s in b.spans {
            let font = Font {
                size: if s.style.code { code_size } else { text_size },
                bold: bold || s.style.bold,
                italic: s.style.italic,
                mono: s.style.code,
            };
            let link = s.link.as_deref().and_then(safe_link).map(str::to_string);
            let run = |text: &str, para| Run {
                text: text.into(),
                font,
                code: s.style.code,
                link: link.clone(),
                para,
            };
            // Only a block's first line gets the space before it.
            match s.text.find('\n').filter(|_| para.before > 0.0) {
                Some(n) => {
                    let (head, tail) = s.text.split_at(n + 1);
                    out.push(run(head, para));
                    para.before = 0.0;
                    if !tail.is_empty() {
                        out.push(run(tail, para));
                    }
                }
                None => out.push(run(&s.text, para)),
            }
        }
        last = para;
    }
    out
}

#[cfg(test)]
mod tests;
