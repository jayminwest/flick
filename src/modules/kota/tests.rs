//! The module's commands and config.

use super::*;
use crate::config::parse;
use crate::core::test_cx;

fn configured(text: &str) -> Result<Kota, String> {
    let mut k = Kota::default();
    k.configure(&parse(text)?.section(ID)?.ok_or("disabled")?)?;
    Ok(k)
}

fn command(k: &mut Kota, words: &[&str], json: bool) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    test_cx("", |cx| {
        cx.json = json;
        k.command(&args, cx)
    })
}

#[test]
fn status_reports_presence_and_how_it_polls() {
    let mut k = configured("").unwrap();
    assert!(!k.active);
    let status = command(&mut k, &["status"], false).unwrap();
    assert_eq!(status, "KOTA: unknown\nNot checked yet\npolling: off (set a key in [kota] to poll)");
    // `[kota]` with only `enabled = true` is the same as none.
    assert!(!configured("[kota]\nenabled = true").unwrap().active);
    let mut k = configured("[kota]\nmachine = \"server\"").unwrap();
    assert!(k.active);
    let json: serde_json::Value = serde_json::from_str(&command(&mut k, &["status"], true).unwrap()).unwrap();
    assert_eq!((json["state"].as_str(), json["polling"].as_str()), (Some("unknown"), Some("every 60 s")));
    let mut k = configured("[kota]\npoll_secs = 0").unwrap();
    assert!(command(&mut k, &["status"], false).unwrap().ends_with("polling: on demand"));
    assert!(configured("[kota]\npoll_secs = 1").is_err());
}

#[test]
fn verbs_and_unknown_commands() {
    let mut k = configured("").unwrap();
    assert_eq!(k.id(), "kota");
    assert_eq!(k.verbs(), "kota status");
    assert_eq!(command(&mut k, &["nope"], false), Err("kota: unknown command \"nope\"".into()));
    assert_eq!(command(&mut k, &["status", "x"], false), Err("kota: unknown command \"status\"".into()));
    assert_eq!(command(&mut k, &[], false), Err("kota: missing command".into()));
}
