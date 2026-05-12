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
    pub ended_at: Option<i64>,
    pub architect_id: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    AgentStart,
    AgentMessage,
    AgentToolUse,
    AgentToolResult,
    AgentEnd,
    AgentError,
    StateChange,
    IterationStart,
    IterationEnd,
    Directive,
    QuestionAsked,
    QuestionAnswered,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub project_id: String,
    pub iteration_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_role: Option<AgentRole>,
    pub r#type: EventType,
    pub payload: String, // JSON-encoded
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub url: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub setup_steps: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub errors: Vec<String>,
    pub logs_tail: String,
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
