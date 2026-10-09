//! The socket client: send one request and print its reply, or print the event stream.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use serde_json::Value;

use crate::core::control::{EVENTS, Flags, JSON, REMOTE, Reply, request_line};

/// Send `words` to the Flick at `path` and print the reply: its text, or with `flags.json`
/// the raw reply line. With `flags.json` the request ends in `--json`, so a module may answer
/// with structured JSON; `flags.remote` adds `--remote` before it. Returns the exit code: 0
/// for an ok reply, 1 otherwise.
pub fn request(path: &Path, words: &[String], flags: Flags) -> i32 {
    let json = flags.json;
    let words = with_flags(words, flags);
    let reply = connect(path).and_then(|stream| exchange(stream, &words));
    match reply {
        Ok(line) => {
            let (out, err, code) = render(&line, json);
            print!("{out}");
            eprint!("{err}");
            code
        }
        Err(e) => {
            eprintln!("flick: {e}");
            1
        }
    }
}

/// Subscribe to the Flick at `path` and print each event line until it goes away.
pub fn events(path: &Path) -> i32 {
    let streamed = connect(path).and_then(|mut stream| {
        writeln!(stream, "{}", request_line(&[EVENTS.to_string()]))?;
        let mut stdout = io::stdout().lock();
        for line in BufReader::new(stream).lines() {
            writeln!(stdout, "{}", line?)?;
            stdout.flush()?;
        }
        Ok(())
    });
    match streamed {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("flick: {e}");
            1
        }
    }
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("can't reach Flick at {} ({e}); is it running?", path.display()),
        )
    })
}

/// The request words: `words`, then `--remote` when `flags.remote`, then `--json` when
/// `flags.json` (the order `core::control::split_flags` takes them off).
fn with_flags(words: &[String], flags: Flags) -> Vec<String> {
    let mut words = words.to_vec();
    if flags.remote {
        words.push(REMOTE.to_string());
    }
    if flags.json {
        words.push(JSON.to_string());
    }
    words
}

/// Write the request line for `words` and read the one reply line.
fn exchange(mut stream: UnixStream, words: &[String]) -> io::Result<String> {
    writeln!(stream, "{}", request_line(words))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Flick closed the connection"));
    }
    Ok(line)
}

/// What to print for reply `line`: (stdout, stderr, exit code).
fn render(line: &str, json: bool) -> (String, String, i32) {
    let reply = Reply::parse(line);
    let code = i32::from(!matches!(reply, Ok(Reply::Ok(_))));
    if json {
        return (format!("{}\n", line.trim_end()), String::new(), code);
    }
    match reply {
        Ok(Reply::Ok(Value::String(out))) if out.is_empty() || out.ends_with('\n') => {
            (out, String::new(), 0)
        }
        Ok(Reply::Ok(Value::String(out))) => (format!("{out}\n"), String::new(), 0),
        Ok(Reply::Ok(value)) => (format!("{value}\n"), String::new(), 0),
        Ok(Reply::Error(e)) | Err(e) => (String::new(), format!("flick: {e}\n"), 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::server::{self, Hub};

    #[test]
    fn replies_print_as_text_or_raw_json() {
        let ok = |s: &str| (s.to_string(), String::new(), 0);
        assert_eq!(render("{\"ok\":\"a\\nb\"}\n", false), ok("a\nb\n"));
        assert_eq!(render("{\"ok\":\"a\\n\"}", false), ok("a\n"));
        assert_eq!(render("{\"ok\":\"\"}", false), ok(""));
        assert_eq!(render("{\"ok\":\"a\"}\n", true), ok("{\"ok\":\"a\"}\n"));
        assert_eq!(render("{\"error\":\"no\"}\n", false), (String::new(), "flick: no\n".into(), 1));
        assert_eq!(
            render("{\"error\":\"no\"}", true),
            ("{\"error\":\"no\"}\n".into(), String::new(), 1)
        );
        assert_eq!(render("{\"ok\":{\"a\":[1]}}", false), ok("{\"a\":[1]}\n"));
        assert_eq!(render("{\"ok\":[1]}\n", true), ok("{\"ok\":[1]}\n"));
        let (out, err, code) = render("garbage", false);
        assert!(out.is_empty() && err.starts_with("flick: bad reply") && code == 1);
    }

    fn upper(words: Vec<String>) -> Reply {
        let text = words.into_iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join(" ");
        Reply::Ok(text.into())
    }

    #[test]
    fn a_request_round_trips_through_a_real_socket() {
        static HUB: Hub = Hub::new();
        let dir = std::env::temp_dir().join(format!("flk-{}-client", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.sock");
        let missing = connect(&path).unwrap_err().to_string();
        assert!(missing.contains("is it running?"), "{missing}");
        assert_eq!(request(&path, &["x".into()], Flags::default()), 1);
        server::spawn(server::bind(&path).unwrap(), upper, &HUB).unwrap();
        let words = ["clip".to_string(), "get".into(), "a b".into()];
        let line = exchange(connect(&path).unwrap(), &words).unwrap();
        assert_eq!(line, "{\"ok\":\"CLIP GET A B\"}\n");
        assert_eq!(request(&path, &words, Flags { json: true, remote: true }), 0);
    }

    #[test]
    fn flag_words_go_last_in_the_order_the_server_takes_them_off() {
        let words = ["task".to_string(), "ls".into()];
        let sent = |json, remote| with_flags(&words, Flags { json, remote });
        assert_eq!(sent(false, false), words);
        assert_eq!(sent(true, false), ["task", "ls", "--json"]);
        assert_eq!(sent(false, true), ["task", "ls", "--remote"]);
        let both = sent(true, true);
        assert_eq!(both, ["task", "ls", "--remote", "--json"]);
        assert_eq!(crate::core::control::split_flags(both).1, Flags { json: true, remote: true });
    }
}
