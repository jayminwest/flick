//! Module `llm`: chat with OpenAI-compatible model servers (mlx-serve, ollama) on the
//! tailnet, with a private mode that stores nothing (plan pl-0696, parent flick-6519). Steps
//! so far: settings and the pure wire (flick-9f17), the curl transport and `llm ping|models`
//! (flick-633e), the private session core (flick-56dc), the normal chat (flick-6a0d). Never
//! imports the `message` module's chat.
//!
//! - `settings.rs`: `[llm]` and `[[llm.servers]]` (`name`, `url`, `private`,
//!   `api_key`). With no server the module runs nothing.
//! - `openai.rs`: the chat request body, the SSE line parser, error bodies, `/v1/models`.
//! - `transport.rs`: one curl per call, body on stdin, never prompt text in argv; an
//!   `api_key` goes in a 0600 curl config file that `keyfile.rs` writes and removes.
//! - `io.rs`: the threads: model lists, streamed replies into an inbox, cancel, watchdog.
//! - `chat.rs`: the normal chat window (surface "llm"): summon, send, stream, stop, history;
//!   `view.rs` draws it, pure; `store.rs` keeps it (`llm_threads`, `llm_messages`).
//! - `views.rs`: the root item `llm:chat` (only with a normal server), the `models` picker
//!   and the `threads` list.
//! - `private.rs`: `PrivateSession`, the private chat's in-memory transcript (no store, no
//!   `Serialize`, redacted `Debug`, wiped on clear and drop), its `private = true` server gate
//!   and when it is wiped (`Wipe`, `wipe_on`).
//! - `private_chat.rs`: the private chat window (surface "llm-private", flick-c325): the root
//!   item `llm:private` and `private_hotkey`, concealed copy, wiped on close, `Locked`,
//!   `Sleep`, reload (`configure`) and quit.
//!
//! `flick llm ping [server]` and `flick llm models [server] [--json]` fetch a normal (not
//! private) server's `/v1/models`, waiting at most `ASK_WAIT`. `flick llm ping private
//! [server]` (flick-72b5) fetches a private server's list the same way and prints only the
//! name, the time and the model count: a `GET` with no body, no chat text, nothing stored. It
//! is the one verb that reaches a private server. Every `llm` verb is denied over the network
//! (`NET_DENIED`): they make this Mac send requests. The private chat itself has no verb.
//!
//! Nothing here logs prompts or replies; errors carry at most 200 characters of a server's
//! error message.

mod chat;
mod io;
mod keyfile;
mod openai;
mod private;
mod private_chat;
mod report;
mod settings;
mod store;
#[cfg(test)]
mod testkit;
mod transport;
mod view;
mod views;
mod wire;

use std::sync::Arc;
use std::time::Duration;

use crate::config::Section;
use crate::core::{Action, Binding, Cx, Event, Item, ItemId, ListView, Module, Outcome, unknown_verb};
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
    chat: chat::Chat,
    private: private_chat::Private,
}

impl Default for Llm {
    fn default() -> Self {
        Llm::with_hooks(wire::HOOKS, wire::UI, wire::PRIVATE_UI)
    }
}

impl Drop for Llm {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl Llm {
    fn with_hooks(hooks: Hooks, ui: chat::Ui, private: private_chat::Ui) -> Llm {
        let (chat, private) = (chat::Chat::new(ui), private_chat::Private::new(private));
        Llm { settings: Settings::default(), shared: Arc::default(), hooks, ask_wait: ASK_WAIT, chat, private }
    }

    /// Fetch `server`'s model list now and wait for it, at most `ask_wait`. Only a `GET` of
    /// `/v1/models`: safe for a private server too (`llm ping private`).
    fn ask(&self, server: &settings::Server) -> Result<(String, Vec<openai::Model>, Duration), String> {
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

    fn migrations(&self) -> &'static [&'static str] {
        store::MIGRATIONS
    }

    /// A reload wipes the private chat. `modules::reload` also configures a throwaway fresh
    /// instance first; it never had a private chat, so only the running module wipes.
    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.settings = table.get::<Settings>()?.check()?;
        self.private_wipe(private::Wipe::Reload);
        Ok(())
    }

    /// `ModuleChanged` for this module: the threads' results and both windows' notes.
    /// `Locked` and `Sleep` wipe the private chat.
    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        if event == (Event::ModuleChanged { module: ID }) {
            // The one posted event arrived: the threads may post again.
            self.shared.lock().posted = false;
            self.chat_drain(cx);
            self.private_drain();
            return true;
        }
        if let Some(why) = private::wipe_on(event) {
            self.private_wipe(why);
        }
        false
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        self.root_items()
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        self.view(view)
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        self.fill(view, cx);
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        self.enter(id, cx)
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        Llm::root_actions(id)
    }

    fn act(&mut self, id: &ItemId, key: &str, _cx: &mut Cx) -> Outcome {
        match Llm::root_actions(id).iter().find(|a| a.key == key) {
            Some(a) => Outcome::Push(ListView::new(ID, a.key)),
            None => Outcome::Stay(None),
        }
    }

    /// `hotkey` shows or hides the chat window; only with a normal server. `private_hotkey`
    /// shows or hides the private one; bound whenever set, so that without a private server it
    /// can say why.
    fn hotkeys(&self) -> Vec<Binding> {
        let set = |spec: &Option<String>| spec.as_ref().filter(|s| !s.trim().is_empty()).cloned();
        let usable = self.settings.normal(None).is_ok();
        let chat = set(&self.settings.hotkey).filter(|_| usable).map(|spec| Binding { spec, key: Ok(views::CHAT.into()) });
        let private = set(&self.settings.private_hotkey).map(|spec| Binding { spec, key: Ok(views::PRIVATE.into()) });
        chat.into_iter().chain(private).collect()
    }

    /// Without a private server, the private hotkey shows the launcher saying so.
    fn hotkey(&mut self, key: &str, cx: &mut Cx) -> Option<ListView> {
        match key {
            views::CHAT => self.chat_toggle(cx),
            views::PRIVATE => return self.private_toggle().err().and_then(|_| self.view(views::PRIVATE)),
            _ => {}
        }
        None
    }

    /// `--json` (`cx.json`) makes `models` answer with `report::models_json`.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            // A private server: only whether it answers, never its models' names.
            ["ping", "private", rest @ ..] if rest.len() <= 1 => {
                let (server, list, took) = self.ask(self.settings.private(rest.first().copied())?)?;
                Ok(report::ping_text(&server, &list, took))
            }
            ["ping" | "models", rest @ ..] if rest.len() <= 1 => {
                let (server, list, took) = self.ask(self.settings.normal(rest.first().copied())?)?;
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
        "llm ping [server] | llm ping private [server] | llm models [server]"
    }
}

#[cfg(test)]
mod tests;
