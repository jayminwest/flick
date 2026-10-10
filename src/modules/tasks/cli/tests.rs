use super::*;

const DAY: i64 = 86_400;

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
    assert_eq!(p("reopen Mail"), Ok(Verb::Reopen { target: "Mail".into() }));
    assert_eq!(p("rm 3"), Ok(Verb::Rm { target: "3".into() }));
    assert_eq!(
        p("rename 3 Write the plan --project flick"),
        Ok(Verb::Rename { target: "3".into(), title: "Write the plan".into(), project: some("flick") })
    );
    // The shell keeps a quoted title in one word.
    let quoted = ["rename", "Write plan", "Plan"].map(String::from);
    assert_eq!(parse(&quoted), Ok(Verb::Rename { target: "Write plan".into(), title: "Plan".into(), project: None }));
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
    assert_eq!(p("rename 3"), "task: usage: task rename <id|title> <new title> [--project P]");
    assert!(p("rename 3 x --all").starts_with("task: usage: task rename"));
    assert_eq!(p("rm a --project x"), "task: usage: task rm <id|title>");
    assert_eq!(p("reopen"), "task: usage: task reopen <id|title>");
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
