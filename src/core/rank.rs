//! Fuzzy ranking plus frecency.

use std::collections::HashMap;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};

use super::Item;

/// Usage counts and last-use timestamps (unix seconds), keyed by item id.
pub type Usage = HashMap<String, (u32, i64)>;

/// Bonus for frequently and recently used items. Usage decays with a 14-day half-life.
pub fn frecency(usage: &Usage, id: &str, now: i64) -> f64 {
    let Some(&(count, last)) = usage.get(id) else { return 0.0 };
    let age_days = (now - last).max(0) as f64 / 86_400.0;
    let weight = f64::from(count) * 0.5f64.powf(age_days / 14.0);
    25.0 * (1.0 + weight).ln()
}

pub struct Ranker {
    matcher: Matcher,
    buf: Vec<char>,
}

impl Ranker {
    pub fn new() -> Self {
        Ranker { matcher: Matcher::new(nucleo_matcher::Config::DEFAULT), buf: Vec::new() }
    }

    fn score(&mut self, pattern: &Pattern, text: &str) -> Option<u32> {
        pattern.score(Utf32Str::new(text, &mut self.buf), &mut self.matcher)
    }

    /// Filter and sort `items` for `query`. An empty query sorts by `bonus`, then title.
    pub fn rank(
        &mut self,
        query: &str,
        items: Vec<Item>,
        bonus: impl Fn(&Item) -> f64,
    ) -> Vec<Item> {
        let query = query.trim();
        let mut scored: Vec<(f64, Item)> = if query.is_empty() {
            items.into_iter().map(|i| (bonus(&i), i)).collect()
        } else {
            let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
            items
                .into_iter()
                .filter_map(|item| {
                    let title = self.score(&pattern, &item.title).map(f64::from);
                    let keyword = item
                        .keywords
                        .iter()
                        .filter_map(|k| self.score(&pattern, k))
                        .max()
                        .map(|s| f64::from(s) * 0.8);
                    let best = title
                        .into_iter()
                        .chain(keyword)
                        .fold(None, |a: Option<f64>, s| Some(a.map_or(s, |a| a.max(s))))?;
                    Some((best + bonus(&item), item))
                })
                .collect()
        };
        // Ties: shorter titles first when searching (closer match), else alphabetical.
        let by_len = !query.is_empty();
        scored.sort_by(|(a, x), (b, y)| {
            b.total_cmp(a)
                .then(if by_len {
                    x.title.len().cmp(&y.title.len())
                } else {
                    std::cmp::Ordering::Equal
                })
                .then_with(|| x.title.to_lowercase().cmp(&y.title.to_lowercase()))
        });
        scored.into_iter().map(|(_, i)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Icon, ItemId};

    fn item(title: &str) -> Item {
        Item::new(ItemId::new("test", title), title, "Run", Icon::Symbol("app"))
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.title.as_str()).collect()
    }

    #[test]
    fn fuzzy_filters_and_ranks_prefix_first() {
        let items = vec![item("Safari"), item("System Settings"), item("Slack"), item("Notes")];
        let out = Ranker::new().rank("sa", items, |_| 0.0);
        assert_eq!(out[0].title, "Safari");
        assert!(!titles(&out).contains(&"Notes"));
    }

    #[test]
    fn acronym_matches() {
        let items = vec![item("Visual Studio Code"), item("Notes")];
        let out = Ranker::new().rank("vsc", items, |_| 0.0);
        assert_eq!(titles(&out), ["Visual Studio Code"]);
    }

    #[test]
    fn bonus_breaks_ties_and_orders_empty_query() {
        let items = vec![item("Slack"), item("Spotify")];
        let out = Ranker::new()
            .rank("", items.clone(), |i| if i.title == "Spotify" { 10.0 } else { 0.0 });
        assert_eq!(titles(&out), ["Spotify", "Slack"]);
        let out = Ranker::new().rank("", vec![item("Zed"), item("Apps")], |_| 0.0);
        assert_eq!(titles(&out), ["Apps", "Zed"]);
        let out = Ranker::new().rank("s", items, |i| if i.title == "Spotify" { 50.0 } else { 0.0 });
        assert_eq!(out[0].title, "Spotify");
    }

    #[test]
    fn keywords_match() {
        let mut i = item("Left Half");
        i.keywords = vec!["snap".into()];
        let out = Ranker::new().rank("snap", vec![i, item("Notes")], |_| 0.0);
        assert_eq!(titles(&out), ["Left Half"]);
    }

    #[test]
    fn frecency_decays() {
        let mut usage = Usage::new();
        usage.insert("a".into(), (10, 0));
        let fresh = frecency(&usage, "a", 0);
        let old = frecency(&usage, "a", 60 * 86_400);
        assert!(fresh > old && old > 0.0);
        assert_eq!(frecency(&usage, "missing", 0), 0.0);
    }
}
