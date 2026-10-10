use super::*;

const PLAIN: Font = Font { size: 13.0, bold: false, italic: false, mono: false };

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn texts(runs: &[Run]) -> Vec<&str> {
    runs.iter().map(|r| r.text.as_str()).collect()
}

#[test]
fn only_http_and_https_links_are_kept() {
    assert_eq!(safe_link("https://a.b/c"), Some("https://a.b/c"));
    assert_eq!(safe_link("HTTP://a.b"), Some("HTTP://a.b"));
    assert_eq!(safe_link("file:///etc/passwd"), None);
    assert_eq!(safe_link("javascript://x"), None);
    assert_eq!(safe_link("mailto:a@b.c"), None);
    assert_eq!(safe_link("a.b"), None);
}

#[test]
fn inline_styles_become_fonts_and_links() {
    let r = runs("a **b** *c* `d` [e](https://e.x) [f](ftp://f.x)", 13.0);
    assert_eq!(texts(&r), vec!["a ", "b", " ", "c", " ", "d", " ", "e", " ", "f"]);
    assert_eq!(r[0].font, PLAIN);
    assert!(r[1].font.bold && !r[1].font.italic);
    assert!(r[3].font.italic && !r[3].font.bold);
    assert_eq!(r[5].font, Font { size: 12.0, mono: true, ..PLAIN });
    assert!(r[5].code && !r[0].code);
    assert_eq!(r[7].link.as_deref(), Some("https://e.x"));
    assert_eq!(r[9].link, None);
    assert!(r.iter().all(|x| x.para == Para::default()));
}

#[test]
fn headings_are_bigger_and_bold_and_spaced_after_the_first_block() {
    let r = runs("# One\n## Two `x`\n### Three\npara", 13.0);
    assert_eq!(texts(&r), vec!["One", "\n", "Two ", "x", "\n", "Three", "\n", "para"]);
    assert_eq!(r[0].font, Font { size: 17.0, bold: true, ..PLAIN });
    assert!(close(r[0].para.before, 0.0));
    assert_eq!(r[2].font, Font { size: 15.0, bold: true, ..PLAIN });
    // Code in a heading keeps the heading's size.
    assert_eq!(r[3].font, Font { size: 15.0, bold: true, mono: true, ..PLAIN });
    assert!(close(r[2].para.before, 8.0));
    assert_eq!(r[5].font, Font { size: 14.0, bold: true, ..PLAIN });
    // A plain paragraph right after a heading has no space before it.
    assert_eq!(r[7].font, PLAIN);
    assert!(close(r[7].para.before, 0.0));
    // The separator ends the block before: it takes that block's paragraph.
    assert_eq!(r[6].para, r[5].para);
    assert_eq!(r[1].font, PLAIN);
}

#[test]
fn bullets_get_a_marker_and_hanging_indents() {
    let r = runs("- a\n  - b\n3. c", 10.0);
    assert_eq!(texts(&r), vec!["•\u{a0}", "a", "\n", "•\u{a0}", "b", "\n", "3.\u{a0}", "c"]);
    let top = Para { first: 0.0, rest: 2.0 * 5.5 + 1.0, before: 0.0 };
    assert_eq!(r[0].para, top);
    assert_eq!(r[1].para, top);
    assert_eq!(r[3].para, Para { first: 12.0, rest: 12.0 + 12.0, before: 0.0 });
    assert_eq!(r[6].para, Para { first: 0.0, rest: 3.0 * 5.5 + 1.0, before: 0.0 });
}

#[test]
fn code_blocks_are_monospaced_and_only_their_first_line_is_spaced() {
    let r = runs("text\n\n```rust\nlet a;\nlet b;\n```", 13.0);
    assert_eq!(texts(&r), vec!["text", "\n", "let a;\n", "let b;"]);
    let mono = Font { size: 12.0, mono: true, ..PLAIN };
    assert_eq!(r[2].font, mono);
    assert!(r[2].code && r[3].code);
    assert!(close(r[2].para.before, 8.0));
    assert!(close(r[3].para.before, 0.0));
}

#[test]
fn a_spaced_paragraph_with_a_styled_second_line_splits_cleanly() {
    // The first span ends in the line break: nothing left after the split.
    let r = runs("a\n\nb\n**c**", 13.0);
    assert_eq!(texts(&r), vec!["a", "\n", "b\n", "c"]);
    assert!(close(r[2].para.before, 8.0));
    assert!(close(r[3].para.before, 0.0));
    assert!(r[3].font.bold);
    assert!(runs("", 13.0).is_empty());
}
