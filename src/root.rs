//! Root search: modules' items ranked by fuzzy score plus frecency, below their direct
//! items. Pure Rust, no `AppKit`.

#[cfg(test)]
use crate::config::Quicklink;
use crate::core::{Item, Ranker, Usage, frecency};

/// Rank root `items` for `query` by fuzzy score plus frecency, then put `direct` items
/// (e.g. "<keyword> <text>" for a quicklink) first, in order.
pub fn rank(
    ranker: &mut Ranker,
    query: &str,
    items: Vec<Item>,
    direct: Vec<Item>,
    usage: &Usage,
    now: i64,
) -> Vec<Item> {
    let mut results = ranker.rank(query, items, |i| frecency(usage, i.id.as_str(), now));
    results.splice(0..0, direct);
    results
}

/// Every root-search item for `apps` and `quicklinks`, through the real registry.
#[cfg(test)]
pub fn root_items(apps: &[crate::modules::apps::App], quicklinks: &[Quicklink]) -> Vec<Item> {
    crate::core::test_cx("", config(quicklinks), |cx| {
        crate::modules::with_apps(apps.to_vec()).items(cx)
    })
}

/// `rank` with the real registry's direct items for `query` and `quicklinks`.
#[cfg(test)]
pub fn rank_root(
    ranker: &mut Ranker,
    query: &str,
    items: Vec<Item>,
    quicklinks: &[Quicklink],
    usage: &Usage,
    now: i64,
) -> Vec<Item> {
    let direct = crate::core::test_cx(query, config(quicklinks), |cx| {
        crate::modules::with_apps(vec![]).direct(cx)
    });
    rank(ranker, query, items, direct, usage, now)
}

#[cfg(test)]
fn config(quicklinks: &[Quicklink]) -> crate::config::Config {
    crate::config::Config { quicklinks: quicklinks.to_vec(), ..Default::default() }
}
