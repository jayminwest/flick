//! The request words of a `{ flick = "<module> <verb> [args]" }` action, split shell-like:
//! whitespace separates words, `'...'` keeps its text as is, `"..."` keeps its text with
//! `\"` and `\\` unescaped, and a backslash outside quotes keeps the next character. No
//! variables, globs or other shell syntax: the words go to the module as written.

/// Split `text` into words. `Err` for an unclosed quote, a trailing backslash or no words.
pub fn split(text: &str) -> Result<Vec<String>, String> {
    let mut words = vec![];
    let mut word = String::new();
    // A word has started, so an empty quoted word ('' or "") still counts.
    let mut started = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("unclosed '".into()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\')) => word.push(c),
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err("unclosed \"".into()),
                        },
                        Some(c) => word.push(c),
                        None => return Err("unclosed \"".into()),
                    }
                }
            }
            '\\' => {
                started = true;
                word.push(chars.next().ok_or("trailing \\")?);
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    if words.is_empty() {
        return Err("empty request".into());
    }
    Ok(words)
}

/// `words` joined for display, single-quoting a word that is empty or would split.
pub fn join(words: &[String]) -> String {
    let shown = words.iter().map(|w| {
        if w.is_empty() || w.contains(|c: char| c.is_whitespace() || "'\"\\".contains(c)) {
            format!("'{}'", w.replace('\'', r"'\''"))
        } else {
            w.clone()
        }
    });
    shown.collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        split(text).unwrap()
    }

    #[test]
    fn splits_on_whitespace_and_honours_quotes() {
        assert_eq!(words("dictation start"), ["dictation", "start"]);
        assert_eq!(words("  dictation\t start \n"), ["dictation", "start"]);
        assert_eq!(words("message post 'hello world'"), ["message", "post", "hello world"]);
        assert_eq!(words(r#"a "b \"c\" \\ \n" d"#), ["a", r#"b "c" \ \n"#, "d"]);
        assert_eq!(words(r"a\ b c\'"), ["a b", "c'"]);
        assert_eq!(words("a '' \"\""), ["a", "", ""]);
        assert_eq!(words("pre'fix'\"ed\""), ["prefixed"]);
    }

    #[test]
    fn rejects_broken_or_empty_text() {
        assert_eq!(split("a 'b").unwrap_err(), "unclosed '");
        assert_eq!(split("a \"b").unwrap_err(), "unclosed \"");
        assert_eq!(split("a \"b\\").unwrap_err(), "unclosed \"");
        assert_eq!(split("a\\").unwrap_err(), "trailing \\");
        assert_eq!(split("").unwrap_err(), "empty request");
        assert_eq!(split(" \t ").unwrap_err(), "empty request");
    }

    #[test]
    fn join_round_trips() {
        for text in ["dictation start", "message post 'hi there'", "a '' 'it'\\''s'"] {
            let w = words(text);
            assert_eq!(join(&w), text);
            assert_eq!(words(&join(&w)), w);
        }
        assert_eq!(join(&words(r#"x "a\"b""#)), r#"x 'a"b'"#);
    }
}
