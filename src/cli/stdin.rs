//! The `--stdin` request word: the client replaces it with what it reads from stdin, so a
//! large or sensitive argument (a card's JSON) stays off the command line and out of `ps`.

use std::io::Read;

/// The request word the client replaces with stdin's contents.
pub const WORD: &str = "--stdin";

/// The most bytes `--stdin` may read.
pub const MAX: usize = 16 * 1024;

/// Why `--stdin` could not be filled in, with the exit code the client should return.
#[derive(Debug, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    /// 2 for a usage mistake (the word twice, stdin a terminal), 1 for bad input.
    pub code: i32,
}

impl Error {
    fn usage(message: &str) -> Error {
        Error { message: message.into(), code: 2 }
    }

    fn input(message: String) -> Error {
        Error { message, code: 1 }
    }
}

/// `words` with the one word that equals `WORD` replaced by everything `input` holds, as is
/// (no trimming; empty input is an empty word). Words without it come back unchanged and
/// `input` is not read. The word may appear at most once; `terminal` (stdin is a terminal,
/// so reading would wait for typing) is refused. More than `MAX` bytes, or input that is not
/// UTF-8, is an error.
pub fn fill(
    words: Vec<String>,
    input: &mut dyn Read,
    terminal: bool,
) -> Result<Vec<String>, Error> {
    let mut at = words.iter().enumerate().filter(|(_, w)| *w == WORD).map(|(i, _)| i);
    let Some(i) = at.next() else { return Ok(words) };
    if at.next().is_some() {
        return Err(Error::usage("--stdin may appear only once"));
    }
    if terminal {
        return Err(Error::usage("--stdin: stdin is a terminal; pipe the text in"));
    }
    let mut bytes = Vec::new();
    let cap = u64::try_from(MAX).unwrap_or(u64::MAX) + 1;
    input.take(cap).read_to_end(&mut bytes).map_err(|e| Error::input(format!("--stdin: {e}")))?;
    if bytes.len() > MAX {
        return Err(Error::input(format!("--stdin: input over {MAX} bytes")));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| Error::input("--stdin: input is not UTF-8".into()))?;
    let mut words = words;
    words[i] = text;
    Ok(words)
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_string()).collect()
    }

    fn fill_with(w: &[&str], input: &[u8]) -> Result<Vec<String>, Error> {
        fill(words(w), &mut io::Cursor::new(input.to_vec()), false)
    }

    /// A reader that fails: proves `fill` did not read it, or reports its error.
    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("broken pipe"))
        }
    }

    #[test]
    fn the_word_becomes_stdin_verbatim_wherever_it_is() {
        let json = "{\"id\":\"a\",\n \"title\":\"it's \\\"x\\\"\"}\n";
        assert_eq!(
            fill_with(&["message", "card", "post", "--stdin"], json.as_bytes()).unwrap(),
            ["message", "card", "post", json]
        );
        assert_eq!(
            fill_with(&["message", "post", "--stdin", "--title", "K"], b"hi").unwrap(),
            ["message", "post", "hi", "--title", "K"]
        );
        // Empty stdin is an empty word; the module decides what that means.
        assert_eq!(fill_with(&["x", "--stdin"], b"").unwrap(), ["x", ""]);
        // Exactly the cap is fine; multi-byte text counts bytes.
        let max = "é".repeat(MAX / 2);
        assert_eq!(fill_with(&["x", "--stdin"], max.as_bytes()).unwrap(), ["x", max.as_str()]);
    }

    #[test]
    fn without_the_word_stdin_is_not_read() {
        let w = words(&["task", "ls", "--stdin=x", "-stdin"]);
        assert_eq!(fill(w.clone(), &mut Broken, true).unwrap(), w);
    }

    #[test]
    fn misuse_and_bad_input_are_errors_with_exit_codes() {
        let usage = |m: &str| Err(Error { message: m.into(), code: 2 });
        let input = |m: &str| Err(Error { message: m.into(), code: 1 });
        assert_eq!(
            fill_with(&["x", "--stdin", "--stdin"], b"a"),
            usage("--stdin may appear only once")
        );
        assert_eq!(
            fill(words(&["x", "--stdin"]), &mut Broken, true),
            usage("--stdin: stdin is a terminal; pipe the text in")
        );
        assert_eq!(
            fill_with(&["x", "--stdin"], &vec![b'a'; MAX + 1]),
            input("--stdin: input over 16384 bytes")
        );
        assert_eq!(
            fill_with(&["x", "--stdin"], &[0xff, 0xfe]),
            input("--stdin: input is not UTF-8")
        );
        assert_eq!(fill(words(&["--stdin"]), &mut Broken, false), input("--stdin: broken pipe"));
    }
}
