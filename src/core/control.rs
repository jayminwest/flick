//! The control protocol spoken over Flick's Unix socket. A request is one line holding a
//! JSON array of strings, `["<module>","<verb>",args...]`; the reply is one line,
//! `{"ok":"<output>"}` or `{"error":"<message>"}`. A request whose last word is `--json`
//! asks for structured output: a module's JSON object or array answer is then sent as the
//! value itself, `{"ok":<value>}`. The request `["events"]` instead turns the connection
//! into a stream of events, one JSON object per line.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The request that subscribes to events instead of getting one reply.
pub const EVENTS: &str = "events";

/// The last request word that asks for a structured reply.
pub const JSON: &str = "--json";

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

/// Split a trailing `--json` off `words`: the words a module sees, and whether a structured
/// reply was asked for.
pub fn split_json(mut words: Vec<String>) -> (Vec<String>, bool) {
    let json = words.pop_if(|w| w == JSON).is_some();
    (words, json)
}

/// A request line for `words`, without the newline.
pub fn request_line(words: &[String]) -> String {
    serde_json::to_string(words).unwrap_or_default()
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
    fn only_a_trailing_json_word_is_split_off() {
        assert_eq!(split_json(words(&["a", "b", "--json"])), (words(&["a", "b"]), true));
        assert_eq!(split_json(words(&["a", "--json", "b"])), (words(&["a", "--json", "b"]), false));
        assert_eq!(split_json(words(&["--json"])), (vec![], true));
    }
}
