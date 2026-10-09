//! Root search: the registered modules and the ranking of their items. Pure Rust, no `AppKit`.

use crate::apps::{App, Apps};
use crate::builtins::Builtins;
use crate::clipboard::Clipboard;
use crate::config::Quicklink;
use crate::core::{Item, Ranker, Registry, Usage, frecency};
use crate::quicklinks::{Quicklinks, keyword_item};
use crate::switcher::Switcher;
use crate::windows;

/// Every module, in root-search order: apps, window commands, quicklinks, builtins. Item
/// ids feed the `usage` table, so their format is pinned by `crate::characterization`.
pub fn registry(apps: Vec<App>) -> Registry {
    Registry::new(vec![
        Box::new(Apps::new(apps)),
        Box::new(windows::Commands),
        Box::new(Quicklinks),
        Box::new(Builtins),
        Box::new(Clipboard),
        Box::new(Switcher::default()),
    ])
}

/// Every root-search item for `apps` and `quicklinks`, through the real registry.
#[cfg(test)]
pub fn root_items(apps: &[App], quicklinks: &[Quicklink]) -> Vec<Item> {
    let config = crate::config::Config { quicklinks: quicklinks.to_vec(), ..Default::default() };
    crate::core::test_cx("", config, |cx| registry(apps.to_vec()).items(cx))
}

/// Rank root `items` for `query` by fuzzy score plus frecency; "<keyword> <text>" puts the
/// matching quicklink first.
pub fn rank_root(
    ranker: &mut Ranker,
    query: &str,
    items: Vec<Item>,
    quicklinks: &[Quicklink],
    usage: &Usage,
    now: i64,
) -> Vec<Item> {
    let mut results = ranker.rank(query, items, |i| frecency(usage, i.id.as_str(), now));
    // "<keyword> <text>" runs a quicklink directly.
    if let Some((keyword, rest)) = query.split_once(' ') {
        let link = quicklinks.iter().find(|q| {
            q.takes_query() && q.keyword.as_deref() == Some(keyword) && !rest.trim().is_empty()
        });
        if let Some(q) = link {
            results.insert(0, keyword_item(q, rest.trim()));
        }
    }
    results
}
