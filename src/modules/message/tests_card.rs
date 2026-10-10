//! `message card` verbs over the shared fixture: post, replace, `reply_to`, the silent `done`
//! update of a dismissed card, origin, and errors. Nothing reaches `AppKit`.

use super::tests::{Fixture, inbox, set_keyboard, take_log};
use super::*;
use crate::core::card as core_card;

const DEPLOY: &str = r#"{"v":1,"id":"c1","title":"Deploy?","blocks":[{"type":"text","md":"Ready to ship **v2**."}],"actions":[{"id":"go","label":"Ship","style":"primary"}]}"#;

/// `DEPLOY` with `state` set and no actions.
fn deploy_in(state: &str) -> String {
    format!(r#"{{"id":"c1","title":"Deploy?","state":"{state}","blocks":[{{"type":"text","md":"Shipped."}}]}}"#)
}

#[test]
fn a_card_shows_through_the_renderer_and_lands_in_history() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    assert_eq!(f.run(&mut m, false, &["card", "post", DEPLOY]), Ok("c1".into()));
    // Open with an action: sticky, no timeout, no press state.
    assert_eq!(take_log(), ["card c1 Deploy? Open|false|None|None|TopRight|4|0|true|true"]);
    assert_eq!(f.run(&mut m, false, &["ls"]).unwrap(), "c1\t11:31\tDeploy? Ready to ship v2. [Ship]");
    assert_eq!(f.run(&mut m, false, &["card", "ls"]).unwrap(), "c1\t11:31\topen\tDeploy?");
    let stored = f.run(&mut m, false, &["card", "get", "c1"]).unwrap();
    assert_eq!(stored, core_card::to_json(&core_card::parse(DEPLOY, core_card::Origin::Remote).unwrap().card));
    let listed: serde_json::Value = serde_json::from_str(&f.run(&mut m, true, &["card", "ls", "--limit", "5"]).unwrap()).unwrap();
    assert_eq!(listed[0]["id"], "c1");
    assert_eq!(listed[0]["remote"], true);
    assert_eq!(listed[0]["card"]["actions"][0]["label"], "Ship");
    let row = f.store.message("c1").unwrap();
    assert!(row.remote && !row.pending && row.title.is_none());
    assert_eq!(row.body, "Deploy?\nReady to ship **v2**.\n[Ship]");
}

#[test]
fn replies_list_warnings_and_a_repost_replaces() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nstyle = \"both\""));
    let odd = r#"{"id":"c1","title":"T","x":1}"#;
    assert_eq!(f.run(&mut m, false, &["card", "post", odd]), Ok("c1\nwarning: unknown key \"x\" ignored".into()));
    assert_eq!(take_log(), ["card c1 T Open|false|None|None|TopRight|4|20|true|false", "notify c1|T|"]);
    let done = deploy_in("done");
    assert_eq!(
        f.run(&mut m, true, &["card", "post", &done]),
        Ok(r#"{"id":"c1","replaced":true,"warnings":[]}"#.into())
    );
    assert_eq!(take_log(), ["card c1 Deploy? Done|false|None|None|TopRight|4|20|true|false", "notify c1|Deploy?|Shipped."]);
    assert_eq!(f.run(&mut m, false, &["card", "ls"]).unwrap(), "c1\t11:31\tdone\tDeploy?");
    // A pending card: no sound, no notification.
    f.run(&mut m, false, &["card", "post", &deploy_in("pending")]).unwrap();
    assert_eq!(take_log(), ["card c1 Deploy? Pending|false|None|None|TopRight|4|20|false|false"]);
    assert_eq!(f.store.messages(10).len(), 1);
}

#[test]
fn a_card_replaces_the_pending_message_it_answers() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["post", "--pending", "--id", "k1", "deploy?"]).unwrap();
    take_log();
    let card = r#"{"id":"c2","title":"Deploy?","reply_to":"k1","actions":[{"id":"go","label":"Ship"}]}"#;
    assert_eq!(f.run(&mut m, true, &["card", "post", card]), Ok(r#"{"id":"c2","replaced":true,"warnings":[]}"#.into()));
    assert_eq!(take_log(), ["dismiss k1", "card c2 Deploy? Open|false|None|None|TopRight|4|0|true|true"]);
    assert!(f.store.message("k1").is_none());
    // A plain post may answer a card the same way it answers a message.
    f.run(&mut m, false, &["post", "--reply-to", "c2", "ok"]).unwrap();
    assert!(take_log()[0].starts_with("show m1 Messages|11:31|Re: Deploy? [Ship]|ok|"));
}

#[test]
fn a_dismissed_card_comes_back_only_when_open_or_error() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["card", "post", DEPLOY]).unwrap();
    assert_eq!(f.run(&mut m, false, &["card", "dismiss", "c1"]), Ok("Dismissed c1".into()));
    take_log();
    for state in ["done", "pending"] {
        f.run(&mut m, false, &["card", "post", &deploy_in(state)]).unwrap();
        assert_eq!(take_log(), [""; 0], "{state} stays in history");
    }
    assert_eq!(f.run(&mut m, false, &["card", "ls"]).unwrap(), "c1\t11:31\tpending\tDeploy?");
    f.run(&mut m, false, &["card", "post", &deploy_in("error")]).unwrap();
    assert!(take_log()[0].starts_with("card c1 "));
    assert_eq!(f.run(&mut m, false, &["card", "ls"]).unwrap(), "c1\t11:31\terror\tDeploy?");
    // Shown again, a done update shows too.
    f.run(&mut m, false, &["card", "post", &deploy_in("done")]).unwrap();
    assert_eq!(take_log().len(), 1);
    // `show` brings a dismissed card back and clears the mark.
    f.run(&mut m, false, &["card", "dismiss", "c1"]).unwrap();
    assert_eq!(f.run(&mut m, false, &["card", "show", "c1"]), Ok("Showing c1".into()));
    f.run(&mut m, false, &["card", "post", &deploy_in("done")]).unwrap();
    assert_eq!(take_log().iter().filter(|l| l.starts_with("card")).count(), 2);
}

#[test]
fn dismiss_all_removes_every_card_and_marks_them() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["post", "--id", "t", "text"]).unwrap();
    f.run(&mut m, false, &["card", "post", DEPLOY]).unwrap();
    f.run(&mut m, false, &["card", "post", r#"{"id":"c2","title":"B"}"#]).unwrap();
    take_log();
    assert_eq!(f.run(&mut m, false, &["card", "dismiss", "--all"]), Ok("Dismissed 2 cards".into()));
    assert_eq!(take_log(), ["dismiss c2", "dismiss c1"]);
    f.run(&mut m, false, &["card", "post", &deploy_in("done")]).unwrap();
    assert_eq!(take_log(), [""; 0]);
    m.dismissed.extend((0..300).map(|i| i.to_string()));
    f.run(&mut m, false, &["card", "dismiss", "c2"]).unwrap();
    assert_eq!(m.dismissed.len(), 1, "the set starts over past its cap");
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["card", "post", DEPLOY]).unwrap();
    assert_eq!(f.run(&mut m, false, &["card", "dismiss", "--all"]), Ok("Dismissed 1 card".into()));
}

#[test]
fn the_origin_is_kept_and_decides_flick_actions() {
    let card = r#"{"id":"s","title":"Run?","actions":[{"id":"r","label":"Run","do":{"flick":["script","run","x"]}}]}"#;
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    let reply = f.run(&mut m, false, &["card", "post", card]).unwrap();
    assert!(reply.starts_with("s\nwarning: "), "{reply}");
    // From the network the only action is refused, so the card does not wait: it times out.
    assert!(take_log()[0].ends_with("|20|true|false"));
    assert!(!card::stored(&f.store.message("s").unwrap()).unwrap().actions[0].enabled());
    f.local = true;
    assert_eq!(f.run(&mut m, false, &["card", "post", card]), Ok("s".into()));
    assert!(take_log()[0].ends_with("|0|true|true"));
    let row = f.store.message("s").unwrap();
    assert!(!row.remote && card::stored(&row).unwrap().actions[0].enabled());
}

#[test]
fn bad_cards_and_verbs_are_errors_and_show_nothing() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    let err = |f: &mut Fixture, m: &mut Inbox, words: &[&str]| f.run(m, false, words).unwrap_err();
    assert!(err(&mut f, &mut m, &["card", "post", "{"]).starts_with("invalid card: card is not JSON"));
    assert_eq!(err(&mut f, &mut m, &["card", "post", r#"{"id":"a"}"#]), "invalid card: card needs a non-empty title string");
    let big = format!(r#"{{"id":"a","title":"{}"}}"#, "x".repeat(17 * 1024));
    assert!(err(&mut f, &mut m, &["card", "post", &big]).starts_with("invalid card: card is 17"));
    assert_eq!(take_log(), [""; 0]);
    assert!(f.store.messages(10).is_empty());
    f.run(&mut m, false, &["post", "--id", "t", "text"]).unwrap();
    for verb in ["get", "show", "dismiss"] {
        assert_eq!(err(&mut f, &mut m, &["card", verb, "t"]), "No card t", "{verb}");
    }
    assert!(err(&mut f, &mut m, &["card", "ls", "--limit", "x"]).contains("not a number"));
    for words in [&["card"][..], &["card", "post"], &["card", "post", "a", "b"], &["card", "nope"]] {
        assert!(err(&mut f, &mut m, words).starts_with("usage: flick message card"), "{words:?}");
    }
    assert_eq!(f.run(&mut m, false, &["card", "ls"]), Ok(String::new()));
    assert_eq!(card::body(&core_card::parse(r#"{"id":"a","title":"T"}"#, core_card::Origin::Local).unwrap().card), "");
}

#[test]
fn card_focus_moves_the_keyboard_into_the_newest_card() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    set_keyboard(1);
    assert_eq!(f.run(&mut m, false, &["card", "focus"]), Ok("A card has the keyboard; Esc gives it back".into()));
    set_keyboard(0);
    assert_eq!(f.run(&mut m, false, &["card", "focus"]), Err("No card shows".into()));
    assert_eq!(take_log(), ["focus", "focus"]);
    assert!(f.run(&mut m, false, &["card", "focus", "c1"]).unwrap_err().starts_with("usage:"));
    set_keyboard(1);
}

/// The bodies of the fenced `json` code blocks of `md`.
fn json_blocks(md: &str) -> Vec<String> {
    let (mut blocks, mut open): (Vec<String>, Option<String>) = (vec![], None);
    for line in md.lines() {
        match (open.as_mut(), line.trim_end()) {
            (None, "```json") => open = Some(String::new()),
            (Some(_), "```") => blocks.extend(open.take()),
            (Some(block), _) => {
                block.push_str(line);
                block.push('\n');
            }
            (None, _) => {}
        }
    }
    assert!(open.is_none(), "an unclosed json block in docs/cards.md");
    blocks
}

#[test]
fn card_spec_prints_the_doc_and_its_examples_parse() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    let spec = f.run(&mut m, false, &["card", "spec"]).unwrap();
    assert_eq!(spec, card::SPEC);
    assert!(spec.starts_with("# Cards: the KOTA spec (schema v1)\n"));
    assert!(spec.contains("message card spec") && spec.len() < 25 * 1024);
    assert!(f.run(&mut m, false, &["card", "spec", "x"]).unwrap_err().starts_with("usage:"));
    // Every `json` block is a complete card KOTA could post (remote), clean: no warnings, and
    // every action it declares is enabled (none refused by the remote policy).
    let blocks = json_blocks(card::SPEC);
    assert!(blocks.len() >= 7, "{}", blocks.len());
    for json in &blocks {
        let parsed = core_card::parse(json, core_card::Origin::Remote).unwrap_or_else(|e| panic!("{e}\n{json}"));
        assert!(parsed.warnings.is_empty(), "{:?}\n{json}", parsed.warnings);
        assert!(parsed.card.actions.iter().all(core_card::Action::enabled), "{json}");
    }
    // The form example's untouched inputs are what a press sends by default.
    let form = blocks.iter().find(|b| b.contains("\"dentist\"")).unwrap();
    let form = core_card::parse(form, core_card::Origin::Remote).unwrap().card;
    assert_eq!(core_card::action::values_json(&form.inputs()), Ok(r#"{"note":"","slot":"tue-10","with":[]}"#.into()));
    assert_eq!(json_blocks("```json\n{}\n```\ntext\n```sh\nx\n```"), ["{}\n"]);
}
