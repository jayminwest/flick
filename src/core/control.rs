//! The control protocol spoken over Flick's Unix socket. A request is one line holding a
//! JSON array of strings, `["<module>","<verb>",args...]`; the reply is one line,
//! `{"ok":"<output>"}` or `{"error":"<message>"}`. The request `["events"]` instead turns the
//! connection into a stream of events, one JSON object per line.

use serde::{Deserialize, Serialize};

/// The request that subscribes to events instead of getting one reply.
pub const EVENTS: &str = "events";

/// The answer to one request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Ok(String),
    Error(String),
}

impl From<Result<String, String>> for Reply {
    fn from(result: Result<String, String>) -> Reply {
        match result {
            Ok(output) => Reply::Ok(output),
            Err(e) => Reply::Error(e),
        }
    }
}

impl Reply {
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
}
