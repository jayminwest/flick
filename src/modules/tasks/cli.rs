//! The `task` verbs, pure: argument parsing, `<id|title>` resolution, local-day ranges and
//! what `ls` and `report` print, as text or (`Serialize`) JSON. Callers pass the tasks, the
//! time rows, the time and the UTC offset.

use std::fmt::Write;

use serde::Serialize;

use super::store::{Status, Task};
use crate::core::track::{Span, local_day, sum_by};
use crate::core::unknown_verb;

const DAY: i64 = 86_400;

/// A parsed `flick task` command.
#[derive(Debug, PartialEq, Eq)]
pub enum Verb {
    /// Also `switch`. A target that matches no task creates it.
    Start { target: String, project: Option<String> },
    Stop,
    Ls { all: bool, project: Option<String> },
    Add { title: String, project: Option<String> },
    Done { target: String },
    Report { range: String, project: Option<String> },
}

const USAGES: &[(&str, &str)] = &[
    ("start", "task start <id|title> [--project P]"),
    ("switch", "task switch <id|title> [--project P]"),
    ("stop", "task stop"),
    ("ls", "task ls [--all] [--project P]"),
    ("add", "task add <title> [--project P]"),
    ("done", "task done <id|title>"),
    ("report", "task report [today|week|<YYYY-MM-DD>[..<YYYY-MM-DD>]] [--project P]"),
];

/// Parse `args` (starting at the verb).
pub fn parse(args: &[String]) -> Result<Verb, String> {
    let Some((verb, rest)) = args.split_first() else { return Err(unknown_verb("task", args)) };
    let Some((_, usage)) = USAGES.iter().find(|(v, _)| v == verb) else {
        return Err(unknown_verb("task", args));
    };
    let bad = || format!("task: usage: {usage}");
    let (mut words, mut project, mut all) = (vec![], None, false);
    let mut rest = rest.iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--project" => project = Some(rest.next().ok_or_else(bad)?.clone()),
            "--all" => all = true,
            _ => words.push(word.as_str()),
        }
    }
    let text = words.join(" ");
    let verb = match verb.as_str() {
        "start" | "switch" if !text.is_empty() => Verb::Start { target: text, project },
        "add" if !text.is_empty() => Verb::Add { title: text, project },
        "ls" if words.is_empty() => return Ok(Verb::Ls { all, project }),
        "report" if words.len() <= 1 => {
            let range = words.first().map_or("today", |w| w).to_owned();
            Verb::Report { range, project }
        }
        "done" if !text.is_empty() && project.is_none() => Verb::Done { target: text },
        "stop" if words.is_empty() && project.is_none() => Verb::Stop,
        _ => return Err(bad()),
    };
    if all { Err(bad()) } else { Ok(verb) }
}

/// The task `target` names among `tasks`, in `project` when given: an integer id, else an
/// exact title (a task not done wins), else a unique case-insensitive prefix of a task not
/// done. `Ok(None)`: nothing matches. Several matches are an error that lists them.
pub fn resolve(target: &str, tasks: &[Task], project: Option<&str>) -> Result<Option<i64>, String> {
    if let Ok(id) = target.parse::<i64>() {
        return match tasks.iter().find(|t| t.id == id) {
            Some(t) => Ok(Some(t.id)),
            None => Err(format!("task: no task {id}")),
        };
    }
    let mut candidates: Vec<&Task> =
        tasks.iter().filter(|t| project.is_none_or(|p| t.project.as_deref() == Some(p))).collect();
    candidates.sort_by_key(|t| t.status == Status::Done);
    let exact: Vec<&Task> = candidates.iter().copied().filter(|t| t.title == target).collect();
    let open = |t: &&Task| t.status != Status::Done;
    let matches: Vec<&Task> = match exact.as_slice() {
        [] => {
            let lower = target.to_lowercase();
            candidates.into_iter().filter(open).filter(|t| t.title.to_lowercase().starts_with(&lower)).collect()
        }
        [first, ..] if open(first) => exact.iter().copied().filter(open).collect(),
        _ => exact,
    };
    match matches.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.id)),
        many => {
            let list: Vec<String> = many.iter().map(|t| format!("{} {}", t.id, label(t))).collect();
            Err(format!("task: \"{target}\" matches several tasks: {}", list.join(", ")))
        }
    }
}

/// "Title #project", or the title alone.
pub fn label(task: &Task) -> String {
    match &task.project {
        Some(p) => format!("{} #{p}", task.title),
        None => task.title.clone(),
    }
}

/// "2h 05m", "12m", "45s".
pub fn duration(secs: i64) -> String {
    match secs {
        ..60 => format!("{}s", secs.max(0)),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

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
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// "YYYY-MM-DD" of day number `z`.
pub fn date(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

fn parse_date(s: &str) -> Option<i64> {
    let mut parts = s.splitn(3, '-').map(str::parse::<i64>);
    let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

/// The local days (first, last, inclusive) of `range`: "today", "week" (Monday to today),
/// "YYYY-MM-DD" or "YYYY-MM-DD..YYYY-MM-DD".
pub fn parse_range(range: &str, now: i64, utc_offset_secs: i32) -> Option<(i64, i64)> {
    let today = local_day(now, utc_offset_secs);
    match range {
        "today" => Some((today, today)),
        "week" => Some((today - (today + 3).rem_euclid(7), today)),
        _ => match range.split_once("..") {
            Some((a, b)) => Some((parse_date(a)?, parse_date(b)?)).filter(|(a, b)| a <= b),
            None => parse_date(range).map(|d| (d, d)),
        },
    }
}

/// `spans` cut to `from..to`, empty parts dropped.
pub fn clip(spans: Vec<Span<i64>>, from: i64, to: i64) -> Vec<Span<i64>> {
    spans
        .into_iter()
        .map(|s| Span { start: s.start.max(from), end: s.end.min(to), subject: s.subject })
        .filter(|s| s.end > s.start)
        .collect()
}

/// One task in `ls`.
#[derive(Debug, Serialize)]
pub struct Row {
    pub id: i64,
    pub title: String,
    pub project: Option<String>,
    pub status: Status,
    pub today_secs: i64,
}

/// The running task in `ls`; `since` is the start of its open time row (none while idle,
/// asleep or locked).
#[derive(Debug, Serialize)]
pub struct Running {
    #[serde(flatten)]
    pub task: Row,
    pub since: Option<i64>,
}

/// What `ls` prints.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub running: Option<Running>,
    pub tasks: Vec<Row>,
}

impl Listing {
    /// `tasks` (running one first) with their time in `today` (spans clipped to today).
    pub fn new(tasks: &[Task], today: &[Span<i64>], running: Option<(&Task, Option<i64>)>) -> Self {
        let row = |t: &Task| Row {
            id: t.id,
            title: t.title.clone(),
            project: t.project.clone(),
            status: t.status,
            today_secs: today.iter().filter(|s| s.subject == t.id).map(Span::secs).sum(),
        };
        let run_id = running.map(|(t, _)| t.id);
        let mut rows: Vec<Row> = tasks.iter().map(row).collect();
        rows.sort_by_key(|r| Some(r.id) != run_id);
        Listing { running: running.map(|(t, since)| Running { task: row(t), since }), tasks: rows }
    }

    pub fn text(&self) -> String {
        let run_id = self.running.as_ref().map(|r| r.task.id);
        let mut out = match &self.running {
            Some(r) => format!("Running: {} ({} today)", r.task.title, duration(r.task.today_secs)),
            None => "No task running".into(),
        };
        for r in &self.tasks {
            let mark = if Some(r.id) == run_id { '▶' } else { ' ' };
            let project = r.project.as_ref().map(|p| format!("  #{p}")).unwrap_or_default();
            let (id, status, secs) = (r.id, r.status.as_str(), duration(r.today_secs));
            let _ = write!(out, "\n{mark} {id:>4}  {status:<5}  {secs:>7}  {}{project}", r.title);
        }
        out
    }
}

/// Local days a report covers.
#[derive(Debug, Serialize)]
pub struct Range {
    pub name: String,
    pub from: i64,
    pub to: i64,
    /// First and last local day, "YYYY-MM-DD".
    pub first: String,
    pub last: String,
}

impl Range {
    pub fn new(name: &str, (first, last): (i64, i64), utc_offset_secs: i32) -> Self {
        Range {
            name: name.to_owned(),
            from: day_start(first, utc_offset_secs),
            to: day_start(last + 1, utc_offset_secs),
            first: date(first),
            last: date(last),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TaskTotal {
    pub id: i64,
    pub title: String,
    pub project: Option<String>,
    pub secs: i64,
}

#[derive(Debug, Serialize)]
pub struct ProjectTotal {
    pub project: Option<String>,
    pub secs: i64,
}

/// What `report` prints.
#[derive(Debug, Serialize)]
pub struct Report {
    pub range: Range,
    pub tasks: Vec<TaskTotal>,
    pub projects: Vec<ProjectTotal>,
    pub total_secs: i64,
}

impl Report {
    /// Totals per task and per project over `spans` (clipped to `range`), only in `project`
    /// when given. `tasks` holds every task, done ones too.
    pub fn new(range: Range, spans: &[Span<i64>], tasks: &[Task], project: Option<&str>) -> Self {
        let find = |id: i64| tasks.iter().find(|t| t.id == id);
        let project_of = |id: i64| find(id).and_then(|t| t.project.clone());
        let spans: Vec<&Span<i64>> = spans
            .iter()
            .filter(|s| project.is_none_or(|p| project_of(s.subject).as_deref() == Some(p)))
            .collect();
        let per_task = sum_by(spans.iter().copied(), |s| s.subject).into_iter().map(|(id, secs)| {
            TaskTotal {
                id,
                title: find(id).map_or_else(|| format!("#{id}"), |t| t.title.clone()),
                project: project_of(id),
                secs,
            }
        });
        let projects = sum_by(spans.iter().copied(), |s| project_of(s.subject))
            .into_iter()
            .map(|(project, secs)| ProjectTotal { project, secs })
            .collect();
        Report {
            range,
            tasks: per_task.collect(),
            projects,
            total_secs: spans.iter().map(|s| s.secs()).sum(),
        }
    }

    pub fn text(&self) -> String {
        let r = &self.range;
        let days = if r.first == r.last { r.first.clone() } else { format!("{}..{}", r.first, r.last) };
        let mut out = format!("Tasks {} ({days}): {}", r.name, duration(self.total_secs));
        for t in &self.tasks {
            let project = t.project.as_ref().map(|p| format!("  #{p}")).unwrap_or_default();
            let _ = write!(out, "\n  {:>7}  {} {}{project}", duration(t.secs), t.id, t.title);
        }
        if !self.projects.is_empty() {
            out.push_str("\nProjects");
        }
        for p in &self.projects {
            let _ = write!(out, "\n  {:>7}  {}", duration(p.secs), p.project.as_deref().unwrap_or("(none)"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &str) -> Vec<String> {
        words.split(' ').map(String::from).collect()
    }

    fn task(id: i64, title: &str, project: Option<&str>, status: Status) -> Task {
        Task { id, title: title.into(), project: project.map(String::from), status }
    }

    fn span(start: i64, end: i64, task: i64) -> Span<i64> {
        Span { start, end, subject: task }
    }

    #[test]
    fn verbs_parse_words_and_flags() {
        let p = |w: &str| parse(&args(w));
        let some = |s: &str| Some(s.to_owned());
        assert_eq!(
            p("start Write plan --project flick"),
            Ok(Verb::Start { target: "Write plan".into(), project: some("flick") })
        );
        assert_eq!(p("switch 3"), Ok(Verb::Start { target: "3".into(), project: None }));
        assert_eq!(p("stop"), Ok(Verb::Stop));
        assert_eq!(p("ls --all --project x"), Ok(Verb::Ls { all: true, project: some("x") }));
        assert_eq!(p("add Mail"), Ok(Verb::Add { title: "Mail".into(), project: None }));
        assert_eq!(p("done Mail"), Ok(Verb::Done { target: "Mail".into() }));
        assert_eq!(p("report"), Ok(Verb::Report { range: "today".into(), project: None }));
        assert_eq!(p("report week --project x"), Ok(Verb::Report { range: "week".into(), project: some("x") }));
    }

    #[test]
    fn bad_verbs_name_their_usage() {
        let p = |w: &str| parse(&args(w)).unwrap_err();
        assert_eq!(p("start"), "task: usage: task start <id|title> [--project P]");
        assert_eq!(p("start a --project"), "task: usage: task start <id|title> [--project P]");
        assert_eq!(p("stop now"), "task: usage: task stop");
        assert_eq!(p("stop --project x"), "task: usage: task stop");
        assert_eq!(p("done a --project x"), "task: usage: task done <id|title>");
        assert_eq!(p("add a --all"), "task: usage: task add <title> [--project P]");
        assert!(p("ls x").starts_with("task: usage: task ls"));
        assert!(p("report a b").starts_with("task: usage: task report"));
        assert_eq!(p("nope"), "task: unknown command \"nope\"");
        assert_eq!(parse(&[]).unwrap_err(), "task: missing command");
    }

    #[test]
    fn targets_resolve_by_id_exact_title_then_unique_prefix() {
        let tasks = [
            task(1, "Write plan", Some("flick"), Status::Doing),
            task(2, "Write plan", Some("kota"), Status::Todo),
            task(3, "Review PR", None, Status::Todo),
            task(4, "Review notes", None, Status::Done),
            task(5, "Mail", None, Status::Done),
            task(6, "Mail", Some("x"), Status::Todo),
        ];
        let r = |target: &str, project: Option<&str>| resolve(target, &tasks, project);
        assert_eq!(r("4", None), Ok(Some(4)));
        assert_eq!(r("9", None), Err("task: no task 9".into()));
        assert_eq!(r("Write plan", Some("kota")), Ok(Some(2)));
        assert_eq!(
            r("Write plan", None),
            Err("task: \"Write plan\" matches several tasks: 1 Write plan #flick, 2 Write plan #kota".into())
        );
        // A done task loses an exact tie, but an exact title still finds it alone.
        assert_eq!(r("Mail", None), Ok(Some(6)));
        assert_eq!(r("Mail", Some("nope")), Ok(None));
        assert_eq!(r("Review notes", None), Ok(Some(4)));
        // Prefixes skip done tasks.
        assert_eq!(r("review", None), Ok(Some(3)));
        assert!(r("w", None).unwrap_err().contains("several"));
        assert_eq!(r("w", Some("flick")), Ok(Some(1)));
        assert_eq!(r("Nothing", None), Ok(None));
    }

    #[test]
    fn dates_and_ranges_are_local_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for day in [-1, 0, 59, 60, 11_017, 20_000, 2_000_000] {
            assert_eq!(parse_date(&date(day)), Some(day), "{day}");
        }
        assert_eq!(date(-1), "1969-12-31");
        // 1970-01-08 is a Thursday; its week starts Monday 1970-01-05.
        let now = 7 * DAY + 3600;
        assert_eq!(parse_range("today", now, 0), Some((7, 7)));
        assert_eq!(parse_range("week", now, 0), Some((4, 7)));
        assert_eq!(parse_range("week", 4 * DAY, 0), Some((4, 4)));
        // 01:00 UTC is still the previous day at UTC-2.
        assert_eq!(parse_range("today", now, -7200), Some((6, 6)));
        assert_eq!(parse_range("1970-01-03", now, 0), Some((2, 2)));
        assert_eq!(parse_range("1970-01-03..1970-01-05", now, 0), Some((2, 4)));
        for bad in ["", "yesterday", "2026-13-01", "2026-01-00", "2026-01", "a..b", "1970-01-05..1970-01-03"] {
            assert_eq!(parse_range(bad, now, 0), None, "{bad}");
        }
        let range = Range::new("week", (4, 7), 3600);
        assert_eq!((range.from, range.to, range.first.as_str()), (4 * DAY - 3600, 8 * DAY - 3600, "1970-01-05"));
    }

    #[test]
    fn durations_read_short_and_clip_cuts() {
        assert_eq!(duration(-5), "0s");
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(12 * 60 + 5), "12m");
        assert_eq!(duration(2 * 3600 + 5 * 60), "2h 05m");
        let cut = clip(vec![span(0, 100, 1), span(150, 300, 2), span(400, 500, 3)], 50, 200);
        assert_eq!(cut, [span(50, 100, 1), span(150, 200, 2)]);
    }

    #[test]
    fn listings_put_the_running_task_first() {
        let tasks = [task(1, "Mail", None, Status::Todo), task(2, "Write", Some("flick"), Status::Doing)];
        let today = [span(0, 600, 2), span(700, 760, 2), span(800, 830, 1)];
        let l = Listing::new(&tasks, &today, Some((&tasks[1], Some(700))));
        assert_eq!(l.tasks.iter().map(|r| (r.id, r.today_secs)).collect::<Vec<_>>(), [(2, 660), (1, 30)]);
        assert_eq!(
            l.text(),
            "Running: Write (11m today)\n▶    2  doing      11m  Write  #flick\n     1  todo       30s  Mail"
        );
        let json = serde_json::to_value(&l).unwrap();
        assert_eq!(json["running"]["id"], 2);
        assert_eq!(json["running"]["since"], 700);
        assert_eq!(json["tasks"][1]["status"], "todo");
        let idle = Listing::new(&[], &[], None);
        assert_eq!(idle.text(), "No task running");
        assert!(serde_json::to_value(&idle).unwrap()["running"].is_null());
    }

    #[test]
    fn reports_total_per_task_and_project() {
        let tasks = [
            task(1, "Write", Some("flick"), Status::Done),
            task(2, "Mail", None, Status::Todo),
            task(3, "Fix", Some("flick"), Status::Doing),
        ];
        let spans = [span(0, 600, 1), span(600, 900, 2), span(900, 1200, 3), span(1200, 1260, 9)];
        let r = Report::new(Range::new("today", (0, 0), 0), &spans, &tasks, None);
        assert_eq!(r.total_secs, 1260);
        assert_eq!(r.tasks.iter().map(|t| (t.id, t.secs)).collect::<Vec<_>>(), [(1, 600), (2, 300), (3, 300), (9, 60)]);
        assert_eq!(r.tasks[3].title, "#9");
        assert_eq!(
            r.projects.iter().map(|p| (p.project.as_deref(), p.secs)).collect::<Vec<_>>(),
            [(Some("flick"), 900), (None, 360)]
        );
        assert_eq!(
            r.text(),
            "Tasks today (1970-01-01): 21m\n      10m  1 Write  #flick\n       5m  2 Mail\n       5m  3 Fix  #flick\n       1m  9 #9\nProjects\n      15m  flick\n       6m  (none)"
        );
        let only = Report::new(Range::new("week", (0, 2), 0), &spans, &tasks, Some("flick"));
        assert_eq!((only.total_secs, only.tasks.len()), (900, 2));
        assert_eq!(only.text().lines().next(), Some("Tasks week (1970-01-01..1970-01-03): 15m"));
        let json = serde_json::to_value(&only).unwrap();
        assert_eq!((json["total_secs"].as_i64(), json["range"]["last"].as_str()), (Some(900), Some("1970-01-03")));
        let empty = Report::new(Range::new("today", (0, 0), 0), &[], &[], None);
        assert_eq!(empty.text(), "Tasks today (1970-01-01): 0s");
    }
}
