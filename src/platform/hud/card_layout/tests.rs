use super::*;
use crate::core::card::{Origin, parse};

fn card(json: &str) -> Card {
    match parse(json, Origin::Local) {
        Ok(p) => p.card,
        Err(e) => panic!("bad test card: {e}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn pending_wins_then_the_module_error_then_the_card_state() {
    let ui = CardUi::default();
    let open = status(State::Open, &ui);
    assert_eq!(
        open,
        Status { symbol: "text.bubble.fill", line: None, spinner: false, enabled: true }
    );

    let pending = CardUi { pending: true, error: Some("boom"), ..ui };
    let s = status(State::Open, &pending);
    assert_eq!(s.line, Some((Tone::Muted, "Sent to KOTA…".into())));
    assert!(s.spinner && !s.enabled);
    assert_eq!(status(State::Pending, &ui), s);

    let failed = CardUi { error: Some("KOTA rejected: no"), ..ui };
    let s = status(State::Done, &failed);
    assert_eq!(s.line, Some((Tone::Error, "KOTA rejected: no".into())));
    assert!(s.enabled && !s.spinner);

    let s = status(State::Error, &ui);
    assert_eq!(s.line, Some((Tone::Error, "KOTA reported an error".into())));
    assert!(s.enabled);
    let s = status(State::Done, &ui);
    assert_eq!(s.line, Some((Tone::Muted, "Done".into())));
    assert_eq!(s.symbol, "checkmark.circle.fill");
    assert!(!s.enabled);
}

#[test]
fn a_note_shows_muted_and_names_what_runs() {
    let ui = CardUi { note: Some("Copied"), ..CardUi::default() };
    let s = status(State::Open, &ui);
    assert_eq!(s.line, Some((Tone::Muted, "Copied".into())));
    assert!(s.enabled && !s.spinner);
    assert_eq!(s.symbol, "text.bubble.fill");
    // A note replaces Done but keeps the state's symbol and enabled flag.
    let s = status(State::Done, &ui);
    assert_eq!(
        (s.line, s.symbol, s.enabled),
        (Some((Tone::Muted, "Copied".into())), "checkmark.circle.fill", false)
    );
    // An error wins over a note.
    let s = status(State::Open, &CardUi { error: Some("boom"), ..ui });
    assert_eq!(s.line, Some((Tone::Error, "boom".into())));
    // While pending the note says what runs.
    let running =
        CardUi { pending: true, note: Some("Running script deploy…"), ..CardUi::default() };
    let s = status(State::Open, &running);
    assert_eq!(s.line, Some((Tone::Muted, "Running script deploy…".into())));
    assert!(s.spinner && !s.enabled);
    // KOTA's own pending state ignores a stale note.
    let s = status(State::Pending, &ui);
    assert_eq!(s.line, Some((Tone::Muted, "Sent to KOTA…".into())));
}

#[test]
fn only_an_enabled_shell_action_on_a_live_card_confirms() {
    let c = card(
        r#"{"id":"c","title":"T","actions":[
            {"id":"run","label":"Run","do":{"shell":"rm -rf ~/tmp/x && echo done"}},
            {"id":"go","label":"Go"},
            {"id":"bad","label":"Bad","do":{"shell":"  "}}]}"#,
    );
    let ui = |confirm| CardUi { confirm, ..CardUi::default() };
    let (a, cmd) = confirming(&c, &ui(Some("run"))).map_or(("", ""), |(a, c)| (a.id.as_str(), c));
    assert_eq!((a, cmd), ("run", "rm -rf ~/tmp/x && echo done"));
    assert!(confirming(&c, &ui(Some("go"))).is_none());
    assert!(confirming(&c, &ui(Some("bad"))).is_none());
    assert!(confirming(&c, &ui(Some("nope"))).is_none());
    assert!(confirming(&c, &ui(None)).is_none());
    let pending = CardUi { pending: true, confirm: Some("run"), ..CardUi::default() };
    assert!(confirming(&c, &pending).is_none());
}

#[test]
fn tooltips_say_why_a_button_is_off_or_that_it_asks() {
    let c = card(
        r#"{"id":"c","title":"T","actions":[
            {"id":"a","label":"A","do":{"open_url":"ftp://x"}},
            {"id":"b","label":"B","do":{"shell":"ls"}},
            {"id":"c","label":"C"}]}"#,
    );
    let tips: Vec<Option<String>> = c.actions.iter().map(tooltip).collect();
    assert_eq!(
        tips,
        [
            Some("open_url: only http and https links".into()),
            Some("Asks before running: ls".into()),
            None
        ]
    );
}

#[test]
fn markdown_is_cleaned_to_plain_text() {
    let md = "\n\n# Deploy\n\nShip **v2** to `prod`?\n\n\n\n- one\n  * two [docs](https://x.y)\n\
              + three [](https://u.v) and [not a link] and [a](b\n#tag stays\n### \n__done__\n\n";
    assert_eq!(
        clean_md(md),
        "Deploy\n\nShip v2 to prod?\n\n• one\n  • two docs\n• three https://u.v and [not a link] \
         and [a](b\n#tag stays\n\ndone"
    );
    assert_eq!(clean_md("[x [y](z)"), "[x y");
    assert_eq!(clean_md("é- not a bullet"), "é- not a bullet");
    assert_eq!(clean_md(""), "");
}

#[test]
fn labels_for_lists_and_progress() {
    assert_eq!(list_row(0, "a", false), "• a");
    assert_eq!(list_row(2, "c", true), "3. c");
    assert_eq!(progress_text(Some(0.404), Some("Uploading")), "Uploading  40%");
    assert_eq!(progress_text(Some(1.0), Some("  ")), "100%");
    assert_eq!(progress_text(None, Some("Indexing")), "Indexing…");
    assert_eq!(progress_text(None, None), "Working…");
}

#[test]
fn the_key_column_fits_the_longest_key_within_limits() {
    assert!(close(key_width(&[20.0, 52.5], 352.0), 56.5));
    assert!(close(key_width(&[20.0], 352.0), 40.0));
    assert!(close(key_width(&[], 352.0), 40.0));
    assert!(close(key_width(&[400.0], 300.0), 120.0));
    assert!(close(key_width(&[45.0], 50.0), 40.0));
}

#[test]
fn choices_pick_their_control_by_kind_and_size() {
    assert_eq!(choice_style(true, 12), ChoiceStyle::Checkboxes);
    assert_eq!(choice_style(false, RADIO_MAX), ChoiceStyle::Radios);
    assert_eq!(choice_style(false, RADIO_MAX + 1), ChoiceStyle::Popup);
    assert!(close(choice_height(ChoiceStyle::Popup, 9), POPUP_H));
    assert!(close(choice_height(ChoiceStyle::Radios, 3), 3.0 * CHOICE_ROW_H));
    assert!(close(choice_height(ChoiceStyle::Checkboxes, 2), 2.0 * CHOICE_ROW_H));
}

#[test]
fn buttons_flow_right_aligned_and_wrap() {
    assert!(close(button_width(20.0, 300.0), BUTTON_MIN_W));
    assert!(close(button_width(100.0, 300.0), 112.0));
    assert!(close(button_width(500.0, 300.0), 300.0));

    let (at, rows) = flow(&[80.0, 100.0], 300.0);
    assert_eq!(rows, 1);
    assert!(close(at[0].0, 300.0 - 188.0) && close(at[1].0, 300.0 - 100.0));
    let (at, rows) = flow(&[150.0, 150.0, 400.0], 300.0);
    assert_eq!(rows, 3);
    let rows_of: Vec<usize> = at.iter().map(|p| p.1).collect();
    assert_eq!(rows_of, [0, 1, 2]);
    assert!(close(at[0].0, 150.0) && close(at[2].0, 0.0));
    assert_eq!(flow(&[], 300.0), (vec![], 0));

    assert!(close(buttons_height(0), 0.0));
    assert!(close(buttons_height(2), 2.0 * BUTTON_H + BUTTON_GAP));
}

#[test]
fn a_redraw_keeps_what_the_user_entered_unless_the_sender_changed_it() {
    let t = |s: &str| Input::Text(s.into());
    let pair = |id: &str, v: Input| (id.to_string(), v);
    let old_initial = vec![pair("note", t("")), pair("env", Input::One(None)), pair("x", t("a"))];
    let old_live = vec![
        pair("note", t("typed")),
        pair("env", Input::One(Some("prod".into()))),
        pair("x", t("b")),
    ];
    let new_initial = vec![
        pair("note", t("")),
        pair("env", Input::One(Some("stg".into()))),
        pair("new", t("n")),
        pair("x", t("a")),
    ];
    assert_eq!(
        carry(Some((&old_initial, &old_live)), new_initial.clone()),
        [
            pair("note", t("typed")),
            pair("env", Input::One(Some("stg".into()))),
            pair("new", t("n")),
            pair("x", t("b")),
        ]
    );
    assert_eq!(carry(None, new_initial.clone()), new_initial);
    assert_eq!(carry(Some((&old_initial, &[])), vec![pair("note", t(""))]), [pair("note", t(""))]);
}
