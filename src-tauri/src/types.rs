//! Shared data types — mirror of TS shared/types.ts. serde uses lowercase /
//! snake_case so the JSON shape matches what the renderer expects.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConductorState {
    Idle,
    Running,
    WrappingUp,
    PreparingPreview,
    Presenting,
    Resuming,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    ProductOwner,
    Architect,
    SpecialistBackend,
    SpecialistFrontend,
    SpecialistDevops,
    Reviewer,
    BlockerReviewer,
    Overseer,
    Presenter,
    /// Resolves git conflicts that arise when a specialist's branch can't
    /// be rebased onto main cleanly. Runs inside the specialist's worktree
    /// with the rebase already paused on conflicts.
    MergeResolver,
    /// Maintains the project's living documentation (docs/PRODUCT.md and
    /// docs/TECH.md). Runs once per iteration after the Reviewer signs off;
    /// its output is what PO and Architect read on the next round.
    Documenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IterationStatus {
    Running,
    WrappingUp,
    Presented,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IterationMode {
    Normal,
    Wrapup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub idea: String,
    pub root_path: String,
    pub state: ConductorState,
    pub created_at: i64,
    pub model_pm: String,
    pub model_specialist: String,
    pub permission_mode: PermissionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationStory {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i_want: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub so_that: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance_criteria: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationRow {
    pub id: String,
    pub project_id: String,
    pub number: i64,
    pub status: IterationStatus,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub summary: Option<String>,
    pub theme: Option<String>,
    pub rationale: Option<String>,
    #[serde(default)]
    pub stories: Vec<IterationStory>,
    pub stack_notes: Option<String>,
    pub mode: Option<IterationMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRow {
    pub id: String,
    pub iteration_id: String,
    pub role: AgentRole,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    pub created_at: i64,
    /// Set when the task first transitions to `InProgress`. Used by the UI
    /// to show a live elapsed timer that reflects actual run time, not the
    /// architect's row-creation time. `#[serde(default)]` keeps backward
    /// compat with pre-existing tasks files.
    #[serde(default)]
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub architect_id: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// A single event in a project's history. The shape of `payload` is a
/// typed discriminated union — see [`crate::events::EventPayload`]. We don't
/// duplicate the variant tag into a separate `type` field because the
/// payload itself carries it via serde's external tagging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub project_id: String,
    pub iteration_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_role: Option<AgentRole>,
    pub payload: crate::events::EventPayload,
    pub ts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteeringMode {
    Soft,
    Override,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteeringRow {
    pub id: String,
    pub project_id: String,
    pub message: String,
    pub mode: SteeringMode,
    pub applied_iteration_id: Option<String>,
    pub ts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    Pending,
    AutoAnswered,
    Answered,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionResolution {
    User,
    Reviewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionRow {
    pub id: String,
    pub project_id: String,
    pub iteration_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_role: Option<AgentRole>,
    pub question: String,
    pub context: String,
    pub status: QuestionStatus,
    pub resolution: Option<QuestionResolution>,
    pub answer: Option<String>,
    pub created_at: i64,
    pub answered_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRow {
    pub id: String,
    pub project_id: String,
    pub role: ChatRole,
    pub text: String,
    pub ts: i64,
    #[serde(default)]
    pub error: Option<String>,
}

/// What the renderer needs to display about the preview. Minimal by design:
/// process management (starting/stopping/healthchecking the dev server) lives
/// entirely in the Presenter agent. We only show its free-form text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewStatus {
    /// Free-form instructions for the user, rendered verbatim (with URLs
    /// auto-linkified by the frontend). `None` means "not prepared yet".
    pub instructions: Option<String>,
    pub prepared_at: Option<i64>,
    pub prep_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub project: Option<ProjectRow>,
    pub iteration: Option<IterationRow>,
    pub tasks: Vec<TaskRow>,
    pub recent_events: Vec<EventRow>,
    pub pending_steering: Option<SteeringRow>,
    pub pending_questions: Vec<QuestionRow>,
    pub preview: PreviewStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectInput {
    pub name: String,
    pub idea: String,
    pub root_path: String,
    #[serde(default)]
    pub model_pm: Option<String>,
    #[serde(default)]
    pub model_specialist: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub iteration: IterationRow,
    pub tasks: Vec<TaskRow>,
}
