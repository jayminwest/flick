//! Fixtures under `../fixtures/` are synthetic (see its README): shaped from `claude --help`
//! and pi's `docs/json.md`, not captured from a real run.

use super::*;
use std::path::PathBuf;

const CLAUDE: &str = include_str!("../fixtures/claude_result.json");
const CLAUDE_ERR: &str = include_str!("../fixtures/claude_error.json");
const PI: &str = include_str!("../fixtures/pi_events.jsonl");
const PI_ERR: &str = include_str!("../fixtures/pi_error.jsonl");

fn launch(kind: Kind) -> Launch {
    Launch {
        kind,
        exe: kind.as_str().into(),
        model: String::new(),
        args: vec![],
        max_usd: 0.0,
        allow: vec!["task".into(), "herdr ls".into()],
        cwd: "/tmp/bridge".into(),
    }
}

fn words(s: &str) -> Vec<String> {
    s.split(' ').map(String::from).collect()
}

const CLAUDE_RULES: [&str; 5] = [
    "--allowedTools",
    "Bash(flick task:*)",
    "Bash(flick --json task:*)",
    "Bash(flick herdr ls:*)",
    "Bash(flick --json herdr ls:*)",
];
const CLAUDE_DENY: [&str; 5] = [
    "--disallowedTools",
    "Bash(flick bridge:*)",
    "Bash(flick --json bridge:*)",
    "Bash(flick activity remote:*)",
    "Bash(flick --json activity remote:*)",
];

#[test]
fn kind_round_trips() {
    for k in [Kind::Claude, Kind::Pi] {
        assert_eq!(Kind::parse(k.as_str()), Some(k));
    }
    assert_eq!(Kind::parse("codex"), None);
}

#[test]
fn claude_oneshot_argv() {
    let got = oneshot_argv(&launch(Kind::Claude), "BRIEF", "what now?", "S", None);
    let mut want = words("claude -p --output-format json --tools Bash");
    want.extend(CLAUDE_RULES.map(String::from));
    want.extend(CLAUDE_DENY.map(String::from));
    want.extend(words("--append-system-prompt BRIEF --session-id S --"));
    want.push("what now?".into());
    assert_eq!(got, want);
}

#[test]
fn claude_options_and_attachment() {
    let mut l = launch(Kind::Claude);
    l.model = "opus".into();
    l.max_usd = 0.5;
    l.args = vec!["--verbose".into()];
    l.allow = vec![];
    let shot = PathBuf::from("/caps/shot.png");
    let got = oneshot_argv(&l, "B", "see", "S", Some(&shot));
    let mut want = words("claude -p --output-format json --tools Bash,Read");
    want.extend(words("--allowedTools Read(/caps/shot.png)"));
    want.extend(CLAUDE_DENY.map(String::from));
    want.extend(words("--append-system-prompt B --session-id S --model opus"));
    want.extend(words("--max-budget-usd 0.5 --add-dir /caps --verbose --"));
    want.push("see\n\nAttached file: /caps/shot.png".into());
    assert_eq!(got, want);
    // No allow entries and no attachment: the --allowedTools flag is left out.
    let got = oneshot_argv(&l, "B", "q", "S", None);
    assert!(!got.contains(&"--allowedTools".to_string()));
}

#[test]
fn claude_interactive_argv() {
    let mut l = launch(Kind::Claude);
    l.max_usd = 1.0;
    let got = interactive_argv(&l, "B", "", "S", None);
    let mut want = words("claude --tools Bash");
    want.extend(CLAUDE_RULES.map(String::from));
    want.extend(CLAUDE_DENY.map(String::from));
    want.extend(words("--append-system-prompt B --session-id S"));
    // No -p, no output format, no budget (print-only), and no `--` without a prompt.
    assert_eq!(got, want);
    let got = interactive_argv(&l, "B", "hi", "S", None);
    assert_eq!(got[got.len() - 2..], ["--".to_string(), "hi".to_string()]);
}

#[test]
fn pi_argv() {
    let mut l = launch(Kind::Pi);
    l.max_usd = 2.0;
    let got = oneshot_argv(&l, "B", "q", "S", None);
    assert_eq!(
        got,
        words("pi -p --mode json --tools bash --append-system-prompt B --session-id S -- q")
    );
    l.model = "sonnet".into();
    let shot = PathBuf::from("/caps/shot.png");
    let got = interactive_argv(&l, "B", "see", "S", Some(&shot));
    let want = "pi --tools bash,read --append-system-prompt B --session-id S --model sonnet \
                @/caps/shot.png -- see";
    assert_eq!(got, words(want));
}

#[test]
fn resume_argv_per_agent() {
    assert_eq!(resume_argv(Kind::Claude, "/bin/claude", "u"), words("/bin/claude --resume u"));
    assert_eq!(resume_argv(Kind::Pi, "pi", "u"), words("pi --session u"));
}

#[test]
fn allowlist_drops_unsafe_and_denied_entries() {
    let allow: Vec<String> = [
        "  activity   today ",
        "",
        "task) Bash(rm:*",
        "Task",
        "x*",
        "bridge",
        "bridge ask",
        "activity remote allow",
        "activity remote",
        "activity remotely",
        "my_mod sub-verb 2",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(
        allowed_rules(&allow),
        [
            "Bash(flick activity today:*)",
            "Bash(flick --json activity today:*)",
            "Bash(flick activity remotely:*)",
            "Bash(flick --json activity remotely:*)",
            "Bash(flick my_mod sub-verb 2:*)",
            "Bash(flick --json my_mod sub-verb 2:*)",
        ]
    );
}

#[test]
fn shell_and_terminal_wrappers_keep_words_positional() {
    let argv =
        words("claude --").into_iter().chain(["a 'b' \"$c\"".to_string()]).collect::<Vec<_>>();
    let got = shell_wrap("/bin/zsh", &argv);
    assert_eq!(got[..4], ["/bin/zsh", "-lc", "exec \"$@\"", "flick-bridge"]);
    assert_eq!(got[4..], argv[..]);
    let term = words("wezterm start --cwd {cwd} --");
    let got = terminal_argv(&term, "/tmp/x", &argv);
    assert_eq!(got[..5], ["wezterm", "start", "--cwd", "/tmp/x", "--"]);
    assert_eq!(got[5..], argv[..]);
    assert_eq!(terminal_argv(&[], "/c", &argv), argv);
}

#[test]
fn session_ids_are_uuid_v4() {
    assert_eq!(session_id([0; 16]), "00000000-0000-4000-8000-000000000000");
    assert_eq!(session_id([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
    let id = session_id(*b"0123456789abcdef");
    assert_eq!(id, "30313233-3435-4637-b839-616263646566");
}

#[test]
fn parses_a_claude_result() {
    let a = parse_claude(CLAUDE);
    assert!(a.text.starts_with("You are working on \"Bridge pure core\""));
    assert_eq!(a.session.as_deref(), Some("3f2a9c1e-7b4d-4e8a-9c21-5d6e7f809a1b"));
    assert_eq!(a.cost_usd, Some(0.0421));
    assert_eq!(a.error, None);
}

#[test]
fn parses_a_claude_error() {
    let a = parse_claude(CLAUDE_ERR);
    assert_eq!(a.text, "");
    assert_eq!(a.error.as_deref(), Some("claude: error_max_budget_usd"));
    assert_eq!(a.session.as_deref(), Some("8c7d6e5f-4a3b-4c2d-9e1f-0a1b2c3d4e5f"));
    // An error with text keeps the text as the error.
    let a = parse_claude(r#"{"type":"result","is_error":true,"result":"Not logged in"}"#);
    assert_eq!((a.text.as_str(), a.error.as_deref()), ("Not logged in", Some("Not logged in")));
}

#[test]
fn claude_verbose_and_stream_shapes() {
    let array = format!(r#"[{{"type":"system"}},{},{{"type":"other"}}]"#, CLAUDE.trim());
    assert_eq!(parse_claude(&array), parse_claude(CLAUDE));
    let lines = format!("{{\"type\":\"system\"}}\nnot json\n{}\n", CLAUDE.trim());
    assert_eq!(parse_claude(&lines), parse_claude(CLAUDE));
}

#[test]
fn claude_falls_back_to_raw_output() {
    let a = parse_claude("  Error: claude not found\n");
    assert_eq!(a, Answer { text: "Error: claude not found".into(), ..Answer::default() });
    // JSON that is not a result object is raw text too.
    assert_eq!(parse_claude(r#"{"type":"system"}"#).text, r#"{"type":"system"}"#);
    assert_eq!(parse_claude("").error.as_deref(), Some("no output"));
    let long = "é".repeat(RAW_LIMIT);
    let a = parse_claude(&long);
    assert_eq!(a.text.len(), RAW_LIMIT);
    assert!(long.starts_with(&a.text));
}

#[test]
fn parses_pi_events() {
    let a = parse_pi(PI);
    assert_eq!(a.text, "You have no running task.\nStart one with `flick task start`.");
    assert_eq!(a.session.as_deref(), Some("a1b2c3d4-e5f6-4789-8abc-def012345678"));
    let cost = a.cost_usd.unwrap();
    assert!((cost - 0.007).abs() < 1e-9, "{cost}");
    assert_eq!(a.error, None);
}

#[test]
fn parses_pi_errors() {
    let a = parse_pi(PI_ERR);
    assert_eq!(a.error.as_deref(), Some("401 invalid x-api-key"));
    assert_eq!((a.text.as_str(), a.cost_usd), ("", Some(0.0)));
    let aborted = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"partial"}],"stopReason":"aborted"}}"#;
    let a = parse_pi(aborted);
    assert_eq!((a.text.as_str(), a.error.as_deref()), ("partial", Some("pi: request aborted")));
    assert_eq!((a.session, a.cost_usd), (None, None));
}

#[test]
fn pi_without_an_answer() {
    let header = PI.lines().next().unwrap();
    let a = parse_pi(header);
    assert_eq!(a.error.as_deref(), Some("pi: no assistant message in the output"));
    assert!(a.session.is_some());
    assert_eq!(parse_pi("pi: unknown option\n").text, "pi: unknown option");
    assert_eq!(parse_pi("\n").error.as_deref(), Some("no output"));
    // A message with no content array reads as no text.
    let a =
        parse_pi(r#"{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}"#);
    assert_eq!((a.text.as_str(), a.error), ("", None));
}

#[test]
fn clip_respects_char_boundaries() {
    assert_eq!(clip("abc", 5), "abc");
    assert_eq!(clip("abc", 2), "ab");
    assert_eq!(clip("aé", 2), "a");
}
