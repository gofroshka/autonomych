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
        // Wave runner exits cleanly on cancel and swallows errors. Without
        // this guard a user-Stop mid-specialist-phase would silently sail
        // past Reviewer/Documenter, both of which also swallow their own
        // errors, and the iteration would get marked Completed with an
        // empty summary — making it unresumable on the next Start.
        if self.is_cancelled() {
            return Err(AppError::Conductor(
                "iteration cancelled mid-specialist-phase".into(),
            ));
        }

        let reviewer = self
            .run_reviewer_stage(&project, &iter, &po_output, mode_is_wrapup)
            .await;
        // run_reviewer_stage maps errors to `None`. Distinguish "reviewer
        // genuinely had nothing to say" from "user pressed Stop while
        // reviewer was running" by checking the cancel token directly.
        if self.is_cancelled() {
            return Err(AppError::Conductor(
                "iteration cancelled during reviewer".into(),
            ));
        }

        // Documenter is best-effort: a failure here doesn't fail the
        // iteration. Worst case the docs lag one iteration behind.
        if let Err(e) = self
            .run_documenter_stage(&project, &iter, &po_output, reviewer.as_ref())
            .await
        {
            tracing::warn!(error = %e, "documenter stage failed — docs may be stale");
        }
        if self.is_cancelled() {
            return Err(AppError::Conductor(
                "iteration cancelled after documenter".into(),
            ));
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
            // Distinguish from a clean "iteration succeeded" so PO of the
            // next iteration sees this in history and treats outcome as
            // "unverified" rather than "everything's fine". The activity
            // log also shows the ReviewerFailed event with the raw text.
            None => format!(
                "⚠ Итерация #{}: тема «{}» — Reviewer не дал валидного JSON-вердикта, выполнение задач не верифицировано. Беклог-айтемы возвращены в Pending для повторной проверки в следующей итерации.",
                iter.number, po_output.iteration_theme
            ),
        };
        // Auto-populate the backlog from this iteration's tail end:
        //   - Failed tasks → re-attempt material (one item per task, dedup by
        //     `origin_task_id` so a re-run of the same task doesn't fan out).
        //   - Skipped-because-dep-failed → not added: those are cascade
        //     effects, not architectural debt; once the root cause is fixed
        //     they get scheduled normally.
        // Then revert any backlog items the Reviewer didn't close — they
        // stay Pending, visible in the next iteration's PO prompt.
        let tail_tasks = self.store.iteration_tasks(&iter.id);
        for t in tail_tasks.iter().filter(|t| matches!(t.status, TaskStatus::Failed)) {
            let _ = self.store.add_backlog_for_task_if_missing(
                &project.id,
                NewBacklogItem {
                    title: format!("Re-attempt: {}", t.title),
                    details: format!(
                        "Task упал в итерации #{}.\nРоль: {:?}\nОписание исходной задачи: {}",
                        iter.number, t.role, t.description
                    ),
                    source: BacklogSource::FailedTask,
                    category: BacklogCategory::Bug,
                    priority: BacklogPriority::High,
                    origin_iteration_id: Some(iter.id.clone()),
                    origin_task_id: Some(t.id.clone()),
                },
            );
        }
        if let Err(e) = self.store.revert_backlog_for_iteration(&iter.id) {
            tracing::warn!(error = %e, "could not revert backlog after iteration");
        }

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
                picked_backlog_ids: vec![],
                add_to_backlog: vec![],
            });
        }

        let history = build_history_summary(&self.store, &project.id);
        let backlog = self.store.active_backlog(&project.id);
        let backlog_section = format_backlog_for_po(&backlog);
        // No more static `project.idea` injection — that field is just a
        // creation note now. The living source of truth is
        // `docs/product/vision.md`, which Documenter rewrites as the
        // project evolves and PO reads explicitly via its
        // doc-exploration step (see system prompt). Imagine the user
        // pivoted from "CRM for landscapers" to "scheduling SaaS" 10
        // iterations in — the original idea would mislead PO; vision.md
        // would reflect the truth.
        let po_prompt = format!(
            "Имя проекта: {}\n\nТекущая итерация: #{} (mode={})\n\n--- История последних итераций ---\n{}\n\n--- BACKLOG ---\n{}\n\n--- Снапшот ключевых файлов проекта ---\n{}\n\nПЕРЕД ЛЮБЫМ РЕШЕНИЕМ прочитай `docs/product/vision.md` — это актуальное видение продукта (его поддерживает Documenter). Если в проекте уже есть `docs/INDEX.md` — ходи по INDEX'ам, не читай всё подряд.\n\nВерни строго JSON по описанному формату.",
            project.name,
            iter.number,
            if mode_is_wrapup { "wrapup" } else { "normal" },
            if history.is_empty() {
                "(нет — это первая итерация)".into()
            } else {
                history
            },
            backlog_section,
            if project_context.is_empty() {
                "(пусто, проект ещё не создан)".into()
            } else {
                project_context.to_string()
            }
        );
        let raw = self
            .run_json_agent(AgentRole::ProductOwner, mode_is_wrapup, po_prompt, project, iter)
            .await?;
        let parsed: PoOutput = extract_json(&raw).unwrap_or(PoOutput {
            iteration_theme: "(без темы)".into(),
            rationale: String::new(),
            stories: vec![],
            picked_backlog_ids: vec![],
            add_to_backlog: vec![],
        });
        self.store.set_iteration_meta(
            &iter.id,
            Some(parsed.iteration_theme.clone()),
            Some(parsed.rationale.clone()),
            Some(parsed.stories.clone()),
            None,
            None,
        )?;
        // Filter PO's picks to the set we actually showed it. Anything
        // else (PO hallucinating ids, already-closed items) is ignored to
        // avoid silently creating orphan state.
        let valid_picks: Vec<String> = {
            let active_ids: std::collections::HashSet<&str> =
                backlog.iter().map(|b| b.id.as_str()).collect();
            parsed
                .picked_backlog_ids
                .iter()
                .filter(|id| active_ids.contains(id.as_str()))
                .cloned()
                .collect()
        };
        if !valid_picks.is_empty() {
            if let Err(e) = self.store.pick_backlog(&valid_picks, &iter.id) {
                tracing::warn!(error = %e, "po: pick_backlog failed");
            }
        }
        // PO's parking-lot proposals — created as fresh Pending items.
        for prop in &parsed.add_to_backlog {
            let _ = self.store.add_backlog(
                &project.id,
                NewBacklogItem {
                    title: prop.title.clone(),
                    details: prop.details.clone(),
                    source: BacklogSource::PoCarryover,
                    category: prop.category,
                    priority: prop.priority,
                    origin_iteration_id: Some(iter.id.clone()),
                    origin_task_id: None,
                },
            );
        }
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
        // Backlog items PO picked into this iteration — Reviewer needs to
        // know which to consider for closure. Format compactly with ids
        // so the LLM can reference them in `closed_backlog_ids`.
        let in_iteration_backlog: Vec<_> = self
            .store
            .list_backlog(&project.id)
            .into_iter()
            .filter(|b| {
                b.picked_in_iteration_id.as_deref() == Some(&iter.id)
                    && matches!(b.status, BacklogStatus::InIteration)
            })
            .collect();
        let backlog_section = format_backlog_for_reviewer(&in_iteration_backlog);

        let reviewer_prompt = format!(
            "Итерация #{}. Тема: {}\nStories:\n{}\n\nВыполненные задачи:\n{}\n\n--- BACKLOG_IN_ITERATION ---\n{}\n\nКорень проекта: {}. Ты можешь читать файлы и запускать команды.\nСделай проверку и верни строго JSON.",
            iter.number,
            po_output.iteration_theme,
            stories_list,
            tasks_list,
            backlog_section,
            project.root_path
        );

        let parsed = match self
            .run_json_agent(AgentRole::Reviewer, mode_is_wrapup, reviewer_prompt, project, iter)
            .await
        {
            Ok(text) => match extract_json::<ReviewerOutput>(&text) {
                Ok(v) => Some(v),
                Err(parse_err) => {
                    // The agent ran successfully but produced unparseable
                    // output. Without surfacing this, the iteration silently
                    // gets `summary = "ревью не получено"` AND the backlog
                    // items PO picked never get closed — they auto-revert
                    // to Pending at iteration end and re-appear as Active.
                    // Both look like ghost behaviour to the user. Log raw
                    // text + emit a structured event so it's visible.
                    let snippet: String = text.chars().take(800).collect();
                    tracing::warn!(
                        iteration = iter.number,
                        error = %parse_err,
                        raw = %snippet,
                        "reviewer JSON failed to parse — full text in stderr above; backlog items will revert to Pending",
                    );
                    self.emit_for(
                        EventPayload::ReviewerFailed {
                            error: format!(
                                "{parse_err}. Raw output (first 600 chars): {}",
                                text.chars().take(600).collect::<String>()
                            ),
                        },
                        Some(iter.id.clone()),
                        None,
                    );
                    None
                }
            },
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
        };

        // Apply backlog side-effects from Reviewer's verdict. If `parsed`
        // is None (JSON didn't parse), the ReviewerFailed event above
        // already surfaced the raw text to the user — items will revert to
        // Pending by the iteration finaliser and re-appear in backlog for
        // the next round.
        if let Some(out) = parsed.as_ref() {
            let picked_ids: std::collections::HashSet<&str> = in_iteration_backlog
                .iter()
                .map(|b| b.id.as_str())
                .collect();
            let valid_closures: Vec<String> = out
                .closed_backlog_ids
                .iter()
                .filter(|id| picked_ids.contains(id.as_str()))
                .cloned()
                .collect();
            if !valid_closures.is_empty() {
                if let Err(e) = self.store.close_backlog(&valid_closures) {
                    tracing::warn!(error = %e, "reviewer: close_backlog failed");
                }
            }
            for prop in &out.add_to_backlog {
                let _ = self.store.add_backlog(
                    &project.id,
                    NewBacklogItem {
                        title: prop.title.clone(),
                        details: prop.details.clone(),
                        source: BacklogSource::ReviewerRisk,
                        category: prop.category,
                        priority: prop.priority,
                        origin_iteration_id: Some(iter.id.clone()),
                        origin_task_id: None,
                    },
                );
            }
        }
        parsed
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
            "Заметка пользователя при создании проекта (МОЖЕТ БЫТЬ УСТАРЕЛОЙ, проект мог пойти в другую сторону — сверяйся с кодом и докой): {idea}\nИмя проекта: {name}\nИтерация #{n}, тема: {theme}\nКорень: {root} — ты в нём.\n\n--- User stories этой итерации ---\n{stories}\n\n--- Задачи и их статусы ---\n{tasks}{reviewer}\n\nОбнови распределённую документацию в `docs/` по алгоритму из системного промпта. Не забудь обновить `docs/product/vision.md` если видение продукта эволюционировало (новая ниша/аудитория/механика), и закоммитить.",
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
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
            backend: project.agent_backend,
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
            // claude_code_preset must be true for any role that uses Read/
            // Glob/Grep — it pulls in Claude Code's tool-use scaffolding.
            claude_code_preset: matches!(
                role,
                AgentRole::Reviewer | AgentRole::ProductOwner | AgentRole::Architect
            ),
            cancel: Some(self.cancel_token()),
            backend: project.agent_backend,
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
    let iters: Vec<_> = store
        .recent_iterations(project_id, HISTORY_DEPTH)
        .into_iter()
        .filter(|i| i.summary.is_some())
        .collect();
    if iters.is_empty() {
        return String::new();
    }

    let themes: Vec<&str> = iters
        .iter()
        .filter_map(|i| i.theme.as_deref())
        .collect();
    let loop_warning = detect_repeating_theme(&themes);

    let body = iters
        .iter()
        .map(|i| {
            let tasks = store.iteration_tasks(&i.id);
            let task_summary = if tasks.is_empty() {
                String::new()
            } else {
                let total = tasks.len();
                let done = tasks
                    .iter()
                    .filter(|t| matches!(t.status, TaskStatus::Done))
                    .count();
                let failed = tasks
                    .iter()
                    .filter(|t| matches!(t.status, TaskStatus::Failed))
                    .count();
                let skipped = tasks
                    .iter()
                    .filter(|t| matches!(t.status, TaskStatus::Skipped))
                    .count();
                let pending = tasks
                    .iter()
                    .filter(|t| {
                        matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)
                    })
                    .count();
                let problem_tasks: Vec<String> = tasks
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.status,
                            TaskStatus::Failed | TaskStatus::Skipped | TaskStatus::Pending
                        )
                    })
                    .map(|t| format!("  - [{:?}] {:?}: {}", t.status, t.role, t.title))
                    .collect();
                let mut s = format!(
                    "\nТаски: {done}/{total} done, {failed} failed, {skipped} skipped, {pending} pending"
                );
                if !problem_tasks.is_empty() {
                    s.push_str("\nНезакрытые:\n");
                    s.push_str(&problem_tasks.join("\n"));
                }
                s
            };
            format!(
                "### Итерация #{} [{:?}]\n{}{}",
                i.number,
                i.status,
                i.summary.clone().unwrap_or_default(),
                task_summary,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    match loop_warning {
        Some(msg) => format!(
            "⚠️ ВНИМАНИЕ — ВОЗМОЖНАЯ ПЕТЛЯ: {msg}.\n\
             Даже если итерации помечены ✓, повторение темы — сигнал что подход не работает \
             либо ты циклишь. НЕ выбирай эту же тему снова. Переключись на другую область проекта, \
             удали проблемную фичу, или поставь ASK_USER. См. секцию «Защита от петель» в системном промпте.\n\n\
             ---\n\n{body}"
        ),
        None => body,
    }
}

/// Compact backlog rendering for PO's prompt, grouped by category so the
/// prioritisation rule ("close critical+bug before features") is obvious
/// at a glance. Items are already sorted by `active_backlog` (category,
/// priority, recency).
fn format_backlog_for_po(items: &[BacklogItem]) -> String {
    if items.is_empty() {
        return "(беклог пуст — это первая итерация или всё закрыто; можешь предложить story сам из идеи проекта)".into();
    }

    let category_label = |c: BacklogCategory| match c {
        BacklogCategory::Critical => "🚨 CRITICAL — БЛОКЕРЫ (фиксить в первую очередь, ничего другого не брать)",
        BacklogCategory::Bug => "🐛 BUG — известные баги (брать после critical, перед фичами)",
        BacklogCategory::TechDebt => "🔧 TECH_DEBT — техдолг и риски",
        BacklogCategory::Feature => "✨ FEATURE — новый функционал",
        BacklogCategory::Wish => "💭 WISH — пожелания / nice-to-have",
    };

    // Sweep through pre-sorted items and emit a heading every time category
    // changes — preserves the priority order while making it readable.
    let mut out = String::new();
    let mut current: Option<BacklogCategory> = None;
    for b in items {
        if current != Some(b.category) {
            if current.is_some() {
                out.push('\n');
            }
            out.push_str("\n## ");
            out.push_str(category_label(b.category));
            out.push('\n');
            current = Some(b.category);
        }
        let status_tag = match b.status {
            BacklogStatus::InIteration => " [уже взят в эту итерацию]",
            _ => "",
        };
        let details_line = if b.details.trim().is_empty() {
            String::new()
        } else {
            format!(
                "\n  details: {}",
                b.details.chars().take(240).collect::<String>()
            )
        };
        out.push_str(&format!(
            "[{}] ({:?}, src={:?}) {}{}{}\n",
            b.id, b.priority, b.source, b.title, status_tag, details_line
        ));
    }
    out
}

/// Backlog view for the Reviewer — only the items PO picked into the
/// current iteration. Reviewer references these ids in `closed_backlog_ids`.
fn format_backlog_for_reviewer(items: &[BacklogItem]) -> String {
    if items.is_empty() {
        return "(PO не выбрал ни одного backlog-айтема в эту итерацию — оцени только по stories выше)".into();
    }
    items
        .iter()
        .map(|b| {
            let details_line = if b.details.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "\n  details: {}",
                    b.details.chars().take(240).collect::<String>()
                )
            };
            format!(
                "[{}] ({:?}, src={:?}) {}{}",
                b.id, b.category, b.source, b.title, details_line
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Naive heuristic: count which lowercased word-tokens (length > 4, excluding
/// a small stoplist) appear in 3+ of the last N iteration themes. If any
/// content-bearing word repeats that often, we treat the project as stuck
/// in a loop and surface a warning to PO. Returns a short human-readable
/// description for the warning message, or None when no repetition was found.
fn detect_repeating_theme(themes: &[&str]) -> Option<String> {
    if themes.len() < 3 {
        return None;
    }
    // Generic words that are everywhere in iteration themes and would
    // produce false positives.
    const STOPWORDS: &[&str] = &[
        "фикс", "фикса", "фиксы", "фикси", "фиксинг",
        "доводка", "доделка", "доделки", "доделать",
        "итерация", "итерации",
        "система", "системы", "системе", "систему",
        "часть", "часта",
        "новая", "новый", "новое", "новые",
        "проект", "проекта", "проекте",
        "обновить", "обновление",
        "довести", "докрутить",
        "полный", "полная", "полное", "полные",
        "общая", "общий", "общее", "общие",
        "поддержка", "поддержки",
        "обработка", "обработки",
        "минимальный", "минимальная",
        "стабильность", "стабильности",
        "fix", "fixing", "fixes",
        "iteration",
        "feature", "features",
        "system", "systems",
        "project",
        "support",
        "improve", "improvement", "improvements",
        "general", "minor",
    ];
    let token_sets: Vec<std::collections::HashSet<String>> = themes
        .iter()
        .map(|t| {
            t.split(|c: char| !c.is_alphanumeric() && c != '_')
                .map(|w| w.to_lowercase())
                .filter(|w| w.chars().count() > 4 && !STOPWORDS.contains(&w.as_str()))
                .collect()
        })
        .collect();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for set in &token_sets {
        for w in set {
            *counts.entry(w.clone()).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, c)| *c >= 3)
        .max_by_key(|(_, c)| *c)
        .map(|(w, c)| format!("«{w}» встречается в {c} из последних {} итераций", themes.len()))
}

#[cfg(test)]
mod tests {
    use super::detect_repeating_theme;

    #[test]
    fn detects_repeated_keyword() {
        let themes = vec![
            "Гарантированный фикс is_active в схеме БД",
            "Фикс is_active: рабочая миграция и entrypoint",
            "Фикс is_active: синхронизация ORM-модели и схемы БД",
            "Фикс синхронизации дат + Docker-only демо",
            "Фикс воркера и стабильность Docker-инфраструктуры",
        ];
        let warn = detect_repeating_theme(&themes).expect("loop should be detected");
        assert!(warn.contains("is_active"));
    }

    #[test]
    fn no_loop_when_themes_diverse() {
        let themes = vec![
            "MVP: каркас фронта и бэка",
            "Аутентификация через JWT",
            "Каталог товаров",
            "Корзина и checkout",
            "Админ-панель",
        ];
        assert!(detect_repeating_theme(&themes).is_none());
    }

    #[test]
    fn stopwords_dont_trigger() {
        let themes = vec![
            "Фикс воркера",
            "Фикс БД",
            "Фикс системы",
        ];
        // «фикс» — стоп-слово, остальные различные → не петля
        assert!(detect_repeating_theme(&themes).is_none());
    }

    #[test]
    fn requires_three_or_more_overlapping() {
        let themes = vec!["Postgres миграции", "Postgres конфиг"];
        // только два — не считаем петлёй
        assert!(detect_repeating_theme(&themes).is_none());
    }
}
