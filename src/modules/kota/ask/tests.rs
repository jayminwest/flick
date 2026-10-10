use super::*;
use crate::modules::kota::testkit::{EXE, HOOKS, set_busy, take_calls};

fn settings(ssh: &str) -> Settings {
    Settings { ssh: ssh.into(), kota_ask: "bin/kota-ask".into(), ..Settings::default() }
}

#[test]
fn ids_are_valid_card_ids_and_distinct() {
    assert_eq!(id_from(0, 0), "k00");
    assert_eq!(id_from(36 * 36 + 35, 37), "k10z1");
    let (a, b) = (new_id(), new_id());
    assert_ne!(a, b);
    assert!(a.starts_with('k') && crate::core::card::valid_id(&a), "{a}");
}

#[test]
fn check_trims_and_refuses_empty_or_long_questions() {
    assert_eq!(check("  hi there \n"), Ok("hi there".into()));
    assert_eq!(check(" \n"), Err("usage: flick kota ask <text>".into()));
    assert_eq!(check(&"é".repeat(MAX_CHARS)).map(|t| t.chars().count()), Ok(MAX_CHARS));
    assert_eq!(check(&"x".repeat(MAX_CHARS + 1)), Err("kota ask: over 2000 characters".into()));
}

#[test]
fn a_sent_ask_posts_the_placeholder_first_and_never_puts_the_text_in_ssh_argv() {
    take_calls();
    let answer = send("k1", "what's on; today?", &settings("ok"), HOOKS);
    assert_eq!(answer, Ok("queued for KOTA (t1)".into()));
    assert_eq!(
        take_calls(),
        [
            format!("run {EXE} message post --pending --id k1 --title KOTA -- what's on; today?"),
            "feed /usr/bin/ssh -o BatchMode=yes -o ConnectTimeout=8 ok bin/kota-ask --id k1 <<what's on; today?".into(),
        ]
    );
}

#[test]
fn a_busy_flick_gets_one_retry() {
    take_calls();
    set_busy(1);
    assert_eq!(send("k1", "hi", &settings("ok"), HOOKS), Ok("queued for KOTA (t1)".into()));
    assert_eq!(take_calls().len(), 3, "pending post, its retry, ssh");
    set_busy(2);
    let answer = send("k2", "hi", &settings("ok"), HOOKS);
    assert_eq!(answer, Ok("queued for KOTA (t1) (no pending card: flick: Flick is busy; try again)".into()));
    set_busy(0);
}

#[test]
fn a_failed_ssh_replaces_the_placeholder_with_the_error() {
    take_calls();
    let why = "ssh: connect to host down port 22: Connection refused";
    assert_eq!(send("k1", "hi", &settings("down"), HOOKS), Err(why.into()));
    let calls = take_calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2], format!("run {EXE} message post --reply-to k1 --title KOTA -- KOTA ask failed: {why}"));
    // Other ways ssh fails, each with a reason.
    for (ssh, why) in [
        ("slow", "ssh: timed out after 15 s"),
        ("silent", "ssh exited 1"),
        ("killed", "ssh was killed"),
        ("x", "ssh: Could not resolve hostname x: nodename nor servname provided"),
    ] {
        assert_eq!(send("k1", "hi", &settings(ssh), HOOKS), Err(why.into()), "{ssh}");
    }
    take_calls();
}

#[test]
fn without_flick_the_ask_still_goes_to_kota() {
    take_calls();
    let answer = send("k1", "flick-down", &settings("ok"), HOOKS);
    assert_eq!(answer, Ok("queued for KOTA (t1) (no pending card: flick: cannot reach Flick: Connection refused)".into()));
    let no_exe = Hooks { exe: || Err("cannot find Flick's binary: gone".into()), ..HOOKS };
    assert_eq!(
        send("k1", "hi", &settings("ok"), no_exe),
        Ok("queued for KOTA (t1) (no pending card: cannot find Flick's binary: gone)".into())
    );
    // No binary and no ssh: nothing to post the error with.
    assert!(send("k1", "hi", &settings("down"), no_exe).is_err());
    assert_eq!(take_calls().len(), 4, "pending, ssh, ssh, ssh");
}

#[test]
fn asks_are_remembered_newest_first_and_settled_by_id() {
    let ask = |id: &str| Ask { id: id.into(), text: "q".into(), at: 1, status: Status::Sending };
    let mut asks = vec![];
    for i in 0..10 {
        remember(&mut asks, ask(&format!("k{i}")));
    }
    assert_eq!(asks.len(), HISTORY);
    assert_eq!((asks[0].id.as_str(), asks[7].id.as_str()), ("k9", "k2"));
    settle(&mut asks, "k9", &Ok("queued".into()));
    settle(&mut asks, "k8", &Err("down".into()));
    settle(&mut asks, "k0", &Ok("forgotten".into()));
    assert_eq!(asks[0].status, Status::Sent("queued".into()));
    assert_eq!(asks[1].status, Status::Failed("down".into()));
}
