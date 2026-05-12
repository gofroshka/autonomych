//! JSON shapes that agents return in their final message. Each one is parsed
//! via `agents::extract_json`. Keeping them in one place makes the agent
//! contract easy to audit.

use crate::types::{AgentRole, IterationStory};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct PoOutput {
    pub iteration_theme: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub stories: Vec<IterationStory>,
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
}

