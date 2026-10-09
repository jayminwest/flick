//! Launcher ranking: fuzzy score plus frecency over the real root items.

use crate::config::Config;
use crate::core::{Ranker, Usage};
use crate::modules::apps::App;
use crate::root::{rank_root, root_items};

const NOW: i64 = 1_700_000_000;
const DAY: i64 = 86_400;

fn apps() -> Vec<App> {
    [
        ("Safari", "/Applications/Safari.app"),
        ("Slack", "/Applications/Slack.app"),
        ("Spotify", "/Applications/Spotify.app"),
        ("System Settings", "/System/Applications/System Settings.app"),
        ("Visual Studio Code", "/Applications/Visual Studio Code.app"),
        ("Notes", "/System/Applications/Notes.app"),
        ("Finder", "/System/Library/CoreServices/Finder.app"),
    ]
    .into_iter()
    .map(|(name, path)| App { name: name.into(), path: path.into() })
    .collect()
}

fn usage() -> Usage {
    [
        ("app:/Applications/Slack.app", 30, NOW - DAY),
        ("app:/Applications/Spotify.app", 3, NOW),
        ("app:/Applications/Safari.app", 50, NOW - 60 * DAY),
        ("window:Left Half", 12, NOW - 2 * DAY),
        ("builtin:Clipboard History", 5, NOW),
        ("quicklink:Google", 8, NOW - 7 * DAY),
    ]
    .into_iter()
    .map(|(id, count, last)| (id.to_string(), (count, last)))
    .collect()
}

fn ranked(query: &str, take: usize) -> Vec<String> {
    let links = Config::default().quicklinks;
    let items = root_items(&apps(), &links);
    rank_root(&mut Ranker::new(), query, items, &links, &usage(), NOW)
        .into_iter()
        .take(take)
        .map(|i| i.id.to_string())
        .collect()
}

#[test]
fn empty_query_orders_by_frecency_then_title() {
    assert_eq!(
        ranked("", 8),
        [
            "app:/Applications/Slack.app",
            "window:Left Half",
            "quicklink:Google",
            "builtin:Clipboard History",
            "app:/Applications/Spotify.app",
            "app:/Applications/Safari.app",
            "window:Almost Maximize",
            "window:Bottom Half",
        ]
    );
}

#[test]
fn fuzzy_score_plus_frecency() {
    assert_eq!(
        ranked("s", 8),
        [
            "app:/Applications/Slack.app",
            "app:/Applications/Spotify.app",
            "app:/Applications/Safari.app",
            "builtin:Clipboard History",
            "quicklink:GitHub Search",
            "builtin:Switch Windows",
            "app:/System/Applications/System Settings.app",
            "app:/Applications/Visual Studio Code.app",
        ]
    );
    assert_eq!(
        ranked("sa", 5),
        [
            "app:/Applications/Slack.app",
            "app:/Applications/Safari.app",
            "quicklink:GitHub Search",
            "builtin:Switch Windows",
            "app:/Applications/Visual Studio Code.app",
        ]
    );
    assert_eq!(ranked("vsc", 8), ["app:/Applications/Visual Studio Code.app"]);
    assert_eq!(
        ranked("left", 8),
        ["window:Left Half", "window:Top Left Quarter", "window:Bottom Left Quarter"]
    );
    assert_eq!(ranked("clip", 8), ["builtin:Clipboard History"]);
    assert!(ranked("zzz", 8).is_empty());
}

#[test]
fn keywords_match_quicklinks() {
    assert_eq!(
        ranked("g", 5),
        [
            "quicklink:Google",
            "quicklink:GitHub Search",
            "window:Right Half",
            "app:/System/Applications/System Settings.app",
            "builtin:Open Flick Config",
        ]
    );
    assert_eq!(
        ranked("gh", 8),
        [
            "quicklink:GitHub Search",
            "window:Right Half",
            "window:Top Right Quarter",
            "window:Bottom Right Quarter",
        ]
    );
}

#[test]
fn keyword_and_text_runs_the_quicklink_first() {
    let links = Config::default().quicklinks;
    let items = root_items(&apps(), &links);
    let out = rank_root(&mut Ranker::new(), "gh  flick ", items, &links, &usage(), NOW);
    assert_eq!(out[0].id, "quicklink:GitHub Search");
    assert_eq!(out[0].subtitle, "\u{201c}flick\u{201d}");
    assert_eq!(out[0].id.arg(), Some("flick"));
    assert_eq!(ranked("g rust lang", 8), ["quicklink:Google"]);
    // A keyword with no text after it ranks normally: no extra quicklink item.
    let plain = ranked("g ", 99);
    assert_eq!(plain.iter().filter(|id| *id == "quicklink:Google").count(), 1);
    assert_eq!(plain, ranked("g", 99));
}
