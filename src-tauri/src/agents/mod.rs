pub mod claude;
pub mod prompts;

pub use claude::{extract_json, run_agent, AgentEvent, AgentInvocation};
pub use prompts::{specialist_full_prompt, system_prompt};

use crate::types::AgentRole;

pub const REVIEWER_TOOLS: &[&str] = &["Read", "Glob", "Grep", "Bash"];
pub const FULL_TOOLS: &[&str] = &["Read", "Write", "Edit", "Glob", "Grep", "Bash"];

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
