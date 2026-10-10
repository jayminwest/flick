//! Module `llm`: chat with OpenAI-compatible model servers (mlx-serve, ollama) on the
//! tailnet, with a private mode that stores nothing (plan pl-0696, parent flick-6519). Steps
//! so far: settings and the pure wire (flick-9f17), the curl transport and `llm ping|models`
//! (flick-633e). No items, views or windows yet. Never imports the `message` module's chat.
//!
//! - `settings.rs`: `[llm]` and `[[llm.servers]]` (`name`, `url`, `private`). With no
//!   server the module runs nothing.
//! - `openai.rs`: the chat request body, the SSE line parser, error bodies, `/v1/models`.
//! - `transport.rs`: one curl per call, body on stdin, never prompt text in argv.
//! - `io.rs`: the threads: model lists, streamed replies into an inbox, cancel, watchdog.
//!
//! `flick llm ping [server]` and `flick llm models [server] [--json]` fetch a normal (not
//! private) server's `/v1/models`, waiting at most `ASK_WAIT`. Every `llm` verb is denied
//! over the network (`NET_DENIED`): they make this Mac send requests.
//!
//! Nothing here logs prompts or replies; errors carry at most 200 characters of a server's
//! error message.

mod io;
mod openai;
mod report;
mod settings;
#[cfg(test)]
mod testkit;
mod transport;
mod wire;

use std::sync::Arc;
use std::time::Duration;

use crate::config::Section;
use crate::core::{Cx, Event, Module, unknown_verb};
use io::{Hooks, Models, Shared};
use settings::Settings;

pub const ID: &str = "llm";

/// How long `llm ping|models` waits for the list: curl's budget plus spawn.
const ASK_WAIT: Duration = Duration::from_millis(io::LIST_MAX_TIME * 1_000 + 500);

pub struct Llm {
    settings: Settings,
    shared: Arc<Shared>,
    hooks: Hooks,
    ask_wait: Duration,
}

impl Default for Llm {
    fn default() -> Self {
        Llm::with_hooks(wire::HOOKS)
    }
}

impl Drop for Llm {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl Llm {
    fn with_hooks(hooks: Hooks) -> Llm {
        Llm { settings: Settings::default(), shared: Arc::default(), hooks, ask_wait: ASK_WAIT }
    }

    /// Fetch a normal server's model list now and wait for it, at most `ask_wait`.
    fn ask(&self, name: Option<&str>) -> Result<(String, Vec<openai::Model>, Duration), String> {
        let server = self.settings.normal(name)?;
        let seq = io::fetch_models(&self.shared, server, self.hooks);
        let key = server.name.as_str();
        let landed = |st: &io::State| st.models.get(key).is_some_and(|m| m.seq >= seq);
        if !self.shared.wait(self.ask_wait, landed) {
            return Err(format!("llm: {key} did not answer in {} s", self.ask_wait.as_secs()));
        }
        let m: Models = self.shared.lock().models.get(key).cloned().unwrap_or_default();
        let list = m.result.unwrap_or(Err(String::from("no answer"))).map_err(|e| format!("llm: {key}: {e}"))?;
        Ok((key.to_string(), list, m.took))
    }
}

impl Module for Llm {
    fn id(&self) -> &'static str {
        ID
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.settings = table.get::<Settings>()?.check()?;
        Ok(())
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        if event == (Event::ModuleChanged { module: ID }) {
            // The one posted event arrived: the threads may post again.
            self.shared.lock().posted = false;
            return true;
        }
        false
    }

    /// `--json` (`cx.json`) makes `models` answer with `report::models_json`.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["ping" | "models", rest @ ..] if rest.len() <= 1 => {
                let (server, list, took) = self.ask(rest.first().copied())?;
                Ok(match words[0] {
                    "ping" => report::ping_text(&server, &list, took),
                    _ if cx.json => report::models_json(&server, &list, took).to_string(),
                    _ => report::models_text(&server, &list),
                })
            }
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "llm ping [server] | llm models [server]"
    }
}

#[cfg(test)]
mod tests;
