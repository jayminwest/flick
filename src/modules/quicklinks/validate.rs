//! Checks for a quicklink typed into the editor form or `flick quicklink add`. Pure.

use super::Quicklink;

/// A quicklink from raw input: every field trimmed, a blank keyword or app left out.
pub fn draft(name: &str, url: &str, keyword: &str, app: &str) -> Quicklink {
    let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    Quicklink { name: name.trim().into(), url: url.trim().into(), keyword: some(keyword), app: some(app) }
}

/// Why `link` cannot join `links`, or `Ok`. `editing` names the link it replaces, which
/// does not clash with itself.
pub fn check(links: &[Quicklink], editing: Option<&str>, link: &Quicklink) -> Result<(), String> {
    if link.name.is_empty() {
        return Err("Name is required".into());
    }
    if link.url.is_empty() {
        return Err("URL is required".into());
    }
    let others = || links.iter().filter(|q| Some(q.name.as_str()) != editing);
    // Names compare without case, so "docs" next to "Docs" is never ambiguous.
    if others().any(|q| q.name.eq_ignore_ascii_case(&link.name)) {
        return Err(format!("A quicklink named \"{}\" already exists", link.name));
    }
    if let Some(keyword) = &link.keyword {
        if keyword.contains(char::is_whitespace) {
            return Err("Keyword cannot contain spaces".into());
        }
        if let Some(q) = others().find(|q| q.keyword.as_ref() == Some(keyword)) {
            return Err(format!("Keyword \"{keyword}\" is used by {}", q.name));
        }
    }
    let url = &link.url;
    if !(url.contains("://") || url.starts_with('/') || url.starts_with("~/")) {
        return Err("URL must contain :// or be a path starting with / or ~/".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links() -> Vec<Quicklink> {
        vec![draft("Docs", "https://docs.rs/{query}", "d", ""), draft("Home", "~/", "", "")]
    }

    fn err(editing: Option<&str>, name: &str, url: &str, keyword: &str) -> String {
        check(&links(), editing, &draft(name, url, keyword, "")).unwrap_err()
    }

    #[test]
    fn draft_trims_and_drops_blank_optionals() {
        let q = draft(" Crates ", " https://crates.io ", "  ", " Safari ");
        assert_eq!((q.name.as_str(), q.url.as_str()), ("Crates", "https://crates.io"));
        assert_eq!((q.keyword, q.app.as_deref()), (None, Some("Safari")));
    }

    #[test]
    fn name_and_url_are_required() {
        assert_eq!(err(None, " ", "https://x", ""), "Name is required");
        assert_eq!(err(None, "X", "", ""), "URL is required");
    }

    #[test]
    fn names_are_unique_in_any_case() {
        let dup = "A quicklink named \"docs\" already exists";
        assert_eq!(err(None, "docs", "https://x", ""), dup);
        // The duplicate wins over a bad URL, so `quicklink add Docs x` says so.
        assert_eq!(err(None, "DOCS", "x", ""), "A quicklink named \"DOCS\" already exists");
        assert_eq!(err(Some("Home"), "docs", "/", ""), dup);
    }

    #[test]
    fn keywords_have_no_spaces_and_are_unique() {
        assert_eq!(err(None, "X", "https://x", "a b"), "Keyword cannot contain spaces");
        assert_eq!(err(None, "X", "https://x", "d"), "Keyword \"d\" is used by Docs");
        assert!(check(&links(), None, &draft("X", "https://x", "D", "")).is_ok());
    }

    #[test]
    fn urls_need_a_scheme_or_a_path() {
        let bad = "URL must contain :// or be a path starting with / or ~/";
        assert_eq!(err(None, "X", "docs.rs", ""), bad);
        assert_eq!(err(None, "X", "~docs", ""), bad);
        for url in ["https://a.b", "raycast://x", "/tmp", "~/Projects"] {
            assert!(check(&links(), None, &draft("X", url, "", "")).is_ok(), "{url}");
        }
    }

    #[test]
    fn editing_a_link_keeps_its_own_name_and_keyword() {
        let docs = draft("docs", "https://docs.rs/{query}", "d", "");
        assert!(check(&links(), Some("Docs"), &docs).is_ok());
        assert_eq!(err(Some("Home"), "Home", "/", "d"), "Keyword \"d\" is used by Docs");
    }
}
