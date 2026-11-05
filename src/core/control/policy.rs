//! Which requests a network caller may send (`net_policy`): the deny table for the control
//! protocol's TCP transport.

/// Requests a network caller may not send: `(module, verbs)`, where no verbs means every
/// request to that module (or the bare word, for `reload`). A verb may be several words
/// (`card press`): it matches when the request's words after the module start with them,
/// so `message card press` is denied while `message card post` is not. These change config,
/// run code, take the keyboard (`message chat`), start the microphone, make this Mac ssh
/// (`kota ask`, `message ask`) or send
/// requests to model servers (`llm`, any verb), restart services or read logs (`sys restart`,
/// `sys tail`), pop a key window (`sys window`), pop the launcher, run an extension with a
/// request of the caller's choosing or spawn extensions at all (`ext open`, `ext call`,
/// `ext refresh`: unlike `kota refresh` it has no rate limit), read the screen, write files
/// or delete data that cannot come back (`task rm` drops the task's tracked time). Review this table when a
/// module gains a verb with side effects. `remote` is handled apart: only `remote status` is
/// allowed.
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
    ("snippet", &["type"]),
    ("message", &["card press", "card focus", "chat", "ask"]),
    ("dictation", &[]),
    ("kota", &["ask"]),
    ("llm", &[]),
    ("sys", &["restart", "tail", "window"]),
    ("ext", &["refresh", "open", "call"]),
    ("tile", &["ws", "restore"]),
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
