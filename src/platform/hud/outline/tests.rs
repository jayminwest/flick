use super::*;
use crate::core::card::{Origin, parse};

fn parsed(json: &str) -> Card {
    match parse(json, Origin::Local) {
        Ok(p) => p.card,
        Err(e) => panic!("bad test card: {e}"),
    }
}

#[test]
fn a_card_lists_its_text_inputs_status_and_action_ids() {
    let c = parsed(
        r#"{"id":"c1","title":"Deploy?","blocks":[
            {"type":"text","md":"Ship it\nnow"},
            {"type":"field","id":"why","label":"Why","value":"ok"},
            {"type":"choice","id":"env","options":["prod","stg"],"selected":"prod"},
            {"type":"choice","id":"tags","options":["a","b"],"multi":true}],
          "actions":[{"id":"yes","label":"Yes"},{"id":"bad","label":"Bad","do":{"nope":1}}]}"#,
    );
    let lines = card(&c, &CardUi::default());
    assert_eq!(lines[0], "title: Deploy?");
    assert!(lines.contains(&"input why: \"ok\"".to_string()), "{lines:?}");
    assert!(lines.contains(&"input env: prod".to_string()), "{lines:?}");
    assert!(lines.contains(&"input tags: ".to_string()), "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("actions: yes \"Yes\", bad \"Bad\" (off)"));
    assert!(lines.iter().any(|l| l.starts_with("| ")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("[Yes]")), "{lines:?}");
    // While pending every action is off and the status says why.
    let pending = card(&c, &CardUi { pending: true, ..CardUi::default() });
    assert!(pending.contains(&"status: Sent to the agent…".to_string()), "{pending:?}");
    assert!(pending.last().is_some_and(|l| l.starts_with("actions: yes \"Yes\" (off),")));
}

#[test]
fn a_shell_confirm_offers_run_and_cancel() {
    let c = parsed(
        r#"{"id":"c2","title":"Clean","actions":[{"id":"rm","label":"Remove","do":{"shell":"rm x"}}]}"#,
    );
    let ui = CardUi { confirm: Some("rm"), ..CardUi::default() };
    assert_eq!(
        card(&c, &ui),
        ["title: Clean", "confirm: rm x", "actions: rm \"Run\", :cancel \"Cancel\""]
    );
    let bare = parsed(r#"{"id":"c3","title":"Note"}"#);
    assert_eq!(card(&bare, &CardUi::default()), ["title: Note"]);
}

#[test]
fn a_text_card_lists_header_body_and_link() {
    let t = TextCard {
        header: "KOTA",
        time: "14:03",
        context: "Re: build",
        body: "one\ntwo",
        link: Some("https://example.com/x"),
        pending: true,
    };
    assert_eq!(
        text(&t),
        [
            "header: KOTA  14:03",
            "context: Re: build",
            "| one",
            "| two",
            "status: waiting for a reply",
            "link: https://example.com/x",
        ]
    );
    let bare = TextCard { time: "", context: "", body: "", link: None, pending: false, ..t };
    assert_eq!(text(&bare), ["header: KOTA"]);
}
