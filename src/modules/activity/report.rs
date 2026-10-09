//! Reports over stored spans: local-day ranges, totals by category, project, app, title and
//! domain, and the raw span list. Pure: callers pass the spans, the time and the UTC offset. Each
//! report is `Serialize` for `--json` and has a plain text form.

use serde::Serialize;

use super::rules::Config;
use std::fmt::Write;

use crate::core::track::{Span, Subject, local_day, split_days, sum_by};

const DAY: i64 = 86_400;
/// Titles, and domains, listed in a report.
const TOP_TITLES: usize = 10;
/// Category of spans no rule names.
pub const UNCATEGORIZED: &str = "Uncategorized";

/// Unix time of local midnight starting local day `day`.
pub fn day_start(day: i64, utc_offset_secs: i32) -> i64 {
    day * DAY - i64::from(utc_offset_secs)
}

/// Days since 1970-01-01 of civil date `y-m-d` (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (y, m, d) of day number `z`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "YYYY-MM-DD HH:MM" in local time.
pub fn local_time(ts: i64, utc_offset_secs: i32) -> String {
    let local = ts + i64::from(utc_offset_secs);
    let (y, m, d) = civil_from_days(local.div_euclid(DAY));
    let secs = local.rem_euclid(DAY);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// The start of `since`: "today", "week" (the last 7 local days) or a local date
/// "YYYY-MM-DD".
pub fn parse_since(since: &str, now: i64, utc_offset_secs: i32) -> Option<i64> {
    let today = local_day(now, utc_offset_secs);
    let day = match since {
        "today" => today,
        "week" => today - 6,
        date => {
            let mut parts = date.splitn(3, '-').map(str::parse::<i64>);
            let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) = (parts.next(), parts.next(), parts.next())
            else {
                return None;
            };
            if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                return None;
            }
            days_from_civil(y, m, d)
        }
    };
    Some(day_start(day, utc_offset_secs))
}

/// "2h 05m", "12m", "45s".
pub fn duration(secs: i64) -> String {
    match secs {
        ..60 => format!("{}s", secs.max(0)),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// Time spent on one name, and its share of the recorded time (0 to 1).
#[derive(Debug, PartialEq, Serialize)]
pub struct Total {
    pub name: String,
    pub secs: i64,
    pub share: f64,
}

/// Totals over `from..to`.
#[derive(Debug, Serialize)]
pub struct Report {
    pub range: &'static str,
    pub from: i64,
    pub to: i64,
    pub recording: bool,
    /// Recorded time.
    pub recorded_secs: i64,
    /// Time between the first and last span that was not recorded: idle, away, asleep,
    /// excluded apps, recording off.
    pub gap_secs: i64,
    /// Recorded time per local day ("YYYY-MM-DD"), oldest first.
    pub by_day: Vec<Total>,
    pub by_category: Vec<Total>,
    pub by_project: Vec<Total>,
    pub by_app: Vec<Total>,
    /// Longest titles, only when titles are stored.
    pub top_titles: Vec<Total>,
    /// Longest web hosts, only when URLs are stored.
    pub top_domains: Vec<Total>,
}

/// The host of http(s) URL `url`, lowercased, without user info, port or `www.`.
pub fn domain(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    }
    .to_lowercase();
    let host = host.strip_prefix("www.").map(str::to_owned).unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

/// `spans` cut to `from..to`, empty parts dropped.
pub fn clip(spans: Vec<Span<Subject>>, from: i64, to: i64) -> Vec<Span<Subject>> {
    spans
        .into_iter()
        .map(|s| Span { start: s.start.max(from), end: s.end.min(to), subject: s.subject })
        .filter(|s| s.end > s.start)
        .collect()
}

impl Report {
    /// The report for `range` over `spans` already clipped to `from..to`.
    pub fn new(
        range: &'static str,
        (from, to, utc_offset_secs): (i64, i64, i32),
        spans: &[Span<Subject>],
        rules: &Config,
        recording: bool,
    ) -> Report {
        let recorded: i64 = spans.iter().map(Span::secs).sum();
        let first = spans.iter().map(|s| s.start).min().unwrap_or(0);
        let last = spans.iter().map(|s| s.end).max().unwrap_or(0);
        let totals = |sums: Vec<(String, i64)>| {
            sums.into_iter()
                .map(|(name, secs)| Total { name, secs, share: secs as f64 / recorded as f64 })
                .collect::<Vec<_>>()
        };
        let by = |key: &dyn Fn(&Span<Subject>) -> Option<String>| {
            totals(sum_by(spans.iter().filter(|s| key(s).is_some()), |s| key(s).unwrap_or_default()))
        };
        let mut by_day: Vec<(i64, i64)> = vec![];
        for (day, span) in split_days(spans, utc_offset_secs) {
            match by_day.last_mut() {
                Some((d, secs)) if *d == day => *secs += span.secs(),
                _ => by_day.push((day, span.secs())),
            }
        }
        let by_day = by_day
            .into_iter()
            .map(|(day, secs)| (local_time(day_start(day, utc_offset_secs), utc_offset_secs)[..10].to_owned(), secs))
            .collect();
        let mut top_titles = by(&|s| s.subject.title.clone());
        top_titles.truncate(TOP_TITLES);
        let mut top_domains = by(&|s| s.subject.url.as_deref().and_then(domain));
        top_domains.truncate(TOP_TITLES);
        Report {
            range,
            from,
            to,
            recording,
            recorded_secs: recorded,
            gap_secs: (last - first - recorded).max(0),
            by_day: totals(by_day),
            by_category: by(&|s| {
                Some(rules.classify(&s.subject).0.unwrap_or(UNCATEGORIZED).to_owned())
            }),
            by_project: by(&|s| rules.classify(&s.subject).1.map(str::to_owned)),
            by_app: by(&|s| Some(s.subject.name.clone())),
            top_titles,
            top_domains,
        }
    }

    pub fn text(&self) -> String {
        let state = if self.recording { "on" } else { "off" };
        let mut out = format!(
            "Activity {} (recording {state})\nRecorded {}, gaps {}",
            self.range,
            duration(self.recorded_secs),
            duration(self.gap_secs)
        );
        let days = if self.by_day.len() > 1 { &self.by_day[..] } else { &[] };
        for (head, totals) in [
            ("Days", days),
            ("Categories", &self.by_category[..]),
            ("Projects", &self.by_project[..]),
            ("Apps", &self.by_app[..]),
            ("Titles", &self.top_titles[..]),
            ("Domains", &self.top_domains[..]),
        ] {
            if totals.is_empty() {
                continue;
            }
            let _ = write!(out, "\n{head}");
            for t in totals {
                let share = (t.share * 100.0).round();
                let _ = write!(out, "\n  {:>8}  {share:>3}%  {}", duration(t.secs), t.name);
            }
        }
        out
    }
}

/// Recorded time on one task id; `task: None` is time with no task running.
#[derive(Debug, PartialEq, Serialize)]
pub struct TaskTotal {
    pub task: Option<i64>,
    pub secs: i64,
    pub share: f64,
}

/// `flick activity today|week --by task`: totals keyed by task id. Activity cannot read
/// task titles; `flick task ls` maps ids to titles.
#[derive(Debug, Serialize)]
pub struct TaskReport {
    pub range: &'static str,
    pub from: i64,
    pub to: i64,
    pub recording: bool,
    pub recorded_secs: i64,
    pub by_task: Vec<TaskTotal>,
}

impl TaskReport {
    /// Totals for `range` over `spans` already clipped to `from..to`, largest first.
    pub fn new(range: &'static str, (from, to): (i64, i64), spans: &[Span<Subject>], recording: bool) -> Self {
        let recorded: i64 = spans.iter().map(Span::secs).sum();
        let by_task = sum_by(spans, |s| s.subject.task)
            .into_iter()
            .map(|(task, secs)| TaskTotal { task, secs, share: secs as f64 / recorded as f64 })
            .collect();
        TaskReport { range, from, to, recording, recorded_secs: recorded, by_task }
    }

    pub fn text(&self) -> String {
        let state = if self.recording { "on" } else { "off" };
        let mut out = format!(
            "Activity {} by task (recording {state})\nRecorded {}",
            self.range,
            duration(self.recorded_secs)
        );
        for t in &self.by_task {
            let share = (t.share * 100.0).round();
            let name = t.task.map_or_else(|| "no task".to_owned(), |id| format!("task {id}"));
            let _ = write!(out, "\n  {:>8}  {share:>3}%  {name}", duration(t.secs));
        }
        out
    }
}

/// One span for `flick activity spans`.
#[derive(Debug, Serialize)]
pub struct SpanOut<'a> {
    pub start: i64,
    pub end: i64,
    pub secs: i64,
    pub app: &'a str,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub url: Option<&'a str>,
    pub task: Option<i64>,
    pub category: Option<&'a str>,
    pub project: Option<&'a str>,
}

pub fn span_list<'a>(spans: &'a [Span<Subject>], rules: &'a Config) -> Vec<SpanOut<'a>> {
    spans
        .iter()
        .map(|s| {
            let (category, project) = rules.classify(&s.subject);
            SpanOut {
                start: s.start,
                end: s.end,
                secs: s.secs(),
                app: &s.subject.app,
                name: &s.subject.name,
                title: s.subject.title.as_deref(),
                url: s.subject.url.as_deref(),
                task: s.subject.task,
                category,
                project,
            }
        })
        .collect()
}

/// Tab-separated lines: local start, duration, app name, bundle id, title, and the URL when
/// the span has one.
pub fn span_text(spans: &[SpanOut], utc_offset_secs: i32) -> String {
    spans
        .iter()
        .map(|s| {
            format!(
                "{}\t{}\t{}\t{}\t{}{}",
                local_time(s.start, utc_offset_secs),
                duration(s.secs),
                s.name,
                s.app,
                s.title.unwrap_or(""),
                s.url.map(|u| format!("\t{u}")).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn span(start: i64, end: i64, app: &str, title: Option<&str>) -> Span<Subject> {
        Span { start, end, subject: Subject::new(app, app, title, None) }
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        for day in [-800_000, -1, 0, 59, 60, 20_000, 2_000_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day);
        }
        assert_eq!(local_time(0, 0), "1970-01-01 00:00");
        assert_eq!(local_time(0, -3600), "1969-12-31 23:00");
        assert_eq!(local_time(3 * 3600 + 25 * 60, 7200), "1970-01-01 05:25");
    }

    #[test]
    fn since_takes_today_week_and_dates() {
        let now = 10 * DAY + 3600;
        assert_eq!(parse_since("today", now, 0), Some(10 * DAY));
        assert_eq!(parse_since("week", now, 0), Some(4 * DAY));
        // 01:00 UTC is still the previous day at UTC-2.
        assert_eq!(parse_since("today", now, -7200), Some(9 * DAY + 7200));
        assert_eq!(parse_since("1970-01-03", now, 3600), Some(2 * DAY - 3600));
        for bad in ["", "yesterday", "2026-13-01", "2026-01-00", "2026-01", "a-b-c"] {
            assert_eq!(parse_since(bad, now, 0), None, "{bad}");
        }
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(duration(-5), "0s");
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(12 * 60 + 5), "12m");
        assert_eq!(duration(2 * 3600 + 5 * 60), "2h 05m");
    }

    #[test]
    fn clip_cuts_to_the_range() {
        let spans = vec![span(0, 100, "a", None), span(150, 300, "b", None), span(400, 500, "c", None)];
        let cut = clip(spans, 50, 200);
        assert_eq!(cut.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(), [(50, 100), (150, 200)]);
    }

    #[test]
    fn reports_total_by_rule_app_and_title() {
        let rules = Config::parse(
            &parse("[[activity.rules]]\napp = \"zed\"\nproject = \"flick\"\ncategory = \"code\"")
                .unwrap()
                .section("activity")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let spans = [
            span(0, 300, "zed", Some("main.rs")),
            span(300, 400, "mail", None),
            span(500, 600, "zed", Some("main.rs")),
        ];
        let r = Report::new("today", (0, 1000, 0), &spans, &rules, true);
        assert_eq!((r.recorded_secs, r.gap_secs), (500, 100));
        let names = |t: &[Total]| t.iter().map(|t| (t.name.clone(), t.secs)).collect::<Vec<_>>();
        assert_eq!(names(&r.by_category), [("code".into(), 400), (UNCATEGORIZED.into(), 100)]);
        assert_eq!(names(&r.by_project), [("flick".into(), 400)]);
        assert_eq!(names(&r.by_app), [("zed".into(), 400), ("mail".into(), 100)]);
        assert_eq!(names(&r.top_titles), [("main.rs".into(), 400)]);
        assert!(r.top_domains.is_empty(), "no URLs stored, no domains");
        assert!((r.by_app[0].share - 0.8).abs() < 1e-9);
        let text = r.text();
        assert!(text.starts_with("Activity today (recording on)\nRecorded 8m, gaps 1m"), "{text}");
        assert!(text.contains("\nProjects\n        6m   80%  flick"), "{text}");
        let json = serde_json::to_value(&r).unwrap();
        assert!(json["by_app"].is_array() && json["recording"] == true);

        let empty = Report::new("week", (0, 1, 0), &[], &Config::default(), false);
        assert_eq!((empty.recorded_secs, empty.gap_secs), (0, 0));
        assert_eq!(empty.text(), "Activity week (recording off)\nRecorded 0s, gaps 0s");
    }

    #[test]
    fn domains_are_http_hosts() {
        assert_eq!(domain("https://www.GitHub.com/a/b?q#x").as_deref(), Some("github.com"));
        assert_eq!(domain("http://me:pw@localhost:8600/x").as_deref(), Some("localhost"));
        assert_eq!(domain("https://[::1]:80/").as_deref(), Some("::1"));
        assert_eq!(domain("HTTPS://docs.rs").as_deref(), Some("docs.rs"));
        for no in ["chrome://newtab/", "file:///tmp/x", "about:blank", "https:///x", ""] {
            assert_eq!(domain(no), None, "{no}");
        }
    }

    #[test]
    fn reports_total_by_domain() {
        let mut spans = [span(0, 300, "brave", None), span(300, 400, "brave", None), span(400, 450, "brave", None)];
        for (s, url) in spans.iter_mut().zip(["https://github.com/a", "https://www.github.com/b", "chrome://newtab/"]) {
            s.subject = s.subject.clone().with_url(Some(url));
        }
        let rules = Config::default();
        let r = Report::new("today", (0, 1000, 0), &spans, &rules, true);
        assert_eq!(r.top_domains, [Total { name: "github.com".into(), secs: 400, share: 400.0 / 450.0 }]);
        assert!(r.text().ends_with("\nDomains\n        6m   89%  github.com"), "{}", r.text());
        let list = span_list(&spans, &rules);
        assert_eq!(serde_json::to_value(&list).unwrap()[0]["url"], "https://github.com/a");
        assert!(span_text(&list[..1], 0).ends_with("\tbrave\tbrave\t\thttps://github.com/a"));
    }

    #[test]
    fn totals_by_task_id() {
        let mut spans = vec![span(0, 300, "zed", None), span(300, 400, "mail", None), span(400, 500, "zed", None)];
        spans[0].subject.task = Some(3);
        spans[2].subject.task = Some(3);
        let r = TaskReport::new("today", (0, 1000), &spans, true);
        assert_eq!(r.recorded_secs, 500);
        assert_eq!(
            r.by_task.iter().map(|t| (t.task, t.secs)).collect::<Vec<_>>(),
            [(Some(3), 400), (None, 100)]
        );
        assert_eq!(
            r.text(),
            "Activity today by task (recording on)\nRecorded 8m\n        6m   80%  task 3\n        1m   20%  no task"
        );
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!((json["by_task"][0]["task"].clone(), json["by_task"][1]["task"].clone()), (3.into(), serde_json::Value::Null));
        let empty = TaskReport::new("week", (0, 1), &[], false);
        assert_eq!(empty.text(), "Activity week by task (recording off)\nRecorded 0s");
    }

    #[test]
    fn span_lists_carry_rules() {
        let spans = [span(0, 90, "zed", Some("x")), span(90, 100, "mail", None)];
        let rules = Config::default();
        let list = span_list(&spans, &rules);
        assert_eq!(list[0].secs, 90);
        assert_eq!(list[1].category, None);
        assert_eq!(
            span_text(&list, 0),
            "1970-01-01 00:00\t1m\tzed\tzed\tx\n1970-01-01 00:01\t10s\tmail\tmail\t"
        );
        assert_eq!(serde_json::to_value(&list).unwrap()[0]["title"], "x");
    }
}
