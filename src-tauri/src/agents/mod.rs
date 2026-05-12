pub mod claude;
pub mod prompts;

pub use claude::{extract_json, run_agent, AgentEvent, AgentInvocation, AgentResult};
pub use prompts::{specialist_full_prompt, system_prompt};

use crate::types::AgentRole;

pub const REVIEWER_TOOLS: &[&str] = &["Read", "Glob", "Grep", "Bash"];
pub const FULL_TOOLS: &[&str] = &["Read", "Write", "Edit", "Glob", "Grep", "Bash"];

pub fn tools_for(role: AgentRole) -> Vec<String> {
    match role {
        AgentRole::SpecialistBackend
        | AgentRole::SpecialistFrontend
        | AgentRole::SpecialistDevops => FULL_TOOLS.iter().map(|s| s.to_string()).collect(),
        AgentRole::Reviewer => REVIEWER_TOOLS.iter().map(|s| s.to_string()).collect(),
        AgentRole::Overseer => vec!["Read".into(), "Glob".into(), "Grep".into()],
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
