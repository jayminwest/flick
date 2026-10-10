//! The control protocol spoken over Flick's Unix socket. A request is one line holding a
//! JSON array of strings, `["<module>","<verb>",args...]`; the reply is one line,
//! `{"ok":"<output>"}` or `{"error":"<message>"}`. A request whose last word is `--json`
//! asks for structured output: a module's JSON object or array answer is then sent as the
//! value itself, `{"ok":<value>}`. Before that word, a trailing `--remote` marks a request
//! from a session that may send its output to a remote model (`Cx::remote`). The request
//! `["events"]` instead turns the connection into a stream of events, one JSON object per
//! line.
//!
//! The same protocol also runs over TCP for peers in the user's tailnet. The pure rules for
//! that transport live here: which addresses are Tailscale addresses (`is_tailnet`), which
//! peers may connect (`peer_matches`), which requests a network caller may send
//! (`net_policy`), and the contract between the remote module and the transport
//! (`NetSettings`, `NetStatus`, `NetHooks`), and the client seam a module asks a peer
//! through (`PeerHooks`).

use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The request that subscribes to events instead of getting one reply.
pub const EVENTS: &str = "events";

/// The last request word that asks for a structured reply.
pub const JSON: &str = "--json";

/// The request word, last or just before `--json`, that marks a remote caller.
pub const REMOTE: &str = "--remote";

/// What the trailing flag words of a request asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// A structured reply (`--json`).
    pub json: bool,
    /// The caller may send the output to a remote model (`--remote`).
    pub remote: bool,
}

/// The answer to one request: text (a JSON string) or, for a `--json` request, any JSON
/// value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Ok(Value),
    Error(String),
}

impl From<Result<String, String>> for Reply {
    fn from(result: Result<String, String>) -> Reply {
        match result {
            Ok(output) => Reply::Ok(Value::String(output)),
            Err(e) => Reply::Error(e),
        }
    }
}

impl Reply {
    /// The reply to a request that asked for `json`: an `Ok` text that parses as a JSON
    /// object or array is sent as that value; any other text stays a string.
    pub fn answer(result: Result<String, String>, json: bool) -> Reply {
        match result {
            Ok(text) if json => match serde_json::from_str::<Value>(&text) {
                Ok(value @ (Value::Object(_) | Value::Array(_))) => Reply::Ok(value),
                _ => Reply::Ok(Value::String(text)),
            },
            other => Reply::from(other),
        }
    }

    /// The reply as one JSON line, without the newline.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse a reply line.
    pub fn parse(line: &str) -> Result<Reply, String> {
        serde_json::from_str(line.trim()).map_err(|e| format!("bad reply from Flick: {e}"))
    }
}

/// Parse a request line into its words. An empty array is an error.
pub fn parse_request(line: &str) -> Result<Vec<String>, String> {
    let words: Vec<String> = serde_json::from_str(line.trim())
        .map_err(|e| format!("bad request (want a JSON array of strings): {e}"))?;
    if words.is_empty() {
        return Err("empty request".into());
    }
    Ok(words)
}

/// Split the trailing flag words off `words`: first a trailing `--json`, then a trailing
/// `--remote`. Returns the words a module sees and the flags. Elsewhere both are arguments.
pub fn split_flags(mut words: Vec<String>) -> (Vec<String>, Flags) {
    let json = words.pop_if(|w| w == JSON).is_some();
    let remote = words.pop_if(|w| w == REMOTE).is_some();
    (words, Flags { json, remote })
}

/// A request line for `words`, without the newline.
pub fn request_line(words: &[String]) -> String {
    serde_json::to_string(words).unwrap_or_default()
}

/// The TCP port the network transport listens on when `[remote] port` is not set.
pub const DEFAULT_PORT: u16 = 7419;

/// Whether `ip` is a Tailscale address: IPv4 100.64.0.0/10 (CGNAT) or IPv6
/// `fd7a:115c:a1e0::/48`. An IPv4-mapped IPv6 address is not one: the transport binds and
/// sees only plain Tailscale addresses.
pub fn is_tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 100 && b & 0xc0 == 64
        }
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// Whether a peer that `tailscale whois` knows by `names` (its `ComputedName`,
/// Hostinfo.Hostname and Node.Name) is one of the allowed `peers`. Case-insensitive; a name
/// also matches by its first DNS label, so `mbp-server.tail1.ts.net.` matches peer
/// `mbp-server`. Empty names and peers match nothing.
pub fn peer_matches(names: &[String], peers: &[String]) -> bool {
    let candidates = names.iter().flat_map(|n| {
        let full = n.trim_end_matches('.');
        [full, full.split('.').next().unwrap_or(full)]
    });
    candidates
        .filter(|c| !c.is_empty())
        .any(|c| peers.iter().any(|p| p.trim_end_matches('.').eq_ignore_ascii_case(c)))
}

/// Requests a network caller may not send: `(module, verbs)`, where no verbs means every
/// request to that module (or the bare word, for `reload`). A verb may be several words
/// (`card press`): it matches when the request's words after the module start with them,
/// so `message card press` is denied while `message card post` is not. These change config,
/// run code, take the keyboard, start the microphone, make this Mac ssh (`kota ask`), read
/// the screen, write files or delete data that cannot come back (`task rm` drops the task's
/// tracked time). Review this table when a module gains a verb with side effects. `remote`
/// is handled apart: only `remote status` is allowed.
const NET_DENIED: &[(&str, &[&str])] = &[
    ("reload", &[]),
    ("flick", &["rebuild", "cancel"]),
    ("keys", &["fire"]),
    ("app", &["uninstall"]),
    ("quicklink", &["add", "remove"]),
    ("capture", &[]),
    ("feedback", &["resolve"]),
    ("task", &["rm"]),
    ("script", &["run"]),
    ("message", &["card press", "card focus"]),
    ("dictation", &[]),
    ("kota", &["ask"]),
];

/// The module whose network-access toggle a network caller may only read.
const REMOTE_MODULE: &str = "remote";

/// Whether a network caller may send `words` (a request after `split_flags`). `Err` is the
/// refusal to send back, naming the module and the denied verb's words. Everything not
/// denied here is allowed; modules still apply their own remote guards, since every network
/// request has `Cx::remote` set.
pub fn net_policy(words: &[String]) -> Result<(), String> {
    let Some((module, rest)) = words.split_first() else { return Ok(()) };
    let first = rest.first().map(String::as_str);
    let starts = |verb: &str| {
        let verb: Vec<&str> = verb.split(' ').collect();
        rest.len() >= verb.len() && rest.iter().zip(&verb).all(|(w, v)| w == v)
    };
    let denied = if module == REMOTE_MODULE {
        (first != Some("status")).then(|| first.unwrap_or_default())
    } else {
        NET_DENIED.iter().filter(|(m, _)| m == module).find_map(|(_, verbs)| {
            if verbs.is_empty() {
                Some(first.unwrap_or_default())
            } else {
                verbs.iter().copied().find(|v| starts(v))
            }
        })
    };
    match denied {
        None => Ok(()),
        Some("") => Err(format!("{module}: not allowed over the network")),
        Some(verb) => Err(format!("{module} {verb}: not allowed over the network")),
    }
}

/// What the network transport should serve: built by the remote module from `[remote]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetSettings {
    /// Tailscale peer names allowed to connect (see `peer_matches`).
    pub peers: Vec<String>,
    /// The TCP port to listen on.
    pub port: u16,
    /// Whether network callers may subscribe to `["events"]`.
    pub events: bool,
}

/// The last network connection the transport saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LastConn {
    /// The peer's name from `tailscale whois`, `None` when whois failed.
    pub name: Option<String>,
    /// The peer's address.
    pub ip: IpAddr,
    /// When it connected, in unix seconds.
    pub at: i64,
    /// Whether it was let in.
    pub allowed: bool,
    /// Why it was refused, `None` when allowed.
    pub reason: Option<String>,
}

/// The transport's state, for `remote status`. The default is "not listening".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NetStatus {
    /// The addresses it listens on; empty when stopped.
    pub listening: Vec<SocketAddr>,
    /// The allowed peers it serves.
    pub peers: Vec<String>,
    /// The last connection, allowed or refused.
    pub last: Option<LastConn>,
    /// Why it is not listening (no Tailscale, bind failure), if it tried.
    pub error: Option<String>,
}

/// How the remote module drives the transport without importing it: `modules/mod.rs`
/// passes the control layer's hooks to the module's constructor; tests pass fakes.
#[derive(Clone, Copy, Debug)]
pub struct NetHooks {
    /// Stop any running listener, then for `Some` start one with these settings.
    pub apply: fn(Option<NetSettings>) -> Result<NetStatus, String>,
    /// The transport's current state.
    pub status: fn() -> NetStatus,
}

/// How a module asks another Mac's Flick one question without importing the client (the
/// `sys` fleet reads a peer's `sys snapshot`): `modules/mod.rs` passes `cli::client::PEER`;
/// tests pass fakes. Blocking, so only background threads call it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the sys fleet reads peers through it (flick-3608)")
)]
#[derive(Clone, Copy, Debug)]
pub struct PeerHooks {
    /// Send `words` with `flags` (`--remote` always) to the Flick at `host` (`name[:port]`)
    /// and read its one reply: connect 5 s per address, 10 s for the reply. `Err` when no
    /// reply came (bad host, unreachable, timed out, connection dropped, not a reply).
    pub ask: fn(host: &str, words: &[String], flags: Flags) -> Result<Reply, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn requests_round_trip_as_json_arrays() {
        let req = words(&["clip", "get", "say \"hi\"\n"]);
        let line = request_line(&req);
        assert_eq!(line, r#"["clip","get","say \"hi\"\n"]"#);
        assert!(!line.contains('\n'));
        assert_eq!(parse_request(&format!("{line}\n")).unwrap(), req);
    }

    #[test]
    fn malformed_requests_are_errors() {
        assert_eq!(parse_request("[]").unwrap_err(), "empty request");
        for bad in ["", "clip list", r#"{"a":1}"#, "[1,2]", r#"["a""#] {
            let err = parse_request(bad).unwrap_err();
            assert!(err.starts_with("bad request"), "{bad}: {err}");
        }
    }

    #[test]
    fn replies_have_one_stable_shape() {
        let ok = Reply::from(Ok("a\nb".to_string()));
        let err = Reply::from(Err::<String, _>("nope".to_string()));
        assert_eq!(ok.to_line(), r#"{"ok":"a\nb"}"#);
        assert_eq!(err.to_line(), r#"{"error":"nope"}"#);
        assert_eq!(Reply::parse(&format!("{}\n", ok.to_line())).unwrap(), ok);
        assert_eq!(Reply::parse(&err.to_line()).unwrap(), err);
        assert!(Reply::parse("{}").unwrap_err().starts_with("bad reply"));
    }

    #[test]
    fn json_requests_embed_object_and_array_answers() {
        let ok = |s: &str| Ok::<_, String>(s.to_string());
        let line = |r: Result<String, String>, json| Reply::answer(r, json).to_line();
        assert_eq!(line(ok(r#"{"b": [1, 2],"a":"x"}"#), true), r#"{"ok":{"a":"x","b":[1,2]}}"#);
        assert_eq!(line(ok("[\n1\n]"), true), r#"{"ok":[1]}"#);
        // Scalars, broken JSON and text stay strings; without --json nothing is parsed.
        assert_eq!(line(ok("3"), true), r#"{"ok":"3"}"#);
        assert_eq!(line(ok("{oops"), true), r#"{"ok":"{oops"}"#);
        assert_eq!(line(ok("[1]"), false), r#"{"ok":"[1]"}"#);
        assert_eq!(line(Err("no".into()), true), r#"{"error":"no"}"#);
        let value = Reply::answer(ok("[1]"), true);
        assert_eq!(Reply::parse(&value.to_line()).unwrap(), value);
    }

    #[test]
    fn only_trailing_flag_words_are_split_off() {
        let flags = |json, remote| Flags { json, remote };
        let cases: [(&[&str], &[&str], Flags); 9] = [
            (&["a", "b"], &["a", "b"], flags(false, false)),
            (&["a", "b", "--json"], &["a", "b"], flags(true, false)),
            (&["a", "b", "--remote"], &["a", "b"], flags(false, true)),
            (&["a", "--remote", "--json"], &["a"], flags(true, true)),
            // --json goes last: before --remote it is an argument.
            (&["a", "--json", "--remote"], &["a", "--json"], flags(false, true)),
            (&["a", "--json", "b"], &["a", "--json", "b"], flags(false, false)),
            (&["a", "--remote", "b"], &["a", "--remote", "b"], flags(false, false)),
            (&["--json"], &[], flags(true, false)),
            (&["--remote", "--json"], &[], flags(true, true)),
        ];
        for (input, rest, want) in cases {
            assert_eq!(split_flags(words(input)), (words(rest), want), "{input:?}");
        }
    }

    #[test]
    fn only_tailscale_ranges_are_tailnet() {
        let tailnet = [
            "100.64.0.0",
            "100.100.100.100",
            "100.127.255.255",
            "fd7a:115c:a1e0::1",
            "fd7a:115c:a1e0:ab12::5",
        ];
        let other = [
            "0.0.0.0",
            "127.0.0.1",
            "192.168.1.10",
            "10.0.0.1",
            "100.63.255.255",
            "100.128.0.0",
            "101.64.0.1",
            "::",
            "::1",
            "fd7a:115c:a1e1::1",
            "fd7a:115c::1",
            "::ffff:100.64.0.1",
            "fe80::1",
        ];
        for ip in tailnet {
            assert!(is_tailnet(ip.parse().unwrap()), "{ip}");
        }
        for ip in other {
            assert!(!is_tailnet(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn peers_match_any_name_case_insensitively() {
        let peers = words(&["mbp-server", "Studio.tail1.ts.net"]);
        let yes: [&[&str]; 5] = [
            &["mbp-server"],
            &["MBP-Server"],
            &["other", "mbp-server.tail1.ts.net."],
            &["studio.TAIL1.ts.net."],
            &["", "x", "mbp-server"],
        ];
        for names in yes {
            assert!(peer_matches(&words(names), &peers), "{names:?}");
        }
        // A suffixed ComputedName, another tailnet's label or a prefix do not match.
        let no: [&[&str]; 5] = [&[], &[""], &["mbp-server-1"], &["studio"], &["mbp"]];
        for names in no {
            assert!(!peer_matches(&words(names), &peers), "{names:?}");
        }
        assert!(!peer_matches(&words(&["mbp-server"]), &[]));
        assert!(!peer_matches(&words(&["", "."]), &words(&[""])));
    }

    #[test]
    fn network_policy_refuses_side_effects() {
        let refused: [&[&str]; 13] = [
            &["reload"],
            &["flick", "rebuild"],
            &["flick", "cancel"],
            &["remote"],
            &["remote", "on"],
            &["remote", "off"],
            &["keys", "fire", "hyper", "down"],
            &["app", "uninstall", "Safari"],
            &["quicklink", "add", "X", "https://x"],
            &["quicklink", "remove", "X"],
            &["capture"],
            &["capture", "screen", "--out", "/tmp/x.png"],
            &["feedback", "resolve", "2026-10-09T11:31:01-07:00"],
        ];
        for req in refused {
            let err = net_policy(&words(req)).unwrap_err();
            assert!(err.ends_with(": not allowed over the network"), "{req:?}: {err}");
        }
        assert_eq!(
            net_policy(&words(&["flick", "rebuild", "HEAD"])).unwrap_err(),
            "flick rebuild: not allowed over the network"
        );
        assert_eq!(
            net_policy(&words(&["capture"])).unwrap_err(),
            "capture: not allowed over the network"
        );
        let allowed: [&[&str]; 13] = [
            &[],
            &["task", "ls"],
            &["herdr", "ls"],
            &["window", "list"],
            &["app", "list"],
            &["app", "open", "Safari"],
            &["clip", "list"],
            &["activity", "status"],
            &["remote", "status"],
            &["remote", "status", "x"],
            &["flick", "version"],
            &["keys", "list"],
            &["feedback", "add", "hi"],
        ];
        for req in allowed {
            assert_eq!(net_policy(&words(req)), Ok(()), "{req:?}");
        }
    }

    #[test]
    fn multi_word_verbs_deny_only_that_verb() {
        let refusal = |req: &[&str]| net_policy(&words(req)).unwrap_err();
        assert_eq!(
            refusal(&["message", "card", "press", "c1", "ok"]),
            "message card press: not allowed over the network"
        );
        assert_eq!(
            refusal(&["message", "card", "focus"]),
            "message card focus: not allowed over the network"
        );
        assert_eq!(refusal(&["script", "run", "Lock"]), "script run: not allowed over the network");
        assert_eq!(refusal(&["kota", "ask", "hi"]), "kota ask: not allowed over the network");
        assert_eq!(refusal(&["remote", ""]), "remote: not allowed over the network");
        for req in [
            &["message", "card", "post", "--stdin"][..],
            &["message", "card"],
            &["message", "card", "pressed"],
            &["message", "press"],
            &["message", "post", "card", "press"],
            &["message", "focus"],
            &["script"],
            &["script", "ls"],
            &["kota", "status"],
            &["kota", "refresh"],
        ] {
            assert_eq!(net_policy(&words(req)), Ok(()), "{req:?}");
        }
    }

    #[test]
    fn net_status_serializes_for_remote_status() {
        assert_eq!(
            serde_json::to_string(&NetStatus::default()).unwrap(),
            r#"{"listening":[],"peers":[],"last":null,"error":null}"#
        );
        let last = LastConn {
            name: Some("mbp-server".into()),
            ip: "100.64.0.2".parse().unwrap(),
            at: 7,
            allowed: false,
            reason: Some("not a peer".into()),
        };
        let status = NetStatus {
            listening: vec!["100.64.0.1:7419".parse().unwrap()],
            peers: words(&["x"]),
            last: Some(last),
            error: None,
        };
        let want = r#"{"listening":["100.64.0.1:7419"],"peers":["x"],"last":{"name":"mbp-server","ip":"100.64.0.2","at":7,"allowed":false,"reason":"not a peer"},"error":null}"#;
        assert_eq!(serde_json::to_string(&status).unwrap(), want);
        assert_eq!(DEFAULT_PORT, 7419);
    }

    #[test]
    fn net_hooks_are_plain_fn_pointers() {
        fn apply(settings: Option<NetSettings>) -> Result<NetStatus, String> {
            let s = settings.ok_or("off")?;
            Ok(NetStatus { peers: s.peers, ..NetStatus::default() })
        }
        let hooks = NetHooks { apply, status: NetStatus::default };
        let on = NetSettings { peers: words(&["a"]), port: DEFAULT_PORT, events: false };
        assert_eq!((hooks.apply)(Some(on.clone())).unwrap().peers, on.peers);
        assert_eq!((hooks.apply)(None).unwrap_err(), "off");
        assert_eq!((hooks.status)(), NetStatus::default());
    }
}
