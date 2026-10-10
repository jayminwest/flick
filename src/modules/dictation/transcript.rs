//! Engine output to the text that goes in: whisper's non-speech tokens out (`[BLANK_AUDIO]`,
//! `[Music]`, `(music)`, `*laughs*`, `♪`), lines joined, whitespace collapsed.

/// Words that mark a `(...)` or `*...*` span as a sound, not speech. A `[...]` span is
/// always dropped: whisper writes no speech in square brackets.
const SOUNDS: &[&str] = &[
    "applause",
    "audio",
    "beep",
    "blank",
    "breath",
    "cough",
    "crowd",
    "inaudible",
    "laugh",
    "music",
    "noise",
    "sigh",
    "silence",
    "sound",
    "static",
    "typing",
    "wind",
];

/// The text to insert, or `None` when nothing but non-speech remains. With
/// `trailing_space`, a space follows it.
pub fn clean(raw: &str, trailing_space: bool) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find(['[', '(', '*', '♪']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let open = rest.chars().next().unwrap_or_default();
        let close = match open {
            '[' => ']',
            '(' => ')',
            '*' => '*',
            _ => {
                // ♪ is a note symbol; drop it.
                rest = &rest[open.len_utf8()..];
                continue;
            }
        };
        let span = rest[1..].find(close).map(|j| (&rest[1..=j], j + 2));
        match span {
            Some((inner, len)) if open == '[' || is_sound(inner) => rest = &rest[len..],
            _ => {
                out.push(open);
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    let text = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let speech = text.chars().any(char::is_alphanumeric);
    speech.then(|| if trailing_space { text + " " } else { text })
}

/// `inner` names a sound: a few words, one of them in `SOUNDS`.
fn is_sound(inner: &str) -> bool {
    let lower = inner.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).collect();
    words.len() <= 3 && words.iter().any(|w| SOUNDS.iter().any(|s| w.starts_with(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(raw: &str) -> Option<String> {
        clean(raw, false)
    }

    #[test]
    fn drops_non_speech_tokens() {
        for raw in ["[BLANK_AUDIO]", " [Music]\n", "(music)", "(upbeat music)", "*laughs*", "♪♪", " ( Silence ) ", "[ Inaudible ]", "", " .\n", "(Applause) [Laughter]"] {
            assert_eq!(text(raw), None, "{raw:?}");
        }
        assert_eq!(text(" [BLANK_AUDIO] Hello there. (music)"), Some("Hello there.".into()));
        assert_eq!(text("♪ La la la ♪"), Some("La la la".into()));
        assert_eq!(text("*coughs* So, the plan"), Some("So, the plan".into()));
    }

    #[test]
    fn keeps_spoken_brackets_and_stars() {
        assert_eq!(text("Call me (maybe) later"), Some("Call me (maybe) later".into()));
        assert_eq!(text("a (sound of the city at night) b").unwrap(), "a (sound of the city at night) b");
        assert_eq!(text("2 * 3 = 6"), Some("2 * 3 = 6".into()));
        assert_eq!(text("open ( paren"), Some("open ( paren".into()));
        assert_eq!(text("x [y"), Some("x [y".into()));
        assert_eq!(text("é *wind* ü"), Some("é ü".into()));
    }

    #[test]
    fn joins_lines_and_adds_the_trailing_space() {
        assert_eq!(text(" Hello\n world.  \n"), Some("Hello world.".into()));
        assert_eq!(clean(" Hi.\n", true), Some("Hi. ".into()));
        assert_eq!(clean("[BLANK_AUDIO]", true), None);
    }
}
