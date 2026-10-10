//! Fakes for the module's tests: `spawn` answers by URL (the argv's last word) with canned
//! output, so no test runs curl or reaches a server. Every argv and stdin body lands in
//! `SENT` for the assertions that prompt text goes over stdin only.

use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use super::io::Hooks;
use super::transport::{Child, Spawned};

pub const STREAM: &str = include_str!("fixtures/chat_stream.txt");
pub const REASONING: &str = include_str!("fixtures/chat_reasoning.txt");
pub const MLX_MODELS: &str = include_str!("fixtures/models_mlx.json");

pub const HOOKS: Hooks = Hooks { spawn, post: || {}, sleep: |d| std::thread::sleep(d / 50) };

/// Hooks whose watchdog never fires within a test.
pub const PATIENT: Hooks = Hooks { sleep: |_| std::thread::sleep(Duration::from_secs(3_600)), ..HOOKS };

/// One spawn: its argv and what was written to its stdin.
#[derive(Clone, Debug, Default)]
pub struct Sent {
    pub argv: Vec<String>,
    pub stdin: Vec<u8>,
}

pub static SENT: Mutex<Vec<Sent>> = Mutex::new(vec![]);

/// What was sent to `url`, in order.
pub fn sent(url: &str) -> Vec<Sent> {
    let sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
    sent.iter().filter(|s| s.argv.last().is_some_and(|u| u == url)).cloned().collect()
}

/// A killed flag that blocking readers wait on.
#[derive(Default)]
struct Gate {
    killed: Mutex<bool>,
    cond: Condvar,
}

struct FakeChild {
    code: Option<i32>,
    gate: Arc<Gate>,
}

impl Child for FakeChild {
    fn kill(&mut self) {
        self.code = None;
        *self.gate.killed.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.gate.cond.notify_all();
    }

    fn wait(&mut self) -> Option<i32> {
        self.code
    }
}

/// Gives `head`, then blocks until the child is killed (or `head` only, without a gate).
struct Hang {
    head: Cursor<Vec<u8>>,
    gate: Arc<Gate>,
}

impl Read for Hang {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.head.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        let mut killed = self.gate.killed.lock().unwrap_or_else(PoisonError::into_inner);
        while !*killed {
            killed = self.gate.cond.wait(killed).unwrap_or_else(PoisonError::into_inner);
        }
        Ok(0)
    }
}

/// Records stdin into `SENT[index]`.
struct Stdin(usize);

impl Write for Stdin {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
        sent[self.0].stdin.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Canned answers by host (`http://<host>/v1/...`):
/// - `mlx`: `MLX_MODELS` / `STREAM`; `reason`: `REASONING`;
/// - `down`: curl's connect error, exit 7; `err`: an error body, exit 22;
/// - `junk`: a non-JSON body, exit 0; `empty`: nothing, exit 0;
/// - `inband`: an error chunk inside the stream;
/// - `hang`: blocks until killed; `half`: one text chunk, then blocks until killed;
/// - `nospawn`: curl does not start.
fn spawn(argv: &[String]) -> Result<Spawned, String> {
    let url = argv.last().cloned().unwrap_or_default();
    let index = {
        let mut sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
        sent.push(Sent { argv: argv.to_vec(), stdin: vec![] });
        sent.len() - 1
    };
    let host = url.trim_start_matches("http://").split('/').next().unwrap_or_default();
    let models = url.ends_with("/v1/models");
    let gate = Arc::new(Gate::default());
    let (out, err, code, hang): (&str, &str, Option<i32>, bool) = match host {
        "nospawn" => return Err("/usr/bin/curl: No such file or directory".into()),
        "mlx" if models => (MLX_MODELS, "", Some(0), false),
        "mlx" => (STREAM, "", Some(0), false),
        "reason" => (REASONING, "", Some(0), false),
        "down" => ("", "curl: (7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server\n", Some(7), false),
        "err" => (r#"{"error":{"message":"model not found"}}"#, "curl: (22) The requested URL returned error: 404\n", Some(22), false),
        "junk" => ("<html>hi</html>\n", "", Some(0), false),
        "inband" => ("data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\ndata: {\"error\":\"overloaded\"}\n", "", Some(0), false),
        "hang" => ("", "", Some(0), true),
        "half" => ("data: {\"choices\":[{\"delta\":{\"content\":\"part\"}}]}\n", "", Some(0), true),
        _ => ("", "", Some(0), false),
    };
    let stdout: Box<dyn Read + Send> = if hang {
        Box::new(Hang { head: Cursor::new(out.as_bytes().to_vec()), gate: Arc::clone(&gate) })
    } else {
        Box::new(Cursor::new(out.as_bytes().to_vec()))
    };
    Ok(Spawned {
        stdin: Box::new(Stdin(index)),
        stdout,
        stderr: Box::new(Cursor::new(err.as_bytes().to_vec())),
        child: Box::new(FakeChild { code, gate }),
    })
}
