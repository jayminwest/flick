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
    /// `project`: `None` keeps the project, `Some("")` clears it.
    Rename { target: String, title: String, project: Option<String> },
    Reopen { target: String },
    Rm { target: String },
    Report { range: String, project: Option<String> },
}

const USAGES: &[(&str, &str)] = &[
    ("start", "task start <id|title> [--project P]"),
    ("switch", "task switch <id|title> [--project P]"),
    ("stop", "task stop"),
    ("ls", "task ls [--all] [--project P]"),
    ("add", "task add <title> [--project P]"),
    ("done", "task done <id|title>"),
    ("reopen", "task reopen <id|title>"),
    ("rename", "task rename <id|title> <new title> [--project P]"),
    ("rm", "task rm <id|title>"),
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
        "reopen" if !text.is_empty() && project.is_none() => Verb::Reopen { target: text },
        "rm" if !text.is_empty() && project.is_none() => Verb::Rm { target: text },
        "rename" => match words.split_first() {
            Some((target, title)) if !title.is_empty() => {
                Verb::Rename { target: (*target).to_owned(), title: title.join(" "), project }
            }
            _ => return Err(bad()),
        },
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
mod tests;
