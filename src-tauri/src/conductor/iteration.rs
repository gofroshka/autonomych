//! Single iteration end to end: Product Owner → Architect → Specialist
//! waves → Reviewer. Designed to be resumable — every stage checks the store
//! for prior output before invoking its agent.

use super::outputs::{ArchOutput, ArchTask, PoOutput, ReviewerOutput};
use super::Conductor;
use crate::agents::{extract_json, run_agent, system_prompt, tools_for, AgentInvocation};
use crate::error::{AppError, AppResult};
use crate::events::EventPayload;
use crate::git;
use crate::store::Store;
use crate::types::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SNAPSHOT_FILES: &[&str] = &[
    "README.md",
    "package.json",
    "tsconfig.json",
    "pyproject.toml",
    "docker-compose.yml",
];
const SNAPSHOT_BYTES_PER_FILE: usize = 800;
const HISTORY_DEPTH: usize = 5;

impl Conductor {
    #[tracing::instrument(
        skip(self, iter),
        fields(
            project_id = %self.project_id,
            iteration = %iter.number,
            iteration_id = %iter.id,
        ),
    )]
    pub(super) async fn run_iteration(
        self: Arc<Self>,
        iter: IterationRow,
    ) -> AppResult<()> {
        let project = self.project_snapshot();
        let mode = iter.mode.unwrap_or(IterationMode::Normal);
        let mode_is_wrapup = matches!(mode, IterationMode::Wrapup);

        let existing_tasks = self.store.iteration_tasks(&iter.id);
        let is_resume = !existing_tasks.is_empty() || iter.theme.is_some();
        if is_resume {
            self.emit_for(
                EventPayload::ResumeIteration {
                    number: iter.number,
                    po_done: iter.theme.is_some(),
                    arch_done: !existing_tasks.is_empty(),
                    tasks_pending: existing_tasks
                        .iter()
                        .filter(|t| {
                            matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)
                        })
                        .count(),
                    summary_done: iter.summary.is_some(),
                },
                Some(iter.id.clone()),
                None,
            );
        } else if let Some(s) = self.store.pending_steering(&project.id) {
            let _ = self.store.consume_steering(&s.id, &iter.id);
        }

        let project_context = snapshot_project_files(&PathBuf::from(&project.root_path)).await;

        let po_output = self
            .run_po_stage(&project, &iter, mode_is_wrapup, &project_context)
            .await?;

        let arch_output = self
            .run_architect_stage(&project, &iter, &po_output, mode_is_wrapup, &project_context)
            .await?;

        self.clone()
            .execute_specialist_waves(&arch_output.tasks, &iter, &project)
            .await;

        let reviewer = self
            .run_reviewer_stage(&project, &iter, &po_output, mode_is_wrapup)
            .await;

        // Documenter is best-effort: a failure here doesn't fail the
        // iteration. Worst case the docs lag one iteration behind.
        if let Err(e) = self
            .run_documenter_stage(&project, &iter, &po_output, reviewer.as_ref())
            .await
        {
            tracing::warn!(error = %e, "documenter stage failed — docs may be stale");
        }

        let summary = match &reviewer {
            Some(r) => format!(
                "{} {}\n\n{}\n\nRisks: {}",
                if r.demoable.unwrap_or(false) {
                    "✓"
                } else {
                    "✗"
                },
                po_output.iteration_theme,
                r.changelog,
                r.risks
            ),
            None => format!("Итерация #{}: ревью не получено", iter.number),
        };
        let final_status = if mode_is_wrapup {
            IterationStatus::Presented
        } else {
            IterationStatus::Completed
        };
        self.store
            .set_iteration_status(&iter.id, final_status, Some(&summary))?;
        self.emit_for(
            EventPayload::IterationEnd {
                mode,
                demoable: reviewer.as_ref().and_then(|r| r.demoable),
                summary,
            },
            Some(iter.id.clone()),
            None,
        );
        Ok(())
    }

    async fn run_po_stage(
        &self,
        project: &ProjectRow,
        iter: &IterationRow,
        mode_is_wrapup: bool,
        project_context: &str,
    ) -> AppResult<PoOutput> {
        // Resume short-circuit: PO already produced theme + stories.
        if let (Some(theme), false) = (iter.theme.clone(), iter.stories.is_empty()) {
            self.emit_for(
                EventPayload::PoSkippedResume {
                    theme: theme.clone(),
                },
                Some(iter.id.clone()),
                None,
            );
            return Ok(PoOutput {
                iteration_theme: theme,
                rationale: iter.rationale.clone().unwrap_or_default(),
                stories: iter.stories.clone(),
            });
        }

        let steering = self.store.pending_steering(&project.id);
        let history = build_history_summary(&self.store, &project.id);
        let po_prompt = format!(
            "Идея проекта: {}\nИмя проекта: {}\n\nТекущая итерация: #{} (mode={})\n\n--- История последних итераций ---\n{}\n\n--- Снапшот файлов проекта ---\n{}\n\n{}\nВерни строго JSON по описанному формату.",
            project.idea,
            project.name,
            iter.number,
            if mode_is_wrapup { "wrapup" } else { "normal" },
            if history.is_empty() {
                "(нет — это первая итерация)".into()
            } else {
                history
            },
            if project_context.is_empty() {
                "(пусто, проект ещё не создан)".into()
            } else {
                project_context.to_string()
            },
            steering
                .as_ref()
                .map(|s| format!("--- USER_STEERING ({:?}) ---\n{}\n", s.mode, s.message))
                .unwrap_or_default()
        );
        let raw = self
            .run_json_agent(AgentRole::ProductOwner, mode_is_wrapup, po_prompt, project, iter)
            .await?;
        let parsed: PoOutput = extract_json(&raw).unwrap_or(PoOutput {
            iteration_theme: "(без темы)".into(),
            rationale: String::new(),
            stories: vec![],
        });
        self.store.set_iteration_meta(
            &iter.id,
            Some(parsed.iteration_theme.clone()),
            Some(parsed.rationale.clone()),
            Some(parsed.stories.clone()),
            None,
            None,
        )?;
        self.emit_for(
            EventPayload::PoDone {
                theme: parsed.iteration_theme.clone(),
                stories: parsed.stories.len(),
            },
            Some(iter.id.clone()),
            None,
        );
        if parsed.stories.is_empty() {
            return Err(AppError::Conductor("PO не вернул ни одной story".into()));
        }
        Ok(parsed)
    }

    async fn run_architect_stage(
        &self,
        project: &ProjectRow,
        iter: &IterationRow,
        po_output: &PoOutput,
        mode_is_wrapup: bool,
        project_context: &str,
    ) -> AppResult<ArchOutput> {
        // Resume short-circuit: tasks already exist in store.
        let reloaded = self.store.iteration_tasks(&iter.id);
        if !reloaded.is_empty() {
            let tasks = reloaded
                .iter()
                .filter_map(|t| {
                    t.architect_id.clone().map(|aid| ArchTask {
                        id: aid,
                        role: t.role,
                        title: t.title.clone(),
                        description: t.description.clone(),
                        depends_on: t.depends_on.clone(),
                    })
                })
                .collect::<Vec<_>>();
            self.emit_for(
                EventPayload::ArchSkippedResume {
                    tasks: tasks.len(),
                },
                Some(iter.id.clone()),
                None,
            );
            return Ok(ArchOutput {
                stack_notes: iter.stack_notes.clone().unwrap_or_default(),
                tasks,
            });
        }

        let arch_prompt = format!(
            "Тема итерации: {}\nОбоснование: {}\n\n--- User stories ---\n{}\n\n--- Снапшот проекта ---\n{}\n\nВерни строго JSON.",
            po_output.iteration_theme,
            po_output.rationale,
            serde_json::to_string_pretty(&po_output.stories).unwrap_or_default(),
            if project_context.is_empty() {
                "(проект пустой)".into()
            } else {
                project_context.to_string()
            }
        );
        let raw = self
            .run_json_agent(AgentRole::Architect, mode_is_wrapup, arch_prompt, project, iter)
            .await?;
        let parsed: ArchOutput = extract_json(&raw).unwrap_or(ArchOutput {
            stack_notes: String::new(),
            tasks: vec![],
        });
        self.store.set_iteration_meta(
            &iter.id,
            None,
            None,
            None,
            Some(parsed.stack_notes.clone()),
            None,
        )?;
        self.emit_for(
            EventPayload::ArchDone {
                tasks: parsed.tasks.len(),
                stack: parsed.stack_notes.clone(),
            },
            Some(iter.id.clone()),
            None,
        );
        if parsed.tasks.is_empty() {
            return Err(AppError::Conductor("Architect не вернул задач".into()));
        }
        let _ = git::tag(
            &PathBuf::from(&project.root_path),
            &format!("autonomych/pre-iter-{}", iter.number),
        )
        .await;
        for t in &parsed.tasks {
            self.store.create_task(
                &iter.id,
                t.role,
                t.title.clone(),
                t.description.clone(),
                Some(t.id.clone()),
                t.depends_on.clone(),
            )?;
        }
        Ok(parsed)
    }

    async fn run_reviewer_stage(
        &self,
        project: &ProjectRow,
        iter: &IterationRow,
        po_output: &PoOutput,
        mode_is_wrapup: bool,
    ) -> Option<ReviewerOutput> {
        let task_rows = self.store.iteration_tasks(&iter.id);
        let stories_list = po_output
            .stories
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{}. {}: {}",
                    i + 1,
                    s.title,
                    s.i_want.clone().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let tasks_list = task_rows
            .iter()
            .map(|r| format!("- [{:?}] {:?}: {}", r.status, r.role, r.title))
            .collect::<Vec<_>>()
            .join("\n");
        let reviewer_prompt = format!(
            "Итерация #{}. Тема: {}\nStories:\n{}\n\nВыполненные задачи:\n{}\n\nКорень проекта: {}. Ты можешь читать файлы и запускать команды.\nСделай проверку и верни строго JSON.",
            iter.number, po_output.iteration_theme, stories_list, tasks_list, project.root_path
        );

        match self
            .run_json_agent(AgentRole::Reviewer, mode_is_wrapup, reviewer_prompt, project, iter)
            .await
        {
            Ok(text) => extract_json::<ReviewerOutput>(&text).ok(),
            Err(e) => {
                self.emit_for(
                    EventPayload::ReviewerFailed {
                        error: e.to_string(),
                    },
                    Some(iter.id.clone()),
                    None,
                );
                None
            }
        }
    }

    /// Maintain the project's living documentation. Runs after the Reviewer
    /// in main worktree (all specialist branches already merged); the
    /// Documenter agent reads existing docs (ours + legacy), figures out
    /// what changed in this iteration, and writes/updates thematic files
    /// in `docs/`. Commits its own changes via Bash.
    async fn run_documenter_stage(
        &self,
        project: &ProjectRow,
        iter: &IterationRow,
        po_output: &PoOutput,
        reviewer: Option<&ReviewerOutput>,
    ) -> AppResult<()> {
        let stories_list = po_output
            .stories
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{}. {}{}",
                    i + 1,
                    s.title,
                    s.i_want
                        .as_ref()
                        .map(|w| format!(" — {w}"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let tasks = self.store.iteration_tasks(&iter.id);
        let tasks_list = tasks
            .iter()
            .map(|t| format!("- [{:?}] {:?}: {}", t.status, t.role, t.title))
            .collect::<Vec<_>>()
            .join("\n");
        let reviewer_note = reviewer
            .map(|r| {
                format!(
                    "\n\nReviewer changelog:\n{}\n\nReviewer risks:\n{}",
                    r.changelog, r.risks
                )
            })
            .unwrap_or_default();

        let prompt = format!(
            "Идея проекта: {idea}\nИмя: {name}\nИтерация #{n}, тема: {theme}\nКорень: {root} — ты в нём.\n\n--- User stories этой итерации ---\n{stories}\n\n--- Задачи и их статусы ---\n{tasks}{reviewer}\n\nОбнови распределённую документацию в `docs/` по алгоритму из системного промпта. Не забудь закоммитить.",
            idea = project.idea,
            name = project.name,
            n = iter.number,
            theme = po_output.iteration_theme,
            root = project.root_path,
            stories = stories_list,
            tasks = tasks_list,
            reviewer = reviewer_note,
        );

        let inv = AgentInvocation {
            role: AgentRole::Documenter,
            system_prompt: system_prompt(AgentRole::Documenter, false, false).to_string(),
            user_prompt: prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_specialist.clone(),
            tools: tools_for(AgentRole::Documenter),
            permission_mode: project.permission_mode,
            max_turns: 30,
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
        };
        let publisher = self.event_publisher();
        let iter_id = iter.id.clone();
        let res = run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, Some(iter_id.clone()), None);
        })
        .await?;

        let summary = res.final_text.trim().to_string();
        tracing::info!(
            iteration = iter.number,
            summary_len = summary.len(),
            "documenter: stage done",
        );
        self.emit_for(
            EventPayload::DocsUpdated { summary },
            Some(iter.id.clone()),
            None,
        );
        Ok(())
    }

    /// Run a one-shot agent that returns JSON. Used by PO / Architect /
    /// Reviewer — all of which return their state in the final message and
    /// don't need long tool-using sessions.
    async fn run_json_agent(
        &self,
        role: AgentRole,
        mode_is_wrapup: bool,
        user_prompt: String,
        project: &ProjectRow,
        iter: &IterationRow,
    ) -> AppResult<String> {
        let inv = AgentInvocation {
            role,
            system_prompt: system_prompt(role, mode_is_wrapup, false).to_string(),
            user_prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_pm.clone(),
            tools: tools_for(role),
            permission_mode: if matches!(project.permission_mode, PermissionMode::BypassPermissions)
            {
                PermissionMode::BypassPermissions
            } else {
                PermissionMode::AcceptEdits
            },
            // PO/Architect/Reviewer all need tool-using turns to explore
            // docs and source before producing JSON. Reviewer historically
            // needed the most (it also runs build/tests); PO/Architect now
            // also read files, so bump them above the old 5-turn cap.
            max_turns: match role {
                AgentRole::Reviewer => 15,
                AgentRole::ProductOwner | AgentRole::Architect => 12,
                _ => 5,
            },
            // claude_code_preset must be true for any role that uses Read/
            // Glob/Grep — it pulls in Claude Code's tool-use scaffolding.
            claude_code_preset: matches!(
                role,
                AgentRole::Reviewer | AgentRole::ProductOwner | AgentRole::Architect
            ),
            cancel: Some(self.cancel_token()),
        };
        let publisher = self.event_publisher();
        let iter_id = iter.id.clone();
        let res = run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, Some(iter_id.clone()), None);
        })
        .await?;
        Ok(res.final_text)
    }
}

/// Cheap project snapshot — just a few well-known config files capped at
/// ~800 chars each. Cheap, deterministic, and good enough for grounding the
/// PO / Architect prompts without blowing the token budget.
async fn snapshot_project_files(root: &Path) -> String {
    use tokio::fs;
    let mut chunks = Vec::new();
    for cand in SNAPSHOT_FILES {
        let p = root.join(cand);
        if let Ok(content) = fs::read_to_string(&p).await {
            let snippet: String = content.chars().take(SNAPSHOT_BYTES_PER_FILE).collect();
            chunks.push(format!("### {cand}\n```\n{snippet}\n```"));
        }
    }
    chunks.join("\n\n")
}

fn build_history_summary(store: &Store, project_id: &str) -> String {
    store
        .recent_iterations(project_id, HISTORY_DEPTH)
        .into_iter()
        .filter(|i| i.summary.is_some())
        .map(|i| {
            format!(
                "### Итерация #{} [{:?}]\n{}",
                i.number,
                i.status,
                i.summary.unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
