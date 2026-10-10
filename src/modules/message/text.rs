//! Pure text helpers: markdown-ish bodies to plain text, one-line previews, clock labels and
//! the `post` arguments.

/// The longest body, in bytes, a post may carry.
pub const MAX_BODY: usize = 16 * 1024;
/// The card shows at most this many lines and characters of a body.
const CARD_LINES: usize = 18;
const CARD_CHARS: usize = 1500;

/// A body as plain text, the `core::markup` way: markers dropped, headings as plain lines,
/// bullets as `•` (or `n.`), `[text](url)` as `text (url)`, runs of blank lines as one.
pub fn plain(body: &str) -> String {
    crate::core::markup::plain(body)
}

/// What the card shows of `text`: at most `CARD_LINES` lines and `CARD_CHARS` characters,
/// "…" when cut.
pub fn clip(text: &str) -> String {
    let mut out = text.lines().take(CARD_LINES).collect::<Vec<_>>().join("\n");
    let mut cut = text.lines().count() > CARD_LINES;
    if out.chars().count() > CARD_CHARS {
        out = out.chars().take(CARD_CHARS).collect::<String>().trim_end().to_string();
        cut = true;
    }
    if cut {
        out.push('…');
    }
    out
}

/// `text` on one line, at most `max` characters, "…" when cut.
pub fn preview(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// "14:03" for a time on the local day of `now`, else "Oct 9 14:03". `offset` is seconds
/// east of UTC.
pub fn stamp(ts: i64, now: i64, offset: i32) -> String {
    let local = ts + i64::from(offset);
    let day = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let time = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    if day == (now + i64::from(offset)).div_euclid(86_400) {
        return time;
    }
    let (_, month, mday) = civil(day);
    format!("{} {mday} {time}", MONTHS[(month - 1) as usize])
}

/// The local day of `ts` as a transcript divider says it: "Today", "Yesterday", "Oct 9",
/// or "Oct 9 2025" outside the year of `now`. `offset` is seconds east of UTC.
pub fn day(ts: i64, now: i64, offset: i32) -> String {
    let day = (ts + i64::from(offset)).div_euclid(86_400);
    let today = (now + i64::from(offset)).div_euclid(86_400);
    match today - day {
        0 => return "Today".into(),
        1 => return "Yesterday".into(),
        _ => {}
    }
    let (year, month, mday) = civil(day);
    let label = format!("{} {mday}", MONTHS[(month - 1) as usize]);
    if year == civil(today).0 { label } else { format!("{label} {year}") }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Days since 1970-01-01 → (year, month 1-12, day 1-31). Howard Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// What `post` was asked to save.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Post {
    pub title: Option<String>,
    pub url: Option<String>,
    pub reply_to: Option<String>,
    pub id: Option<String>,
    /// The conversation it belongs to.
    pub thread: Option<String>,
    pub pending: bool,
    /// A reply still streaming: the same id is posted again until a post without it.
    pub partial: bool,
    pub body: String,
}

pub const POST_USAGE: &str = "usage: flick message post [--title t] [--url https://…] [--reply-to id] [--id id] [--thread t] [--pending|--partial] [--] <body...>";

/// `post` arguments: flags first, then the body words (joined with spaces); `--` ends the
/// flags.
pub fn parse_post(mut args: &[String]) -> Result<Post, String> {
    let mut post = Post::default();
    loop {
        let (flag, value) = match args {
            [f, ..] if f == "--" => {
                args = &args[1..];
                break;
            }
            [f, rest @ ..] if f == "--pending" || f == "--partial" => {
                if f == "--pending" { post.pending = true } else { post.partial = true }
                args = rest;
                continue;
            }
            [f, v, rest @ ..] if f.starts_with("--") => {
                args = rest;
                (f.as_str(), v.clone())
            }
            [f] if f.starts_with("--") => return Err(format!("{f}: needs a value; {POST_USAGE}")),
            _ => break,
        };
        let value = Some(value).filter(|v| !v.trim().is_empty());
        match flag {
            "--title" => post.title = value,
            "--url" => post.url = value,
            "--reply-to" => post.reply_to = value,
            "--id" => post.id = value,
            "--thread" => post.thread = value,
            _ => return Err(format!("{flag}: unknown flag; {POST_USAGE}")),
        }
    }
    post.body = args.join(" ").trim().to_string();
    if post.body.is_empty() {
        return Err(POST_USAGE.into());
    }
    if post.pending && post.partial {
        return Err("--pending and --partial: a post is one or the other".into());
    }
    if post.body.len() > MAX_BODY {
        return Err(format!("message: body over {MAX_BODY} bytes"));
    }
    if let Some(url) = &post.url
        && !(url.starts_with("https://") || url.starts_with("http://"))
    {
        return Err(format!("--url {url}: only http and https links"));
    }
    for id in [&post.id, &post.reply_to, &post.thread].into_iter().flatten() {
        if !crate::core::card::valid_id(id) {
            return Err(format!("{id}: an id is 1-64 of A-Z a-z 0-9 . _ -"));
        }
    }
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn markdown_becomes_plain_text() {
        let md = "## Plan\r\n\r\n\r\n- **one** `x`\n  * two\n+ [docs](https://a.b/c)\n\n#tag stays\n\n";
        assert_eq!(plain(md), "Plan\n\n• one x\n  • two\n• docs (https://a.b/c)\n\n#tag stays");
        assert_eq!(plain("[a] and [b](u) and [c"), "[a] and b (u) and [c");
        assert_eq!(plain("[x](no close"), "[x](no close");
        assert_eq!(plain("*soft* 1. one\n1. one"), "soft 1. one\n1. one");
        assert_eq!(plain(""), "");
    }

    #[test]
    fn long_bodies_are_clipped() {
        assert_eq!(clip("a\nb"), "a\nb");
        let many = (0..30).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        assert!(clip(&many).ends_with("17…"), "{}", clip(&many));
        let wide = "x".repeat(2000);
        assert_eq!(clip(&wide).chars().count(), CARD_CHARS + 1);
    }

    #[test]
    fn previews_are_one_line() {
        assert_eq!(preview("a\n  b\tc", 10), "a b c");
        assert_eq!(preview("hello world", 6), "hello…");
    }

    #[test]
    fn stamps_show_the_date_off_today() {
        // 2026-10-09 18:31:01 UTC, PDT.
        let ts = 1_791_570_661;
        assert_eq!(stamp(ts, ts, -25_200), "11:31");
        assert_eq!(stamp(ts - 86_400, ts, -25_200), "Oct 8 11:31");
        assert_eq!(stamp(0, ts, 0), "Jan 1 00:00");
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn days_are_named_relative_to_now() {
        // 2026-10-09 18:31:01 UTC, PDT: 11:31 local.
        let ts = 1_791_570_661;
        assert_eq!(day(ts, ts, -25_200), "Today");
        assert_eq!(day(ts - 11 * 3600, ts, -25_200), "Today", "00:31 local");
        assert_eq!(day(ts - 12 * 3600, ts, -25_200), "Yesterday", "23:31 the day before");
        assert_eq!(day(ts - 2 * 86_400, ts, -25_200), "Oct 7");
        assert_eq!(day(ts - 365 * 86_400, ts, -25_200), "Oct 9 2025");
    }

    #[test]
    fn post_reads_flags_then_the_body() {
        let p = parse_post(&words(&[
            "--title", "KOTA", "--url", "https://x.y", "--reply-to", "k1", "--id", "m.2",
            "--pending", "hello", "there",
        ]))
        .unwrap();
        assert_eq!(
            p,
            Post {
                title: Some("KOTA".into()),
                url: Some("https://x.y".into()),
                reply_to: Some("k1".into()),
                id: Some("m.2".into()),
                pending: true,
                body: "hello there".into(),
                ..Post::default()
            }
        );
        let p = parse_post(&words(&["--thread", "t1", "--partial", "--id", "r", "so", "far"])).unwrap();
        assert_eq!((p.thread.as_deref(), p.partial, p.pending, p.body.as_str()), (Some("t1"), true, false, "so far"));
        let p = parse_post(&words(&["--title", " ", "--", "--not-a-flag"])).unwrap();
        assert_eq!((p.title, p.body.as_str()), (None, "--not-a-flag"));
    }

    #[test]
    fn post_refuses_bad_input() {
        let err = |w: &[&str]| parse_post(&words(w)).unwrap_err();
        assert_eq!(err(&[]), POST_USAGE);
        assert_eq!(err(&["--title", "t"]), POST_USAGE);
        assert!(err(&["--title"]).starts_with("--title: needs a value"));
        assert!(err(&["--nope", "x", "body"]).starts_with("--nope: unknown flag"));
        assert!(err(&["--url", "file:///etc", "b"]).contains("only http and https"));
        assert!(err(&["--id", "a b", "b"]).contains("an id is"));
        assert!(err(&["--reply-to", &"x".repeat(65), "b"]).contains("an id is"));
        assert!(err(&["--thread", "a/b", "b"]).contains("an id is"));
        assert!(err(&["--pending", "--partial", "b"]).contains("one or the other"));
        assert!(err(&[&"x".repeat(MAX_BODY + 1)]).contains("over"));
    }
}
