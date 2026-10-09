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

/// Percent-encode every byte outside RFC 3986's unreserved set, uppercase hex.
fn percent_encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[usize::from(b >> 4)]));
                out.push(char::from(HEX[usize::from(b & 0xF)]));
            }
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

    #[test]
    fn percent_encodes_all_but_unreserved_bytes() {
        assert_eq!(percent_encode("Az09-_.~"), "Az09-_.~");
        assert_eq!(percent_encode(" /?#%+\n"), "%20%2F%3F%23%25%2B%0A");
        assert_eq!(percent_encode("é→"), "%C3%A9%E2%86%92");
        assert_eq!(percent_encode(""), "");
    }
}
