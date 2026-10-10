//! Fleet actions on threads (flick-4a4c): a tail and a restart each run on their own thread
//! with a time budget, write `State::tail` or `State::acted`, post `ModuleChanged`, and
//! answer a CLI caller through `core::later`. The main thread only starts them.

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use super::act::{self, Place, Run, Target};
use super::io::{Hooks, Shared};
use super::poll::SSH_BUDGET;
use super::run::Exit;
use crate::core::later::Answer;

/// Budget of a tail or a restart on this Mac (`kickstart -k` waits for the old process).
pub const LOCAL_BUDGET: Duration = Duration::from_secs(10);

/// How long a restart's result shows in the fleet views' footer, seconds.
pub const SHOW_ACTED: u64 = 60;

/// The last tail asked for, shown by view `log`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tail {
    /// Which run this is; a newer tail replaces it and an older thread's result is dropped.
    pub id: u64,
    pub machine: String,
    pub service: String,
    /// The command, as shown.
    pub shown: String,
    /// `None` while it runs.
    pub text: Option<Result<String, String>>,
    /// When it finished, unix seconds.
    pub at: Option<u64>,
}

/// Run `run` with its stdin, if any, within the budget for `place`.
fn exec(run: &Run, place: &Place, hooks: Hooks) -> Result<Exit, String> {
    let budget = if *place == Place::Local { LOCAL_BUDGET } else { SSH_BUDGET };
    match &run.input {
        Some(input) => (hooks.run_input)(&run.argv, input, budget),
        None => (hooks.run)(&run.argv, budget),
    }
}

/// Start a tail of `target`'s log: it becomes `State::tail`. `answer` gets the lines.
pub fn tail(shared: &Arc<Shared>, target: &Target, run: Run, hooks: Hooks, answer: Option<Sender<Answer>>) -> Result<(), String> {
    let id = {
        let mut st = shared.lock();
        let id = st.tail.as_ref().map_or(1, |t| t.id + 1);
        let (machine, service) = (target.machine.clone(), target.service.name.clone());
        st.tail = Some(Tail { id, machine, service, shown: run.shown.clone(), text: None, at: None });
        id
    };
    let (sh, place) = (Arc::clone(shared), target.place.clone());
    let spawned = thread::Builder::new().name("sys-tail".into()).spawn(move || {
        let got = act::tailed(exec(&run, &place, hooks));
        let mut st = sh.lock();
        if let Some(t) = st.tail.as_mut().filter(|t| t.id == id) {
            t.text = Some(got.clone());
            t.at = Some((hooks.now)());
        }
        drop(st);
        sh.changed(hooks);
        if let Some(tx) = answer {
            let _ = tx.send(got);
        }
    });
    spawned.map(|_| ()).map_err(|e| format!("sys tail: no thread: {e}"))
}

/// Start a restart of `target`: its result becomes `State::acted`. `answer` gets it too.
pub fn restart(shared: &Arc<Shared>, target: &Target, run: Run, hooks: Hooks, answer: Option<Sender<Answer>>) -> Result<(), String> {
    let (sh, place, what) = (Arc::clone(shared), target.place.clone(), target.what());
    let spawned = thread::Builder::new().name("sys-restart".into()).spawn(move || {
        let got = act::restarted(exec(&run, &place, hooks), &what);
        let text = got.clone().unwrap_or_else(|e| e);
        sh.lock().acted = Some((text, (hooks.now)()));
        sh.changed(hooks);
        if let Some(tx) = answer {
            let _ = tx.send(got);
        }
    });
    spawned.map(|_| ()).map_err(|e| format!("sys restart: no thread: {e}"))
}

/// The last restart's result while it is recent.
pub fn acted(acted: Option<&(String, u64)>, now: u64) -> Option<&str> {
    acted.filter(|(_, at)| now.saturating_sub(*at) < SHOW_ACTED).map(|(text, _)| text.as_str())
}
