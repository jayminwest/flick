//! The capture module's root items, the Recent Captures view (`capture/recent`) and its
//! action menu.

use std::collections::HashMap;
use std::path::Path;

use crate::core::{Action, Confirm, ConfirmRow, Cx, Icon, Item, ItemId, ListView};
use crate::store::{Store, now};

use super::store::{Row, Shots};

/// The Recent Captures view's name.
pub const RECENT: &str = "recent";
/// Rows the view shows.
const RECENT_MAX: u32 = 200;
/// Key prefix of a recorded shot: `capture:shot/<row id>`.
const SHOT: &str = "shot/";

/// Capture Area, Capture Window, Capture Screen, then `ink` (annotation and drawing), then
/// Recent Captures.
pub fn root_items(dir: &Path, ink: Vec<Item>) -> Vec<Item> {
    let dir = tilde(dir);
    let rows = [
        ("area", "Capture Area", "rectangle.dashed", "Drag to select; Space picks a window"),
        ("window", "Capture Window", "macwindow", "Click a window"),
        ("screen", "Capture Screen", "display", "The display under the mouse"),
    ];
    let mut items: Vec<Item> = rows
        .into_iter()
        .map(|(key, title, symbol, how)| Item {
            subtitle: format!("{how}, save to {dir}"),
            accessory: "Capture".into(),
            keywords: vec!["screenshot screen shot snap shottr".into()],
            ..Item::new(ItemId::new("capture", key), title, "Capture", Icon::Symbol(symbol))
        })
        .collect();
    items.extend(ink);
    items.push(Item {
        accessory: "Capture".into(),
        keywords: vec!["screenshots history".into()],
        ..Item::new(ItemId::new("capture", "recent"), "Recent Captures", "Open", Icon::Symbol("photo.on.rectangle"))
    });
    items
}

pub fn recent() -> ListView {
    ListView {
        placeholder: "Search recent captures…".into(),
        footer: "Recent Captures  ·  esc to go back".into(),
        ..ListView::new("capture", RECENT)
    }
}

/// Rows whose file still exists, newest first. Rows whose file is gone are deleted.
pub fn live_shots(store: &Store, limit: u32) -> Vec<Row> {
    let (live, gone): (Vec<Row>, Vec<Row>) =
        store.shots(limit).into_iter().partition(|r| Path::new(&r.path).exists());
    for row in gone {
        store.delete_shot(row.id);
    }
    live
}

/// One row per recorded shot: file name, size and folder, age.
pub fn refresh(view: &mut ListView, cx: &mut Cx) {
    let items: Vec<Item> = live_shots(cx.store, RECENT_MAX)
        .into_iter()
        .map(|r| {
            let path = Path::new(&r.path);
            let file = path.file_name().map_or_else(|| r.path.clone(), |f| f.to_string_lossy().into_owned());
            let dir = path.parent().map(tilde).unwrap_or_default();
            Item {
                subtitle: format!("{}×{}  ·  {dir}", r.width, r.height),
                accessory: age(now() - r.taken),
                keywords: vec![r.kind.clone()],
                ..Item::new(ItemId::new("capture", format!("{SHOT}{}", r.id)), file, "Open", Icon::File(path.to_path_buf()))
            }
        })
        .collect();
    // Bonus keeps newest-first order for equal scores.
    let order: HashMap<String, usize> =
        items.iter().enumerate().map(|(i, item)| (item.id.to_string(), i)).collect();
    view.items = cx.ranker.rank(cx.query, items, |i| -(order[i.id.as_str()] as f64) * 1e-3);
    view.empty = if cx.query.is_empty() { "No captures yet" } else { "No Results" }.into();
}

/// The row id of key `shot/<id>`.
pub fn shot_id(key: &str) -> Option<i64> {
    key.strip_prefix(SHOT)?.parse().ok()
}

/// A shot's action menu.
pub fn actions() -> Vec<Action> {
    vec![
        Action::new("copy-image", "Copy Image", Icon::Symbol("doc.on.doc")),
        Action::new("annotate", "Annotate", Icon::Symbol("pencil.and.outline")),
        Action::new("reveal", "Show in Finder", Icon::Symbol("folder")),
        Action::new("copy-path", "Copy Path", Icon::Symbol("link")),
        Action::new("trash", "Move to Trash", Icon::Symbol("trash")),
    ]
}

/// Ask before moving `row`'s file to the Trash.
pub fn confirm_trash(row: &Row) -> Confirm {
    let path = Path::new(&row.path);
    let file = path.file_name().map_or_else(|| row.path.clone(), |f| f.to_string_lossy().into_owned());
    Confirm {
        rows: vec![ConfirmRow { subtitle: row.path.clone(), ..ConfirmRow::new(file.clone()) }],
        label: "Move to Trash".into(),
        destructive: true,
        ..Confirm::new("capture", format!("trash/{}", row.id), format!("Move {file} to the Trash?"))
    }
}

/// `secs` ago, coarsely.
fn age(secs: i64) -> String {
    match secs.max(0) {
        0..60 => "Just now".into(),
        s @ 60..3600 => format!("{}m ago", s / 60),
        s @ 3600..86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// `path` with the home directory shown as `~`.
fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_is_coarse() {
        assert_eq!(age(-5), "Just now");
        assert_eq!(age(59), "Just now");
        assert_eq!(age(120), "2m ago");
        assert_eq!(age(7200), "2h ago");
        assert_eq!(age(3 * 86_400), "3d ago");
    }

    #[test]
    fn shot_keys_round_trip() {
        assert_eq!(shot_id("shot/42"), Some(42));
        assert_eq!(shot_id("shot/x"), None);
        assert_eq!(shot_id("area"), None);
    }

    #[test]
    fn tilde_shows_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(tilde(&home), "~");
        assert_eq!(tilde(&home.join("Pictures/Flick")), "~/Pictures/Flick");
        assert_eq!(tilde(Path::new("/tmp/x")), "/tmp/x");
    }
}
