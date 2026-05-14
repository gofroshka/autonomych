//! JSON shapes that agents return in their final message. Each one is parsed
//! via `agents::extract_json`. Keeping them in one place makes the agent
//! contract easy to audit.

use crate::types::{AgentRole, BacklogCategory, BacklogPriority, IterationStory};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct PoOutput {
    pub iteration_theme: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub stories: Vec<IterationStory>,
    /// IDs of backlog items PO is addressing this iteration. The conductor
    /// flips those rows to InIteration after parsing this output. Invalid
    /// or unknown ids are ignored — PO might hallucinate, we don't want
    /// orphan state.
    #[serde(default)]
    pub picked_backlog_ids: Vec<String>,
    /// New items PO wants to park for later iterations (not picked now).
    /// Created as Pending backlog rows.
    #[serde(default)]
    pub add_to_backlog: Vec<PoBacklogProposal>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PoBacklogProposal {
    pub title: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub category: BacklogCategory,
    #[serde(default)]
    pub priority: BacklogPriority,
}

#[derive(Debug, Deserialize)]
pub(super) struct ArchOutput {
    #[serde(default)]
    pub stack_notes: String,
    #[serde(default)]
    pub tasks: Vec<ArchTask>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct ArchTask {
    pub id: String,
    pub role: AgentRole,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReviewerOutput {
    pub demoable: Option<bool>,
    #[serde(default)]
    pub changelog: String,
    #[serde(default)]
    pub risks: String,
    /// IDs of backlog items the reviewer considers actually done. Anything
    /// PO picked but isn't listed here stays open for the next iteration.
    #[serde(default)]
    pub closed_backlog_ids: Vec<String>,
    /// New items reviewer surfaced — typically follow-up risks or
    /// incomplete pieces. Created as Pending backlog rows.
    #[serde(default)]
    pub add_to_backlog: Vec<ReviewerBacklogProposal>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReviewerBacklogProposal {
    pub title: String,
    #[serde(default)]
    pub details: String,
    /// Defaults to `Bug` because that's what Reviewer typically surfaces;
    /// the agent can override to `tech_debt` / `critical` etc. in its JSON.
    #[serde(default = "default_reviewer_category")]
    pub category: BacklogCategory,
    #[serde(default)]
    pub priority: BacklogPriority,
}

fn default_reviewer_category() -> BacklogCategory {
    BacklogCategory::Bug
}
