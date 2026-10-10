//! The KOTA chat window's pure side (plan pl-75d3): `model.rs` turns a thread's stored
//! messages into transcript rows, titles, navigation, header status and the HUD-vs-window
//! policy; `ask.rs` builds what an ask sends (the question with its `[context]` block, the
//! kota-ask argv with `--thread`); `attach.rs` validates the remote attach dir and builds a
//! screenshot's upload command. No `AppKit`, no I/O: the wiring (flick-eedd) maps the rows
//! onto `platform::surface` and runs the commands on a worker.

pub mod ask;
pub mod attach;
pub mod model;
