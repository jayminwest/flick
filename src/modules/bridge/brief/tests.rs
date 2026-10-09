use super::*;

fn input() -> BriefInput {
    BriefInput {
        now_local: "Fri 9 Oct 2026 14:05".into(),
        allow: vec!["task".into(), "herdr ls".into()],
        context: vec![
            ("task ls".into(), Ok("  {\"ok\":[]}\n".into())),
            ("activity today".into(), Err("activity: remote use not permitted".into())),
        ],
        interactive: false,
    }
}

#[test]
fn states_the_rules_and_the_context() {
    let b = brief(&input());
    for want in [
        "Flick is their local launcher",
        "`flick <module> <verb> --json`, with `--json` last",
        "Never read flick.db or any file under ~/Library/Application Support/Flick",
        "'not permitted', stop and tell the user",
        "`flick activity remote allow`",
        "Never send input to herdr agents",
        "Never run `flick bridge` or `flick activity remote`.",
        "Commands you may run: `flick task`, `flick herdr ls`.",
        "Now: Fri 9 Oct 2026 14:05.",
        "\n## flick task ls --json\n{\"ok\":[]}\n",
        "\n## flick activity today --json\nerror: activity: remote use not permitted\n",
    ] {
        assert!(b.contains(want), "missing {want:?} in\n{b}");
    }
    assert!(!b.contains("flick events"));
}

#[test]
fn interactive_sessions_learn_about_events() {
    let b = brief(&BriefInput { interactive: true, ..input() });
    assert!(b.contains("run `flick events`"));
}

#[test]
fn no_allow_and_no_context() {
    let b = brief(&BriefInput { allow: vec![], context: vec![], ..input() });
    assert!(b.contains("Commands you may run: none."));
    assert!(!b.contains("Context snapshot"));
}

#[test]
fn clips_each_verb_and_the_total() {
    let big = "x".repeat(VERB_LIMIT + 10);
    let context: Vec<(String, Result<String, String>)> =
        (0..6).map(|i| (format!("v{i}"), Ok(big.clone()))).collect();
    let s = snapshot(&context);
    // Four full verbs fill the 16 KB total; the rest are named only.
    assert_eq!(s.matches(&"x".repeat(VERB_LIMIT)).count(), 4);
    assert_eq!(s.matches("(cut; run the command for the rest)").count(), 4);
    assert_eq!(s.matches("(omitted: snapshot limit reached; run the command)").count(), 2);
    assert!(s.contains("## flick v5 --json"));
    assert!(s.len() < TOTAL_LIMIT + 1024);
}

#[test]
fn a_short_last_verb_fits_the_remaining_room() {
    let context: Vec<(String, Result<String, String>)> = vec![
        ("a".into(), Ok("y".repeat(VERB_LIMIT * 3 + 100))),
        ("b".into(), Ok("y".repeat(VERB_LIMIT * 3 + 100))),
        ("c".into(), Ok("y".repeat(VERB_LIMIT * 3 + 100))),
        ("d".into(), Ok("small".into())),
        ("e".into(), Err("z".repeat(VERB_LIMIT + 5))),
    ];
    let s = snapshot(&context);
    assert!(s.contains("## flick d --json\nsmall\n"));
    // Error text is clipped per verb and does not count against the total.
    assert!(s.contains(&format!("error: {}\n", "z".repeat(VERB_LIMIT))));
    assert!(!s.contains(&"z".repeat(VERB_LIMIT + 1)));
}
