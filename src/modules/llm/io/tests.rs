//! The threads over fake curls (`testkit`): never a real server.

use super::*;
use crate::modules::llm::openai::{Chat, Role, Turn, Usage, chat_body};
use crate::modules::llm::testkit::{HOOKS, PATIENT, sent};

fn server(host: &str) -> Server {
    Server { name: host.into(), url: format!("http://{host}"), private: false }
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Wait until every fetch, stream and watchdog thread has ended: each holds a clone of `shared`.
fn threads_done(shared: &Arc<Shared>) {
    wait_until("the threads to end", || Arc::strong_count(shared) == 1);
}

/// Fetch `host`'s list and wait for it.
fn list(shared: &Arc<Shared>, host: &str) -> Models {
    let seq = fetch_models(shared, &server(host), HOOKS);
    wait_until(host, || shared.lock().models.get(host).is_some_and(|m| m.seq >= seq));
    shared.lock().models[host].clone()
}

fn list_error(host: &str) -> String {
    list(&Arc::default(), host).result.unwrap().unwrap_err()
}

#[test]
fn a_model_list_lands_under_its_server() {
    let shared = Arc::<Shared>::default();
    let m = list(&shared, "mlx");
    assert_eq!((m.seq, m.fetching), (1, false));
    let ids: Vec<String> = m.result.unwrap().unwrap().into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["qwen3-30b-a3b-4bit", "gemma-3-12b-it-4bit"]);
    assert!(shared.lock().posted);
    assert_eq!(list(&shared, "mlx").seq, 2);
    let argv = &sent("http://mlx/v1/models")[0].argv;
    assert_eq!(argv[..2], ["/usr/bin/curl", "-q"]);
    assert!(argv.join(" ").contains("--max-time 3 --url"));
}

#[test]
fn failed_lists_say_why() {
    assert_eq!(list_error("down"), "(7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server");
    assert_eq!(list_error("err"), "model not found");
    assert_eq!(list_error("junk"), "not a model list: <html>hi</html>");
    assert_eq!(list_error("nospawn"), "/usr/bin/curl: No such file or directory");
    // A hung server: the watchdog kills curl after its budget.
    assert_eq!(list_error("hang"), "curl was stopped");
}

#[test]
fn one_fetch_per_server_at_a_time() {
    let shared = Arc::<Shared>::default();
    let first = fetch_models(&shared, &server("hang"), HOOKS);
    assert_eq!(fetch_models(&shared, &server("hang"), HOOKS), first);
    assert!(shared.lock().models["hang"].fetching);
    wait_until("watchdog", || shared.lock().models["hang"].seq == first);
}

#[test]
fn stop_drops_a_fetch_in_flight() {
    let shared = Arc::<Shared>::default();
    fetch_models(&shared, &server("hang"), HOOKS);
    shared.stop();
    assert!(!shared.lock().models["hang"].fetching);
    // The watchdog kills it; its late result is dropped.
    threads_done(&shared);
    assert_eq!(shared.lock().models["hang"].seq, 0);
}

fn body(text: &str) -> Vec<u8> {
    let turns = [Turn { role: Role::User, content: text }];
    chat_body(&Chat { model: "qwen3", system: "", turns: &turns, max_tokens: 0 })
}

/// Stream from `host` to its end; its pieces and status.
fn reply(host: &str, prompt: &str) -> (Vec<Piece>, Status) {
    let shared = Arc::<Shared>::default();
    let id = chat(&shared, &server(host), body(prompt), 30, PATIENT);
    let mut pieces = vec![];
    let mut status = Status::Running;
    wait_until(host, || {
        let (p, s) = shared.take(id).unwrap();
        pieces.extend(p);
        status = s;
        status != Status::Running
    });
    assert_eq!(shared.take(id), None, "an ended stream is forgotten once taken");
    (pieces, status)
}

#[test]
fn a_reply_streams_into_the_inbox_and_the_prompt_goes_over_stdin_only() {
    let (pieces, status) = reply("mlx", "the secret canary 7f3a");
    assert_eq!(status, Status::Done);
    assert_eq!(pieces[..3], [Piece::Text("Hello".into()), Piece::Text(", world".into()), Piece::Text(".".into())]);
    assert_eq!(pieces[3..], [Piece::Finish("stop".into()), Piece::Usage(Usage { prompt: 12, completion: 4 }), Piece::Done]);
    let sent = sent("http://mlx/v1/chat/completions");
    let canary = sent.iter().find(|s| String::from_utf8_lossy(&s.stdin).contains("the secret canary 7f3a")).unwrap();
    assert!(!canary.argv.iter().any(|w| w.contains("canary")));
    assert!(canary.argv.join(" ").contains("--max-time 30 -H Content-Type: application/json"));
}

#[test]
fn reasoning_arrives_before_the_answer() {
    let (pieces, status) = reply("reason", "2+2");
    assert_eq!(status, Status::Done);
    assert_eq!(pieces[0], Piece::Reasoning("Two plus two".into()));
    assert_eq!(pieces[2], Piece::Text("4".into()));
}

#[test]
fn failed_replies_say_why() {
    let failed = |host: &str| reply(host, "x").1;
    assert_eq!(failed("err"), Status::Failed("model not found".into()));
    assert_eq!(failed("down"), Status::Failed("(7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server".into()));
    assert_eq!(failed("empty"), Status::Failed("the server sent no reply".into()));
    assert_eq!(failed("junk"), Status::Failed("the server sent no reply".into()));
    assert_eq!(failed("nospawn"), Status::Failed("/usr/bin/curl: No such file or directory".into()));
    let (pieces, status) = reply("inband", "x");
    assert_eq!(status, Status::Failed("overloaded".into()));
    assert_eq!(pieces, [Piece::Text("a".into()), Piece::Error("overloaded".into())]);
}

#[test]
fn a_failed_curl_with_a_json_error_body_reports_the_body() {
    let seen = Seen { raw: r#"{"error":"busy"}"#.into(), ..Seen::default() };
    assert_eq!(seen.status(Ok(Ended { code: Some(22), stderr: "curl: (22) 503".into() })), Status::Failed("busy".into()));
    assert_eq!(seen.status(Ok(Ended { code: Some(0), stderr: String::new() })), Status::Failed("busy".into()));
}

#[test]
fn cancel_keeps_what_arrived() {
    let shared = Arc::<Shared>::default();
    let id = chat(&shared, &server("half"), body("x"), 30, PATIENT);
    let mut pieces = vec![];
    wait_until("first piece", || {
        pieces.extend(shared.take(id).unwrap().0);
        !pieces.is_empty()
    });
    assert_eq!(pieces, [Piece::Text("part".into())]);
    assert!(cancel(&shared, id));
    assert!(!cancel(&shared, id));
    assert!(!cancel(&shared, 999));
    assert_eq!(shared.take(id), Some((vec![], Status::Cancelled)));
}

#[test]
fn a_reply_past_its_budget_times_out() {
    let shared = Arc::<Shared>::default();
    let id = chat(&shared, &server("hang"), body("x"), 1, HOOKS);
    wait_until("watchdog", || shared.lock().stream(id).is_some_and(|s| s.status != Status::Running));
    assert_eq!(shared.take(id), Some((vec![], Status::Failed("timed out after 1 s".into()))));
    // A watchdog that wakes after its stream ended (and was taken) leaves it be.
    let done = chat(&shared, &server("mlx"), body("x"), 1, HOOKS);
    wait_until("reply", || shared.take(done).is_some_and(|(_, s)| s == Status::Done));
    threads_done(&shared);
    assert!(shared.lock().streams.is_empty());
}

#[test]
fn stop_kills_every_stream() {
    let shared = Arc::<Shared>::default();
    let id = chat(&shared, &server("half"), body("x"), 30, PATIENT);
    wait_until("child", || shared.lock().stream(id).is_some_and(|s| s.child.is_some()));
    shared.stop();
    assert_eq!(shared.take(id), None);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(shared.take(id), None);
}

#[test]
fn one_event_is_posted_until_it_arrives() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static POSTS: AtomicUsize = AtomicUsize::new(0);
    let hooks = Hooks { post: || {
        POSTS.fetch_add(1, Ordering::SeqCst);
    }, ..PATIENT };
    let shared = Arc::<Shared>::default();
    let id = chat(&shared, &server("mlx"), body("x"), 30, hooks);
    wait_until("end", || shared.lock().stream(id).is_some_and(|s| s.status != Status::Running));
    assert_eq!(POSTS.load(Ordering::SeqCst), 1);
    shared.lock().posted = false;
    let id = chat(&shared, &server("mlx"), body("x"), 30, hooks);
    wait_until("end", || shared.lock().stream(id).is_some_and(|s| s.status != Status::Running));
    assert_eq!(POSTS.load(Ordering::SeqCst), 2);
}
