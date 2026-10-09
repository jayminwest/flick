//! The build stamp `scripts/bundle.sh` bakes in: `FLICK_BUILD_SHA` (full sha),
//! `FLICK_BUILD_DIRTY` (`1`/`0`), `FLICK_BUILD_TIME` (RFC 3339 UTC) and `FLICK_BUILD_SOURCE`
//! (checkout path). Read with `option_env!`, so cargo rebuilds when they change. A plain
//! `cargo build` sets none of them: a dev build.

use std::path::PathBuf;

/// What this binary was built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// Full commit sha; `None` for a dev build (or bundle.sh's `unknown` outside git).
    pub sha: Option<String>,
    /// Built from a tree with uncommitted changes.
    pub dirty: bool,
    /// RFC 3339 UTC, as bundle.sh wrote it.
    pub time: Option<String>,
    /// The checkout it was built from; the crate directory for a dev build.
    pub source: PathBuf,
}

impl Stamp {
    /// The stamp compiled into this binary.
    pub fn baked() -> Stamp {
        Stamp::from_parts(
            option_env!("FLICK_BUILD_SHA"),
            option_env!("FLICK_BUILD_DIRTY"),
            option_env!("FLICK_BUILD_TIME"),
            option_env!("FLICK_BUILD_SOURCE").filter(|s| !s.is_empty()).unwrap_or(env!("CARGO_MANIFEST_DIR")),
        )
    }

    /// A stamp from raw env values. A sha that is not 7 to 40 hex digits means a dev build.
    pub fn from_parts(sha: Option<&str>, dirty: Option<&str>, time: Option<&str>, source: &str) -> Stamp {
        let sha = sha.map(str::trim).filter(|s| is_sha(s)).map(str::to_lowercase);
        let time = time.map(str::trim).filter(|t| !t.is_empty()).map(String::from);
        Stamp { sha, dirty: dirty.is_some_and(|d| d.trim() == "1"), time, source: source.into() }
    }

    /// `abc1234`, `abc1234-dirty`, or `dev`.
    pub fn short(&self) -> String {
        let sha = self.sha.as_deref().map_or("dev", |s| &s[..s.len().min(7)]);
        if self.dirty { format!("{sha}-dirty") } else { sha.to_string() }
    }

    /// `abc1234-dirty built 2026-10-09 18:01 UTC`, or `dev build` without a stamp.
    pub fn label(&self) -> String {
        match (&self.sha, &self.time) {
            (None, _) => "dev build".into(),
            (Some(_), Some(time)) => format!("{} built {}", self.short(), display_time(time)),
            (Some(_), None) => self.short(),
        }
    }
}

/// 7 to 40 hex digits.
pub fn is_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `2026-10-09T18:01:55Z` as `2026-10-09 18:01 UTC`; anything else as is.
pub fn display_time(time: &str) -> String {
    match (time.get(..10), time.get(10..11), time.get(11..16)) {
        (Some(date), Some("T"), Some(hm)) if time.ends_with('Z') => format!("{date} {hm} UTC"),
        _ => time.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn no_env_is_a_dev_build_from_the_crate() {
        let s = Stamp::from_parts(None, None, None, "/src/flick");
        assert_eq!(s.sha, None);
        assert!(!s.dirty);
        assert_eq!((s.short(), s.label()), ("dev".into(), "dev build".into()));
        assert_eq!(s.source, PathBuf::from("/src/flick"));
        // bundle.sh outside a git checkout writes `unknown`.
        assert_eq!(Stamp::from_parts(Some("unknown"), Some("0"), None, "/x").sha, None);
        assert_eq!(Stamp::from_parts(Some(""), None, Some(" "), "/x").time, None);
    }

    #[test]
    fn a_bundle_stamp_reads_sha_dirty_and_time() {
        let s = Stamp::from_parts(Some(SHA), Some("1"), Some("2026-10-09T18:01:55Z"), "/src");
        assert_eq!(s.sha.as_deref(), Some(SHA));
        assert_eq!(s.short(), "0123456-dirty");
        assert_eq!(s.label(), "0123456-dirty built 2026-10-09 18:01 UTC");
        let clean = Stamp::from_parts(Some(&SHA.to_uppercase()), Some("0"), None, "/src");
        assert_eq!(clean.sha.as_deref(), Some(SHA));
        assert_eq!(clean.label(), "0123456");
    }

    #[test]
    fn shas_are_7_to_40_hex_digits() {
        assert!(is_sha("abc1234") && is_sha(SHA));
        assert!(!is_sha("abc123") && !is_sha("dev") && !is_sha(&format!("{SHA}0")));
        assert!(!is_sha("abc123g"));
    }

    #[test]
    fn times_other_than_utc_rfc3339_show_as_is() {
        assert_eq!(display_time("2026-10-09T18:01:55Z"), "2026-10-09 18:01 UTC");
        assert_eq!(display_time("2026-10-09T18:01:55+02:00"), "2026-10-09T18:01:55+02:00");
        assert_eq!(display_time("yesterday"), "yesterday");
    }

    #[test]
    fn this_binary_has_a_stamp() {
        let s = Stamp::baked();
        assert!(s.source.is_absolute(), "{}", s.source.display());
        assert!(s.sha.is_none() || s.label().contains(&s.short()));
    }
}
