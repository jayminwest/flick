use super::*;

/// A regular window of `bundle`: standard subrole, enabled fullscreen button, focused.
fn window(bundle: &str) -> Facts<'_> {
    Facts {
        bundle,
        subrole: STANDARD,
        fullscreen: Button::Enabled,
        buttons: true,
        focused: true,
        normal_level: true,
    }
}

fn ws(n: usize) -> Ws {
    Ws::new(n).unwrap()
}

/// The user's config: Slack on 5, Spotify on 6.
fn rules() -> Rules {
    Rules::new(
        vec![("com.tinyspeck.slackmacgap".into(), ws(5)), ("com.spotify.client".into(), ws(6))],
        vec!["com.example.float".into()],
    )
}

#[test]
fn pinned_apps_open_on_their_workspace() {
    let r = rules();
    assert_eq!(r.pin("com.tinyspeck.slackmacgap"), Some(ws(5)));
    assert_eq!(r.pin("com.spotify.client"), Some(ws(6)));
    assert_eq!(r.pin("com.apple.Safari"), None);
    assert_eq!(Rules::default().pin("com.spotify.client"), None);
}

#[test]
fn regular_windows_tile() {
    assert_eq!(rules().kind(&window("com.apple.Safari")), Kind::Tile);
    assert_eq!(rules().kind(&window("")), Kind::Tile, "an app without a bundle id");
}

#[test]
fn flick_system_settings_and_configured_apps_float() {
    let r = rules();
    for bundle in ["com.jayminwest.flick", "com.apple.systempreferences", "com.example.float"] {
        assert_eq!(r.kind(&window(bundle)), Kind::Float, "{bundle}");
    }
    assert_eq!(Rules::default().kind(&window("com.example.float")), Kind::Tile);
}

#[test]
fn dialogs_float() {
    let r = rules();
    let picker = Facts { subrole: "AXDialog", ..window("com.apple.Safari") };
    assert_eq!(r.kind(&picker), Kind::Float, "file picker");
    let about = Facts { fullscreen: Button::Missing, ..window("com.apple.finder") };
    assert_eq!(r.kind(&about), Kind::Float, "no fullscreen button");
    let login = Facts { fullscreen: Button::Disabled, ..window("com.apple.Safari") };
    assert_eq!(r.kind(&login), Kind::Float, "fullscreen button disabled");
}

#[test]
fn apps_that_hide_their_fullscreen_button_still_tile() {
    for bundle in NO_FULLSCREEN_BUTTON {
        let f = Facts { fullscreen: Button::Missing, ..window(bundle) };
        assert_eq!(rules().kind(&f), Kind::Tile, "{bundle}");
    }
}

#[test]
fn qutebrowser_tiles_with_a_dialog_subrole_but_its_real_dialogs_do_not_slip_through() {
    let r = rules();
    let bare = Facts { subrole: "AXDialog", ..window("org.qutebrowser.qutebrowser") };
    assert_eq!(r.kind(&bare), Kind::Tile);
    let chrome_find = Facts { subrole: "AXUnknown", ..window("com.google.Chrome") };
    assert_eq!(r.kind(&chrome_find), Kind::Float, "the fullscreen exception is not a subrole one");
}

#[test]
fn popups_are_not_managed() {
    let r = rules();
    let tooltip = Facts {
        subrole: "AXUnknown",
        fullscreen: Button::Missing,
        buttons: false,
        focused: false,
        normal_level: false,
        ..window("com.jayminwest.flick")
    };
    assert_eq!(r.kind(&tooltip), Kind::Popup, "even for an app that floats");
    assert_eq!(r.kind(&Facts { focused: true, ..tooltip }), Kind::Float, "focused: a dialog");
    assert_eq!(r.kind(&Facts { buttons: true, ..tooltip }), Kind::Float, "buttons: a dialog");
    assert_eq!(r.kind(&Facts { normal_level: true, ..tooltip }), Kind::Float, "normal level");
}
