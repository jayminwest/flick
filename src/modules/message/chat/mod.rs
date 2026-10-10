//! The KOTA chat window (plan pl-75d3). Pure side: `model.rs` turns a thread's stored
//! messages into transcript rows, titles, navigation, header status and the HUD-vs-window
//! policy; `ask.rs` builds what an ask sends (the question with its `[context]` block, the
//! kota-ask argv with `--thread`); `attach.rs` validates the remote attach dir and builds a
//! screenshot's upload command; `view.rs` maps rows onto `platform::surface`'s. Wiring
//! (flick-eedd): `session.rs` (summon, hide, keys, threads, redraws), `asks.rs` (the ask
//! queue and worker, `message ask`/`chat`), `threads.rs` (the launcher's `message/threads`)
//! and `wire.rs` (the real hooks: the "chat" surface, the front app, ssh).

pub mod ask;
pub mod asks;
#[cfg_attr(not(test), expect(dead_code, reason = "screenshot upload; wired in flick-65bd"))]
pub mod attach;
#[cfg(test)]
pub mod fake;
pub mod model;
pub mod session;
pub mod threads;
pub mod view;
mod wire;
