//! Root search: the items every launch lists and their ranking. Pure Rust, no `AppKit`.

use crate::apps::App;
use crate::config::Quicklink;
use crate::search::{Action, Icon, Item, Ranker, Usage, frecency};
use crate::windows::WindowAction;

/// Every root-search item: apps, window commands, quicklinks, builtins. Item ids feed the
/// `usage` table, so their format is pinned by `crate::characterization`.
pub fn root_items(apps: &[App], quicklinks: &[Quicklink]) -> Vec<Item> {
    let mut items: Vec<Item> = apps
        .iter()
        .map(|a| Item {
            id: format!("app:{}", a.path.display()),
            title: a.name.clone(),
            subtitle: String::new(),
            accessory: "Application".into(),
            icon: Icon::File(a.path.clone()),
            action: Action::LaunchApp(a.path.clone()),
            keywords: vec![],
        })
        .collect();

    items.extend(WindowAction::ALL.iter().map(|&w| Item {
        id: format!("window:{}", w.title()),
        title: w.title().into(),
        subtitle: "Window Management".into(),
        accessory: "Command".into(),
        icon: Icon::Symbol(w.symbol()),
        action: Action::Window(w),
        keywords: vec!["window".into()],
    }));

    items.extend(quicklinks.iter().enumerate().map(|(index, q)| Item {
        id: format!("quicklink:{}", q.name),
        title: q.name.clone(),
        subtitle: q.keyword.clone().unwrap_or_default(),
        accessory: "Quicklink".into(),
        icon: Icon::Symbol("link"),
        action: Action::Quicklink { index, query: (!q.takes_query()).then(String::new) },
        keywords: q.keyword.iter().cloned().collect(),
    }));

    let builtins = [
        ("Clipboard History", "doc.on.clipboard", Action::ClipboardHistory, "paste"),
        ("Switch Windows", "macwindow.on.rectangle", Action::SwitchWindows, "focus alt tab"),
        ("Open Flick Config", "gearshape", Action::OpenConfig, "settings preferences"),
        ("Reload Flick Config", "arrow.clockwise", Action::ReloadConfig, "refresh"),
        ("Quit Flick", "power", Action::Quit, "exit"),
    ];
    items.extend(builtins.into_iter().map(|(title, symbol, action, keywords)| Item {
        id: format!("builtin:{title}"),
        title: title.into(),
        subtitle: "Flick".into(),
        accessory: "Command".into(),
        icon: Icon::Symbol(symbol),
        action,
        keywords: vec![keywords.into()],
    }));
    items
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
    let mut results = ranker.rank(query, items, |i| frecency(usage, &i.id, now));
    // "<keyword> <text>" runs a quicklink directly.
    if let Some((keyword, rest)) = query.split_once(' ') {
        let link = quicklinks.iter().enumerate().find(|(_, q)| {
            q.takes_query() && q.keyword.as_deref() == Some(keyword) && !rest.trim().is_empty()
        });
        if let Some((index, q)) = link {
            results.insert(
                0,
                Item {
                    id: format!("quicklink:{}", q.name),
                    title: q.name.clone(),
                    subtitle: format!("“{}”", rest.trim()),
                    accessory: "Quicklink".into(),
                    icon: Icon::Symbol("link"),
                    action: Action::Quicklink { index, query: Some(rest.trim().to_string()) },
                    keywords: vec![],
                },
            );
        }
    }
    results
}
