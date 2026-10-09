//! Import quicklinks from Raycast's "Export Quicklinks" JSON.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{self, Quicklink};

#[derive(Deserialize)]
struct RaycastLink {
    name: String,
    link: String,
}

/// Rewrite Raycast's `{argument …}` and `{query}` placeholders as `{query}`.
/// Returns the link and any placeholders Flick can't fill (e.g. `{clipboard}`).
fn convert_link(link: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut unsupported = Vec::new();
    let mut rest = link;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else { break };
        let inner = rest[start + 1..start + len].trim();
        out.push_str(&rest[..start]);
        if inner.starts_with("argument") || inner.eq_ignore_ascii_case("query") {
            out.push_str("{query}");
        } else {
            out.push_str(&rest[start..=start + len]);
            unsupported.push(format!("{{{inner}}}"));
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    (out, unsupported)
}

pub struct Import {
    pub added: Vec<Quicklink>,
    pub duplicates: usize,
    pub warnings: Vec<String>,
}

/// Convert `json`, skipping links whose name or URL is already in `existing` (or earlier in the file).
fn convert(json: &str, existing: &[Quicklink]) -> Result<Import, String> {
    let links: Vec<RaycastLink> =
        serde_json::from_str(json).map_err(|e| format!("not a Raycast quicklinks export: {e}"))?;
    let mut seen: Vec<(String, String)> =
        existing.iter().map(|q| (q.name.to_lowercase(), q.url.clone())).collect();
    let mut import = Import { added: vec![], duplicates: 0, warnings: vec![] };
    for link in links {
        let (url, unsupported) = convert_link(&link.link);
        let name = link.name.trim().to_string();
        if seen.iter().any(|(n, u)| *n == name.to_lowercase() || *u == url) {
            import.duplicates += 1;
            continue;
        }
        if !unsupported.is_empty() {
            import.warnings.push(format!("{name}: unsupported {}", unsupported.join(", ")));
        }
        seen.push((name.to_lowercase(), url.clone()));
        import.added.push(Quicklink { name, url, keyword: None });
    }
    Ok(import)
}

/// Append the quicklinks in Raycast export `path` to the config file.
#[expect(
    clippy::items_after_statements,
    clippy::format_push_string,
    reason = "pre-gate code; behavior frozen until flick-ea94"
)]
pub fn import_file(path: &Path) -> Result<String, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let config = config::load()?;
    let import = convert(&json, &config.quicklinks)?;

    #[derive(Serialize)]
    struct Section<'a> {
        quicklinks: &'a [Quicklink],
    }
    let config_path = config::config_path();
    if !import.added.is_empty() {
        let toml =
            toml::to_string(&Section { quicklinks: &import.added }).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&config_path)
            .map_err(|e| format!("{}: {e}", config_path.display()))?;
        write!(file, "\n# Imported from Raycast\n{toml}").map_err(|e| e.to_string())?;
    }

    let mut report = format!(
        "Added {} quicklinks to {} ({} duplicates skipped).",
        import.added.len(),
        config_path.display(),
        import.duplicates
    );
    for w in &import.warnings {
        report.push_str(&format!("\n  warning: {w}"));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_placeholders() {
        assert_eq!(convert_link("https://x.com/?q={argument}").0, "https://x.com/?q={query}");
        assert_eq!(
            convert_link(r#"https://x.com/{argument name="repo" default="a"}"#).0,
            "https://x.com/{query}"
        );
        assert_eq!(convert_link("https://x.com/?q={Query}").0, "https://x.com/?q={query}");
        let (url, unsupported) = convert_link("https://x.com/?q={clipboard}");
        assert_eq!(
            (url.as_str(), unsupported),
            ("https://x.com/?q={clipboard}", vec!["{clipboard}".to_string()])
        );
        assert_eq!(convert_link("http://localhost:3000/").0, "http://localhost:3000/");
    }

    #[test]
    fn skips_duplicates_by_name_or_url() {
        let existing =
            vec![Quicklink { name: "Docs".into(), url: "https://docs.rs".into(), keyword: None }];
        let json = r#"[
            {"name": "docs", "link": "https://other.com"},
            {"name": "Rust Docs", "link": "https://docs.rs"},
            {"name": "Open WebUI", "link": "http://host:3000/", "icon": "link"},
            {"name": "Open-WebUI", "link": "http://host:3000/"},
            {"name": "Search", "link": "https://duckduckgo.com/?q={argument}"}
        ]"#;
        let import = convert(json, &existing).unwrap();
        let names: Vec<_> = import.added.iter().map(|q| q.name.as_str()).collect();
        assert_eq!(names, ["Open WebUI", "Search"]);
        assert_eq!(import.duplicates, 3);
        assert!(import.added[1].takes_query());
    }
}
