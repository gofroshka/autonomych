//! Agents subsystem.
//!
//! Public surface:
//! - [`AgentInvocation`] — describes one agent run, regardless of backend.
//! - [`AgentEvent`] / [`AgentResult`] — what the runner streams / returns.
//! - [`run_agent`] — entry point. Dispatches to [`claude::run_claude_agent`]
//!   or [`codex::run_codex_agent`] based on `inv.backend`.
//!
//! Adding a third backend (e.g. local LLM via Ollama) is a matter of writing
//! a new private runner that produces the same [`AgentEvent`] stream and
//! adding a match arm to [`run_agent`].

pub mod claude;
pub mod codex;
pub mod prompts;

pub use claude::extract_json;
pub use prompts::{presenter_chat_prompt, specialist_full_prompt, system_prompt};

use crate::error::AppResult;
use crate::types::{AgentBackend, AgentRole, PermissionMode};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

pub const REVIEWER_TOOLS: &[&str] = &["Read", "Glob", "Grep", "Bash"];
pub const FULL_TOOLS: &[&str] = &["Read", "Write", "Edit", "Glob", "Grep", "Bash"];

/// Fully-resolved description of one agent run. Constructed by callers in
/// the conductor; consumed by [`run_agent`] which routes to the right
/// backend.
#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub role: AgentRole,
    pub system_prompt: String,
    pub user_prompt: String,
    pub cwd: PathBuf,
    pub model: String,
    /// Names of Claude Code's built-in tools to enable. Ignored by Codex,
    /// which surfaces tools through its own sandbox & MCP config.
    pub tools: Vec<String>,
    pub permission_mode: PermissionMode,
    pub max_turns: u32,
    /// Claude-specific: append vs replace the system prompt preset.
    /// Ignored by Codex.
    pub claude_code_preset: bool,
    pub cancel: Option<CancellationToken>,
    pub backend: AgentBackend,
}

/// Streamed event from a running agent. Both backends produce the same
/// shape so the conductor (and the renderer) don't need to care.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum AgentEvent {
    Start { role: AgentRole },
    AssistantText { role: AgentRole, text: String },
    ToolUse { role: AgentRole, tool: String, input: Value },
    ToolResult { role: AgentRole, content: String, is_error: bool },
    End { role: AgentRole, final_text: String, turns: u32, duration_ms: u64 },
    AgentError { role: AgentRole, message: String },
}

/// Final result of an agent invocation. `turns` and `duration_ms` are
/// exposed alongside the text so callers can log them; the running tally is
/// also emitted through [`AgentEvent::End`].
#[derive(Debug)]
pub struct AgentResult {
    pub final_text: String,
    #[allow(dead_code)]
    pub turns: u32,
    #[allow(dead_code)]
    pub duration_ms: u64,
}

/// Run an agent under whichever backend `inv.backend` requests. The two
/// backends produce identical event streams — only the spawned subprocess
/// and JSON protocol differ.
pub async fn run_agent<F>(inv: AgentInvocation, on_event: F) -> AppResult<AgentResult>
where
    F: FnMut(AgentEvent) + Send,
{
    match inv.backend {
        AgentBackend::ClaudeCode => claude::run_claude_agent(inv, on_event).await,
        AgentBackend::Codex => codex::run_codex_agent(inv, on_event).await,
    }
}

pub fn tools_for(role: AgentRole) -> Vec<String> {
    match role {
        AgentRole::SpecialistBackend
        | AgentRole::SpecialistFrontend
        | AgentRole::SpecialistDevops
        // Conflict resolver needs the full toolkit: read both sides, edit
        // files to remove markers, and run `git add` / `git rebase --continue`.
        | AgentRole::MergeResolver
        // Documenter reads the whole repo, edits markdown, commits via Bash.
        | AgentRole::Documenter => FULL_TOOLS.iter().map(|s| s.to_string()).collect(),
        AgentRole::Reviewer => REVIEWER_TOOLS.iter().map(|s| s.to_string()).collect(),
        // PO / Architect / Overseer all need to explore the project: read
        // docs we maintain, plus any existing README/docs in legacy
        // projects. Read-only — they don't write code.
        AgentRole::ProductOwner | AgentRole::Architect | AgentRole::Overseer => {
            vec!["Read".into(), "Glob".into(), "Grep".into()]
        }
        AgentRole::Presenter => vec![
            "Read".into(),
            "Glob".into(),
            "Grep".into(),
            "Bash".into(),
            "Write".into(),
        ],
        _ => vec![],
    }
}
