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
    /// Iteration paused mid-flight. Either the user pressed Stop, or an
    /// agent error broke the iteration before it finished. The iteration
    /// row stays `Running` in the store — clicking Start picks it up
    /// exactly where it stopped. The user can also swap models / CLI
    /// while paused.
    Paused,
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

/// Which agent CLI to spawn for every role's invocation in this project.
/// Per-project, decided at creation time. Defaults to ClaudeCode for
/// back-compat with existing projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackend {
    #[default]
    ClaudeCode,
    Codex,
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
    /// Free-form note the user wrote at creation, also seeded into
    /// `docs/product/vision.md`. Display only — agents read the doc.
    pub idea: String,
    pub root_path: String,
    pub state: ConductorState,
    pub created_at: i64,
    pub model_pm: String,
    pub model_specialist: String,
    pub permission_mode: PermissionMode,
    pub agent_backend: AgentBackend,
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
    /// architect's row-creation time.
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub architect_id: Option<String>,
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

// ===========================================================================
// Backlog — durable parking lot for "what to do next" items, sourced from
// the user, the Reviewer's flagged risks, failed/skipped tasks, and the
// Presenter's mid-demo bug reports. PO picks 1-3 items per iteration and
// produces stories from them; Reviewer closes items it deems done.
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogStatus {
    /// Parked, waiting for a future iteration.
    Pending,
    /// Currently being addressed in `picked_in_iteration_id`.
    InIteration,
    /// Reviewer signed off that this is done. Stays for history.
    Done,
    /// User or PO explicitly decided not to do it.
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogSource {
    /// Operator typed it (preview steering, "add to backlog" button).
    UserSteering,
    /// Reviewer's risks/next_iteration_hints field surfaced an item.
    ReviewerRisk,
    /// Task with status=Failed at iteration end → auto-added.
    FailedTask,
    /// Task with status=Skipped (dep failed or graph-deadlock).
    SkippedTask,
    /// Presenter chat noticed a code bug and drafted steering.
    PresenterBug,
    /// PO explicitly parked an idea via `add_to_backlog` in its output.
    PoCarryover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogPriority {
    High,
    #[default]
    Normal,
    Low,
}

/// What KIND of work the item is. Orthogonal to `source` (who put it there)
/// and `priority` (how urgent within its category). PO sorts the backlog
/// by category first — bugs/critical before features/wishes — so that
/// the system fixes itself before piling on new work, without us hard-
/// coding "fix this exact thing now" into the PO prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogCategory {
    /// Что-то блокирует работу или демо — фикс обязателен перед чем-либо ещё.
    Critical,
    /// Нерабочее, но не блокер — починить как можно раньше.
    Bug,
    /// Технический долг / риск / неполная реализация — приоритет ниже багов,
    /// но выше нового функционала.
    TechDebt,
    /// Новая фича, которую решили строить.
    Feature,
    /// Идея / пожелание / nice-to-have. Берётся когда всё остальное закрыто.
    #[default]
    Wish,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogItem {
    pub id: String,
    pub project_id: String,
    /// Short label shown in lists. PO writes stories from this.
    pub title: String,
    /// Free-form details — context, error message, reproduction, etc.
    pub details: String,
    pub source: BacklogSource,
    pub category: BacklogCategory,
    pub priority: BacklogPriority,
    pub status: BacklogStatus,
    pub created_at: i64,
    /// When `status == InIteration`, the iteration that owns it. Cleared
    /// when reviewer closes / pause-reverts the item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picked_in_iteration_id: Option<String>,
    /// First iteration that surfaced this item — useful for "this is the
    /// 3rd time we tried X" detection in PO's UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_iteration_id: Option<String>,
    /// For `FailedTask` / `SkippedTask` — points back at the task row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

/// What PO/UI sends in to create a new backlog item. id/created_at/status
/// are filled by the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBacklogItem {
    pub title: String,
    #[serde(default)]
    pub details: String,
    pub source: BacklogSource,
    #[serde(default)]
    pub category: BacklogCategory,
    #[serde(default)]
    pub priority: BacklogPriority,
    #[serde(default)]
    pub origin_iteration_id: Option<String>,
    #[serde(default)]
    pub origin_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub project: Option<ProjectRow>,
    pub iteration: Option<IterationRow>,
    pub tasks: Vec<TaskRow>,
    pub recent_events: Vec<EventRow>,
    pub pending_questions: Vec<QuestionRow>,
    pub preview: PreviewStatus,
    /// Non-null only when the conductor is sleeping out a rate-limit
    /// cooldown. Drives the "продолжим через MM:SS" countdown in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown: Option<crate::conductor::CooldownInfo>,
    /// Pending + currently-in-iteration backlog items for the project.
    /// Done/Dismissed items are filtered out here — UI can fetch them
    /// separately if needed.
    pub backlog: Vec<BacklogItem>,
}

/// Reply from the Presenter agent when the user reports an issue during a
/// running demo. The agent decides whether the issue is its own fault
/// (launch / env / port) and fixes it, or whether it's a real bug in the
/// iteration's code and the next iteration should pick it up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenterChatReply {
    /// Free-form markdown for the user. Always present.
    pub reply: String,
    /// If the issue is a code bug, the agent fills this with a suggested
    /// steering message for the next iteration. The UI offers to copy it
    /// into the course-correction box. `None` means the agent either fixed
    /// it itself or no steering is warranted.
    pub draft_steering: Option<String>,
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
    #[serde(default)]
    pub agent_backend: Option<AgentBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub iteration: IterationRow,
    pub tasks: Vec<TaskRow>,
}
