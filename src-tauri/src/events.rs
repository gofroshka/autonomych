//! Typed event pipeline.
//!
//! [`EventPayload`] is the single typed contract for everything that crosses
//! the conductor → store → renderer boundary. No more stringified JSON blobs:
//! every event variant has named fields, and frontend reads it as a
//! discriminated union.
//!
//! [`EventBus`] decouples the conductor from Tauri: production wires
//! [`TauriEventBus`] but tests can plug a [`NoopEventBus`] or a recording
//! mock.

use crate::types::{
    AgentRole, ConductorState, EventRow, IterationMode, QuestionResolution,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

/// Every event the system can emit. Tagged externally via `type` so the
/// frontend can pattern-match on a discriminated union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    // ---- Agent runtime ----
    AgentStart {
        role: AgentRole,
    },
    AgentMessage {
        role: AgentRole,
        text: String,
    },
    AgentToolUse {
        role: AgentRole,
        tool: String,
        input: serde_json::Value,
    },
    AgentToolResult {
        role: AgentRole,
        content: String,
        is_error: bool,
    },
    AgentEnd {
        role: AgentRole,
        turns: u32,
        duration_ms: u64,
    },
    AgentError {
        role: AgentRole,
        message: String,
    },

    // ---- Conductor state machine ----
    StateChange {
        state: ConductorState,
    },

    // ---- Iteration boundaries ----
    IterationStart {
        number: i64,
        mode: IterationMode,
    },
    IterationEnd {
        mode: IterationMode,
        demoable: Option<bool>,
        summary: String,
    },
    IterationError {
        error: String,
    },

    // ---- Iteration stages (diagnostics) ----
    ResumeIteration {
        number: i64,
        po_done: bool,
        arch_done: bool,
        tasks_pending: usize,
        summary_done: bool,
    },
    PoSkippedResume {
        theme: String,
    },
    PoDone {
        theme: String,
        stories: usize,
    },
    ArchSkippedResume {
        tasks: usize,
    },
    ArchDone {
        tasks: usize,
        stack: String,
    },
    ReviewerFailed {
        error: String,
    },

    // ---- Wave runner ----
    WaveStarted {
        size: usize,
    },
    TasksSkipped {
        count: usize,
        reason: String,
    },
    GraphDeadlock,

    // ---- Worktree / merge ----
    WorktreeFailed {
        error: String,
    },
    MergeFailed {
        conflict: bool,
        message: String,
    },
    /// Specialist's branch couldn't rebase cleanly onto main. The Merge
    /// Resolver agent is being invoked to fix the conflict markers.
    MergeConflict {
        files: Vec<String>,
    },
    /// Merge Resolver finished. `summary` is the agent's free-form report
    /// of what it did. `ok=true` means rebase completed and the branch is
    /// now mergeable; `false` means resolver gave up (rebase was aborted).
    MergeResolved {
        ok: bool,
        summary: String,
    },

    /// Documenter finished its pass after this iteration. `summary` is its
    /// free-form report of which doc files it touched.
    DocsUpdated {
        summary: String,
    },

    // ---- ask_user routing ----
    AskUserInvoked {
        question: String,
        context: String,
    },
    QuestionAsked {
        question_id: String,
        question: String,
        context: String,
        reasoning: Option<String>,
    },
    QuestionAnswered {
        question_id: String,
        resolution: QuestionResolution,
        answer_preview: String,
        reasoning: Option<String>,
    },

    // ---- User directives ----
    WrapUpRequested,
    PresentationOnly,
    ResumeForPreview {
        iteration: i64,
    },
    Resumed,

    // ---- Preview lifecycle ----
    PreviewPrepDone,
    PreviewPrepFailed {
        error: String,
    },
    PreviewShutdownDone,
    PreviewShutdownSkipped {
        reason: String,
    },

    // ---- Loop / runtime errors ----
    Backoff {
        duration_ms: u64,
        consecutive: usize,
    },
    TooManyFailures {
        consecutive: usize,
    },
    LoopError {
        error: String,
    },

    // ---- Provider rate-limit cooldown ----
    /// Emitted when the conductor classifies an agent error as a provider
    /// rate-limit and decides to sleep instead of pausing. The UI uses
    /// `retry_at_ms` for the countdown.
    CooldownStarted {
        retry_at_ms: i64,
        reason: String,
    },
    /// Emitted when the cooldown sleep ended naturally (or because the
    /// user pressed Continue to skip the wait). The conductor then retries
    /// the same iteration.
    CooldownEnded {
        skipped_by_user: bool,
    },
    /// Emitted when the user pressed Stop during cooldown. The cooldown
    /// is abandoned and the conductor falls into regular Paused.
    CooldownCancelled,
}

impl EventPayload {
    /// Does this event change something the user can see on the dashboard?
    /// The renderer treats these as snapshot-refresh triggers; everything
    /// else (assistant text streams, tool noise) gets debounced. Kept on the
    /// Rust side as a single source of truth — the TS layer mirrors this list.
    #[allow(dead_code)]
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            Self::StateChange { .. }
                | Self::IterationStart { .. }
                | Self::IterationEnd { .. }
                | Self::IterationError { .. }
                | Self::AgentStart { .. }
                | Self::AgentEnd { .. }
                | Self::AgentError { .. }
                | Self::QuestionAsked { .. }
                | Self::QuestionAnswered { .. }
        )
    }

    /// The (optional) agent role this event is about. Used when persisting
    /// the row so filtering by agent works without parsing the payload.
    pub fn agent_role(&self) -> Option<AgentRole> {
        match self {
            Self::AgentStart { role }
            | Self::AgentMessage { role, .. }
            | Self::AgentToolUse { role, .. }
            | Self::AgentToolResult { role, .. }
            | Self::AgentEnd { role, .. }
            | Self::AgentError { role, .. } => Some(*role),
            _ => None,
        }
    }
}

/// Publish events to whoever is listening. Sync because the only sink we
/// have (Tauri's `app.emit`) is sync and microsecond-fast.
pub trait EventBus: Send + Sync {
    fn publish(&self, row: &EventRow);
}

/// Production sink: emit on the Tauri `"event"` channel.
pub struct TauriEventBus {
    app: AppHandle,
}

impl TauriEventBus {
    /// Wrap a Tauri `AppHandle` as an event-publishing trait object.
    pub fn arced(app: AppHandle) -> Arc<dyn EventBus> {
        Arc::new(Self { app })
    }
}

impl EventBus for TauriEventBus {
    fn publish(&self, row: &EventRow) {
        if let Err(e) = self.app.emit("event", row) {
            tracing::warn!(error = %e, "event publish failed");
        }
    }
}

/// Drop-all sink for tests that don't care about events.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct NoopEventBus;

#[cfg(test)]
impl EventBus for NoopEventBus {
    fn publish(&self, _row: &EventRow) {}
}
