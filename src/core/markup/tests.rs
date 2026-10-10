use super::*;

const PLAIN: Style = Style { bold: false, italic: false, code: false };
const BOLD: Style = Style { bold: true, ..PLAIN };
const CODE: Style = Style { code: true, ..PLAIN };

fn span(text: &str, style: Style, link: Option<&str>) -> Span {
    Span { text: text.into(), style, link: link.map(Into::into) }
}

/// The spans of a one-paragraph `s`.
fn line(s: &str) -> Vec<Span> {
    let blocks = parse(s);
    assert_eq!(blocks.len(), 1, "{s:?}: {blocks:?}");
    assert_eq!(blocks[0].kind, Kind::Paragraph);
    blocks[0].spans.clone()
}

/// Spans as `(text, b/i/c flags, link)` for compact asserts.
fn flat(s: &str) -> Vec<(String, String, Option<String>)> {
    line(s)
        .into_iter()
        .map(|sp| {
            let f = [(sp.style.bold, 'b'), (sp.style.italic, 'i'), (sp.style.code, 'c')];
            let flags = f.iter().filter(|(on, _)| *on).map(|(_, ch)| *ch).collect();
            (sp.text, flags, sp.link)
        })
        .collect()
}

fn t(text: &str, flags: &str) -> (String, String, Option<String>) {
    (text.into(), flags.into(), None)
}

fn l(text: &str, flags: &str, url: &str) -> (String, String, Option<String>) {
    (text.into(), flags.into(), Some(url.into()))
}

#[test]
fn blocks_by_kind() {
    let md = "# One\n## Two\n#### Four\n####### seven\n#tag\n\n- a\n  * b\n    + c\n12. d\n\n\
              ```rust\nfn x() {}\n\n  y\n```\nafter";
    let kinds: Vec<(Kind, String, bool)> =
        parse(md).into_iter().map(|b| (b.kind.clone(), b.text(), b.gap)).collect();
    let bullet = |depth, number| Kind::Bullet { depth, number };
    assert_eq!(
        kinds,
        vec![
            (Kind::Heading(1), "One".into(), false),
            (Kind::Heading(2), "Two".into(), false),
            (Kind::Heading(3), "Four".into(), false),
            (Kind::Paragraph, "####### seven\n#tag".into(), false),
            (bullet(0, None), "a".into(), true),
            (bullet(1, None), "b".into(), false),
            (bullet(2, None), "c".into(), false),
            (bullet(0, Some(12)), "d".into(), false),
            (Kind::Code { lang: "rust".into() }, "fn x() {}\n\n  y".into(), true),
            (Kind::Paragraph, "after".into(), false),
        ]
    );
}

#[test]
fn block_edges() {
    // Not bullets: no space after the marker, too many digits, `1)`.
    let p = parse("-x\n1234567890. y\n1) z\n\t- tab");
    assert_eq!(p[0].text(), "-x\n1234567890. y\n1) z");
    assert_eq!(p[1].kind, Kind::Bullet { depth: 2, number: None });
    // An unclosed fence runs to the end; an empty one has no spans; ``` with more ticks
    // in the info string is text.
    let p = parse("```\n```\n```py\nopen *x*\n");
    assert!(p[0].spans.is_empty());
    assert_eq!(p[1].spans, vec![span("open *x*", CODE, None)]);
    assert_eq!(parse("```a```")[0].kind, Kind::Paragraph);
    // Long nesting is capped.
    assert_eq!(
        parse(&format!("{}- deep", " ".repeat(40)))[0].kind,
        Kind::Bullet { depth: 8, number: None }
    );
    assert!(parse("").is_empty());
    assert!(parse("\n \n\t\n").is_empty());
}

#[test]
fn paragraph_lines_keep_breaks_and_styles() {
    let p = parse("**a**\nb\n\nc");
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].spans, vec![span("a", BOLD, None), span("\nb", PLAIN, None)]);
    assert_eq!((p[1].text(), p[1].gap), ("c".into(), true));
}

#[test]
fn emphasis() {
    assert_eq!(flat("a **b** c"), vec![t("a ", ""), t("b", "b"), t(" c", "")]);
    assert_eq!(
        flat("__b__ _i_ *i*"),
        vec![t("b", "b"), t(" ", ""), t("i", "i"), t(" ", ""), t("i", "i")]
    );
    assert_eq!(flat("*a **b** c*"), vec![t("a ", "i"), t("b", "bi"), t(" c", "i")]);
    assert_eq!(flat("**a *b* c**"), vec![t("a ", "b"), t("b", "bi"), t(" c", "b")]);
    assert_eq!(flat("***x***"), vec![t("*x", "b"), t("*", "")]);
    assert_eq!(flat("**x***"), vec![t("x", "b"), t("*", "")]);
    // Closers skip code spans and escapes.
    assert_eq!(flat("*a `*` b*"), vec![t("a ", "i"), t("*", "ic"), t(" b", "i")]);
    assert_eq!(flat("*a `` b*"), vec![t("a `` b", "i")]);
    assert_eq!(flat("*a \\* b*"), vec![t("a * b", "i")]);
}

#[test]
fn unclosed_or_misplaced_markers_stay_literal() {
    for s in [
        "**open",
        "2 * 3 * 4",
        "a * b*",
        "snake_case_name",
        "_a_b",
        "**",
        "*",
        "_x",
        "x *",
        "a ``b` c",
        "`",
        "[a](b",
        "[a]",
        "[a] (b)",
        "[x](a b)",
        "[x]()",
        "\\",
        "\\a",
        "http://",
        "xhttps://a.b",
        "hello",
        "*a\\",
    ] {
        assert_eq!(flat(s), vec![t(s, "")], "{s:?}");
    }
}

#[test]
fn code_spans_and_escapes() {
    assert_eq!(flat("`a*b*`"), vec![t("a*b*", "c")]);
    assert_eq!(flat("``a`b``"), vec![t("a`b", "c")]);
    assert_eq!(flat("\\*not\\* \\[x]"), vec![t("*not* [x]", "")]);
}

#[test]
fn links() {
    assert_eq!(
        flat("see [docs](https://a.b/c)."),
        vec![t("see ", ""), l("docs", "", "https://a.b/c"), t(".", "")]
    );
    assert_eq!(flat("[](https://u.v)"), vec![l("https://u.v", "", "https://u.v")]);
    assert_eq!(
        flat("[**b** `c` [no](x)](u)"),
        vec![
            t("[", ""),
            t("b", "b"),
            t(" ", ""),
            t("c", "c"),
            t(" ", ""),
            l("no", "", "x"),
            t("](u)", "")
        ]
    );
    assert_eq!(
        flat("[**b** `c` https://x.y](u)"),
        vec![l("b", "b", "u"), l(" ", "", "u"), l("c", "c", "u"), l(" https://x.y", "", "u")]
    );
    assert_eq!(flat("[x [y](z)"), vec![t("[x ", ""), l("y", "", "z")]);
}

#[test]
fn bare_urls() {
    let u = "https://a.b/c?d=1";
    assert_eq!(flat(&format!("go {u}.")), vec![t("go ", ""), l(u, "", u), t(".", "")]);
    assert_eq!(
        flat("(http://x.y)"),
        vec![t("(", ""), l("http://x.y", "", "http://x.y"), t(")", "")]
    );
    assert_eq!(flat("http://w.org/A_(b)"), vec![l("http://w.org/A_(b)", "", "http://w.org/A_(b)")]);
    assert_eq!(flat("**https://x.y**"), vec![l("https://x.y", "b", "https://x.y")]);
    assert_eq!(
        flat("<https://x.y>"),
        vec![t("<", ""), l("https://x.y", "", "https://x.y"), t(">", "")]
    );
    assert_eq!(flat("é https://x"), vec![t("é ", ""), l("https://x", "", "https://x")]);
}

#[test]
fn plain_text() {
    let md = "\n\n# Deploy\n\nShip **v2** to `prod`?\n\n\n\n- one\n  * two [docs](https://x.y)\n\
              + three [](https://u.v) and [not a link] and [a](b\n#tag stays\n### \n__done__\n\n";
    assert_eq!(
        plain(md),
        "Deploy\n\nShip v2 to prod?\n\n• one\n  • two docs (https://x.y)\n• three https://u.v and \
         [not a link] and [a](b\n#tag stays\n\ndone"
    );
    assert_eq!(plain("[x [y](z)"), "[x y (z)");
    assert_eq!(plain("[**a** b](u) c [u](u)"), "a b (u) c u");
    assert_eq!(plain("see https://a.b/c."), "see https://a.b/c.");
    assert_eq!(plain("é- not a bullet"), "é- not a bullet");
    assert_eq!(plain(""), "");
    assert_eq!(
        plain("1. a\n2. *b*\r\n\r\n```sh\n\nls\n\n\npwd\n```\n\n\n"),
        "1. a\n2. b\n\nls\n\n\npwd"
    );
    assert_eq!(plain("```\n\nx\n```"), "x");
    assert_eq!(plain("#\n\n#\n\nx"), "x");
    assert_eq!(plain("x\n#\n```\n```"), "x");
}

#[test]
fn never_panics_on_odd_input() {
    let nasty = [
        "*_`[\\",
        "é*é_é`é[é](é)é\\é",
        "**é**é__é__",
        "[é](é",
        "`é",
        "\\é",
        "https://é.é/é",
        "_é_",
        "*\u{0}*",
    ];
    for s in nasty {
        let _ = parse(s);
        let _ = plain(s);
    }
    let deep = "*a **b _c __d ".repeat(600) + &"e*".repeat(200);
    assert!(!parse(&deep).is_empty());
    let ticks = "`".repeat(5000);
    assert_eq!(plain(&ticks), ticks);
}
