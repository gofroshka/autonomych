//! Preview state — minimal holder for the Presenter agent's output.
//!
//! Process management (starting the dev server, killing it later) is
//! delegated entirely to the agent. We don't track PIDs, URLs, log files,
//! manifests, or anything else. The agent returns a free-form text message
//! describing how the user should test the project; we render that as-is.

use crate::types::PreviewStatus;

#[derive(Debug, Default)]
pub struct PreviewState {
    /// Free-form, human-readable instructions from the LAUNCH agent. URLs,
    /// credentials, what to click, anything useful. Rendered verbatim in
    /// the Presenting overlay.
    pub instructions: Option<String>,
    /// Wall-clock time when launch finished.
    pub prepared_at: Option<i64>,
    /// Error message if launch failed (agent crashed, returned empty, etc.).
    /// Mutually exclusive with `instructions` for display purposes.
    pub prep_error: Option<String>,
    /// Free-form message from the most recent SHUTDOWN agent run, kept for
    /// the activity feed / debug.
    pub shutdown_message: Option<String>,
}

impl PreviewState {
    pub fn reset_prep(&mut self) {
        self.instructions = None;
        self.prepared_at = None;
        self.prep_error = None;
    }

    pub fn status(&self) -> PreviewStatus {
        PreviewStatus {
            instructions: self.instructions.clone(),
            prepared_at: self.prepared_at,
            prep_error: self.prep_error.clone(),
        }
    }
}
