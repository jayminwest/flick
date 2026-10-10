//! What a child process left behind. flick-4f39 adds the runner.

/// A finished child process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exit {
    /// `None` when a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
