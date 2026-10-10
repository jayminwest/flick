//! What Flick Help says: one topic per feature, with a one-line how-to and the `docs/`
//! page that explains it. Data only; the draw and annotate keys come from
//! `platform::ink::legend`, the same list the on-screen legend shows.

use crate::platform::ink::legend::{self, Key, Mode};

/// The `docs/` folder on GitHub; a topic's `doc` picks the page (and section).
pub const DOCS: &str = "https://github.com/jayminwest/flick/blob/main/docs/";

pub struct Topic {
    /// Part of the item id (`help:topic/<slug>`): permanent once released.
    pub slug: &'static str,
    pub title: &'static str,
    pub how: &'static str,
    pub symbol: &'static str,
    /// The page under `docs/`, with an optional `#section`.
    pub doc: &'static str,
    /// The topic's keys get their own view (`keys/<slug>`).
    pub keys: Option<Mode>,
}

const fn topic(
    slug: &'static str,
    title: &'static str,
    how: &'static str,
    symbol: &'static str,
    doc: &'static str,
) -> Topic {
    Topic { slug, title, how, symbol, doc, keys: None }
}

/// Every topic, in the order the help view lists them.
pub const TOPICS: [Topic; 24] = [
    topic("launcher", "Launcher", "Type to search apps and commands. ⌃J ⌃K move, ↵ runs, ⌘K shows actions, esc goes back.", "magnifyingglass", "configuration.md#launcher-keys"),
    topic("windows", "Window Commands", "Type a command such as Left Half, or bind one to a hotkey in [window.keys].", "rectangle.split.2x1", "configuration.md"),
    topic("switcher", "Window Switcher", "Switch Windows lists open windows. The first is the previous one, so ↵ jumps back.", "macwindow.on.rectangle", "configuration.md"),
    topic("desktop", "Desktop Toggle", "One hotkey ([desktop] hotkey) jumps to the most recent app on another desktop.", "rectangle.2.swap", "configuration.md"),
    topic("quicklinks", "Quicklinks", "Type <keyword> <text> to open a link with your text. Create Quicklink adds one.", "link", "configuration.md#quicklinks"),
    topic("clipboard", "Clipboard History", "The last 500 copied texts. ↵ pastes the selected one into the previous app.", "doc.on.clipboard", "configuration.md"),
    topic("capture", "Screenshots", "Capture Area, Window or Screen saves a PNG and copies it. Recent Captures lists them.", "camera.viewfinder", "capture.md"),
    Topic {
        keys: Some(Mode::Editor { copy: true }),
        ..topic("annotate", "Annotate a Screenshot", "Capture Area and Annotate opens the editor. Keys pick the tool and color; ↵ saves. ? shows the keys.", "pencil.tip.crop.circle", "capture.md")
    },
    Topic {
        keys: Some(Mode::Overlay),
        ..topic("draw", "Draw on Screen", "Starts with the pen. 1-5 change color, A R P H T X change tool, ⌘Z undoes, ↵ keeps, esc clears.", "scribble", "capture.md")
    },
    topic("cursor", "Highlight Cursor", "A ring follows the pointer and pulses on clicks. Run Stop Highlighting Cursor to end it.", "cursorarrow.rays", "capture.md"),
    topic("activity", "Activity", "Start Activity Recording tracks the app in front. Activity Today shows where the day went.", "clock", "activity.md"),
    topic("tasks", "Tasks", "Start Task runs a timer on one task at a time. Tasks Today lists the time per task.", "play.circle", "tasks.md"),
    topic("herdr", "Herdr Agents", "Coding agents in herdr, waiting ones first. ↵ jumps to the agent's pane.", "terminal", "herdr.md"),
    topic("kota", "KOTA", "The K in the menu bar is KOTA's state; the number is cards waiting on you. Ask KOTA… sends a question.", "k.circle", "kota.md"),
    topic("fleet", "Fleet", "Your Macs and their services from [[sys.machine]]. ⌘K: Screen Sharing, Open Dash, Tail Log, Restart….", "server.rack", "sys.md"),
    topic("keys", "Key Triggers", "Caps Lock as Hyper, and key chords that run an action. Set them up in [keys].", "keyboard", "keys.md"),
    topic("dictation", "Dictation", "Hold the dictation chord, speak, release: the text goes into the focused field. Runs on this Mac only.", "mic", "dictation.md"),
    topic("feedback", "Feedback", "Type fb <text> to save a note about Flick. Recent Feedback lists your notes.", "text.bubble", "feedback.md"),
    topic("message", "Messages and Cards", "Agents such as KOTA post messages and cards to a corner. card_hotkey moves the keyboard into the newest card.", "bubble.left.and.bubble.right", "message.md"),
    topic("chat", "KOTA Chat", "chat_hotkey opens a chat with KOTA. ⌘N new thread, ⌘[ ⌘] switch, ⌘⇧V clipboard, ⌘⇧S screenshot.", "bubble.left.and.text.bubble.right", "message.md#chat"),
    topic("llm", "Local Model Chat", "Chat with models on your own servers: add [[llm.servers]], then open Local Model Chat. ⌘. stops a reply.", "brain", "local-llm.md"),
    topic("remote", "Remote Access", "Lets Macs in your tailnet run flick commands here. Off until you turn it on.", "network", "remote.md"),
    topic("config", "Configuration", "Open Flick Config edits config.toml; Reload Flick Config applies it.", "gearshape", "configuration.md"),
    topic("cli", "Command Line", "flick <module> <verb> asks the running Flick; flick help lists the verbs.", "apple.terminal", "cli.md"),
];

/// The topic named `slug`.
pub fn find(slug: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.slug == slug)
}

/// The docs page for `topic`.
pub fn url(topic: &Topic) -> String {
    format!("{DOCS}{}", topic.doc)
}

/// The keys a topic's key view lists; none for a topic without keys.
pub fn keys(topic: &Topic) -> Vec<Key> {
    topic.keys.map(legend::keys).unwrap_or_default()
}

/// `topic` as plain text: title, how-to, then each key on its own line.
pub fn text(topic: &Topic) -> String {
    let head = format!("{}: {}", topic.title, topic.how);
    let keys = keys(topic).into_iter().map(|k| format!("  {}  {}", k.press, k.long));
    std::iter::once(head).chain(keys).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_unique_and_found() {
        for (i, t) in TOPICS.iter().enumerate() {
            assert!(TOPICS[..i].iter().all(|o| o.slug != t.slug), "{}", t.slug);
            assert!(find(t.slug).is_some_and(|f| f.title == t.title));
            assert!(url(t).starts_with(DOCS) && t.doc.contains(".md"));
        }
        assert!(find("nope").is_none());
        // `topic` runs at compile time for TOPICS; once here so coverage sees it.
        assert_eq!(topic("s", "t", "h", "y", "a").keys, None);
    }

    #[test]
    fn every_topic_page_exists() {
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
        for t in &TOPICS {
            let page = t.doc.split('#').next().unwrap_or_default();
            assert!(docs.join(page).is_file(), "{}: docs/{page} is missing", t.slug);
        }
        for slug in ["kota", "fleet", "dictation"] {
            assert!(find(slug).is_some(), "{slug}");
        }
    }

    #[test]
    fn draw_and_annotate_list_their_keys() {
        let draw = find("draw").unwrap();
        let draw_keys: Vec<&str> = keys(draw).iter().map(|k| k.press).collect();
        assert!(draw_keys.starts_with(&["A", "R", "P", "H", "T", "X", "1-5", "⌘Z"]));
        assert!(keys(find("annotate").unwrap()).iter().any(|k| k.press == "⌘S"));
        assert!(keys(find("launcher").unwrap()).is_empty());
    }

    #[test]
    fn text_is_the_how_to_then_the_keys() {
        assert_eq!(
            text(find("config").unwrap()),
            "Configuration: Open Flick Config edits config.toml; Reload Flick Config applies it."
        );
        let draw = text(find("draw").unwrap());
        assert!(draw.contains("\n  1-5  Pick color 1 to 5"), "{draw}");
        assert!(draw.contains("\n  ⌘Z  Undo the last shape (there is no eraser)"), "{draw}");
    }
}
