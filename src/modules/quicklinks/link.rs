//! A quicklink: a named URL or path, optionally taking a `{query}` argument.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Quicklink {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
}

impl Quicklink {
    pub fn takes_query(&self) -> bool {
        self.url.contains("{query}")
    }

    /// The URL to open. Paths (`/…`, `~/…`) become file URLs.
    pub fn expand(&self, query: &str) -> String {
        let url = self.url.replace("{query}", &percent_encode(query));
        if let Some(rest) = url.strip_prefix("~/") {
            format!("file://{}", dirs::home_dir().unwrap_or_default().join(rest).display())
        } else if url.starts_with('/') {
            format!("file://{url}")
        } else {
            url
        }
    }
}

#[expect(clippy::format_push_string, reason = "pre-gate code; behavior frozen until flick-ea94")]
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_query_and_paths() {
        let q =
            Quicklink { name: "G".into(), url: "https://x.com/?q={query}".into(), keyword: None };
        assert!(q.takes_query());
        assert_eq!(q.expand("rust lang&co"), "https://x.com/?q=rust%20lang%26co");
        let p = Quicklink { name: "P".into(), url: "/tmp".into(), keyword: None };
        assert_eq!(p.expand(""), "file:///tmp");
    }
}
