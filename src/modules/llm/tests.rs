//! The module over fake curls (`testkit`): never a real server.

use super::*;
use crate::config::parse;
use crate::core::test_cx;
use testkit::HOOKS;

const SERVERS: &str = "[[llm.servers]]\nname = \"mlx\"\nurl = \"http://mlx\"\n\
    [[llm.servers]]\nname = \"down\"\nurl = \"http://down/v1\"\n\
    [[llm.servers]]\nname = \"hang\"\nurl = \"http://hang\"\n\
    [[llm.servers]]\nname = \"vault\"\nurl = \"http://mlx:11235\"\nprivate = true\n";

fn configured(text: &str) -> Result<Llm, String> {
    let mut m = Llm::with_hooks(HOOKS, testkit::UI, testkit::PRIVATE);
    m.configure(&parse(text)?.section(ID)?.ok_or("disabled")?)?;
    Ok(m)
}

fn command(m: &mut Llm, words: &[&str], json: bool) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    test_cx("", |cx| {
        cx.json = json;
        m.command(&args, cx)
    })
}

#[test]
fn without_servers_nothing_runs_and_verbs_say_how_to_add_one() {
    let mut m = configured("").unwrap();
    let none = "llm: no servers; add a [[llm.servers]] table to config.toml";
    assert_eq!(command(&mut m, &["ping"], false).unwrap_err(), none);
    assert_eq!(command(&mut m, &["models"], true).unwrap_err(), none);
    assert!(m.shared.lock().models.is_empty());
    test_cx("", |cx| assert!(!m.on_event(Event::Started, cx)));
}

#[test]
fn ping_and_models_ask_a_normal_server() {
    let mut m = configured(SERVERS).unwrap();
    let ping = command(&mut m, &["ping"], false).unwrap();
    assert!(ping.starts_with("llm: mlx answers in ") && ping.ends_with(" ms (2 models)"), "{ping}");
    let text = command(&mut m, &["models", "mlx"], false).unwrap();
    assert_eq!(text, "qwen3-30b-a3b-4bit   loaded  ctx 40960  chat,reasoning\ngemma-3-12b-it-4bit  unloaded  chat");
    let json: serde_json::Value = serde_json::from_str(&command(&mut m, &["models"], true).unwrap()).unwrap();
    assert_eq!(json["server"], "mlx");
    assert_eq!(json["models"][0]["context_length"], 40_960);
    assert_eq!(m.shared.lock().models["mlx"].seq, 3);
}

#[test]
fn errors_name_the_server() {
    let mut m = configured(SERVERS).unwrap();
    let down = "llm: down: (7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server (is the server running, and its port published (tailscale serve)?)";
    assert_eq!(command(&mut m, &["ping", "down"], false).unwrap_err(), down);
    assert_eq!(command(&mut m, &["models", "down"], false).unwrap_err(), down);
    let private = "llm: vault is private; only the private chat talks to it";
    assert_eq!(command(&mut m, &["ping", "vault"], false).unwrap_err(), private);
    assert_eq!(command(&mut m, &["models", "vault"], false).unwrap_err(), private);
    assert_eq!(command(&mut m, &["ping", "nope"], false).unwrap_err(), "llm: no server \"nope\"");
    assert!(command(&mut m, &["chat"], false).unwrap_err().contains("llm"));
    assert!(m.verbs().contains("llm ping") && m.verbs().contains("llm models"));
}

/// flick-72b5: `ping private` reaches a private server with a bodiless `GET /v1/models`
/// and says only whether it answers; `models` and a plain `ping` still refuse it.
#[test]
fn ping_private_asks_only_a_private_server_for_its_list() {
    let mut m = configured(SERVERS).unwrap();
    let ping = command(&mut m, &["ping", "private"], false).unwrap();
    assert!(ping.starts_with("llm: vault answers in ") && ping.ends_with(" ms (2 models)"), "{ping}");
    assert!(!ping.contains("qwen"), "{ping}");
    assert_eq!(command(&mut m, &["ping", "private", "vault"], true).unwrap().split(" in ").next(), Some("llm: vault answers"));
    let sent = testkit::sent("http://mlx:11235/v1/models");
    assert!(!sent.is_empty() && sent.iter().all(|s| s.stdin.is_empty()));
    let normal = "llm: mlx is not private; the private chat never uses it";
    assert_eq!(command(&mut m, &["ping", "private", "mlx"], false).unwrap_err(), normal);
    assert!(command(&mut m, &["ping", "private", "vault", "x"], false).unwrap_err().contains("llm"));
    assert!(command(&mut m, &["models", "private"], false).unwrap_err().contains("no server \"private\""));
    let mut none = configured("[[llm.servers]]\nname = \"mlx\"\nurl = \"http://mlx\"\n").unwrap();
    assert_eq!(command(&mut none, &["ping", "private"], false).unwrap_err(), settings::NO_PRIVATE);
    assert!(m.verbs().contains("llm ping private"));
    // It opens no private session and takes no `Cx` path to the store.
    assert!(m.private.room.lock().unwrap().is_none());
}

#[test]
fn a_slow_server_is_not_waited_on_past_the_budget() {
    let mut m = configured(SERVERS).unwrap();
    m.ask_wait = Duration::from_millis(10);
    assert_eq!(command(&mut m, &["ping", "hang"], false).unwrap_err(), "llm: hang did not answer in 0 s");
}

#[test]
fn extra_words_are_unknown() {
    let mut m = configured(SERVERS).unwrap();
    assert!(command(&mut m, &["ping", "mlx", "x"], false).unwrap_err().contains("llm"));
}

#[test]
fn its_own_event_marks_its_views_stale_and_rearms_posting() {
    let mut m = configured(SERVERS).unwrap();
    m.shared.lock().posted = true;
    test_cx("", |cx| {
        assert!(!m.on_event(Event::ModuleChanged { module: "kota" }, cx));
        assert!(m.shared.lock().posted);
        assert!(m.on_event(Event::ModuleChanged { module: ID }, cx));
    });
    assert!(!m.shared.lock().posted);
}

#[test]
fn a_bad_table_is_refused() {
    assert_eq!(configured("[llm]\ntimeout_secs = 0").err().unwrap(), "[llm]: timeout_secs must be 1 to 3600");
}
