//! Specialist task lifecycle: spin up worktree, run agent, route blocker
//! questions through the Blocker Reviewer / user, commit, merge back.

use super::outputs::ArchTask;
use super::Conductor;
use crate::agents::{extract_json, run_agent, system_prompt, tools_for, AgentInvocation};
use crate::error::{AppError, AppResult};
use crate::events::EventPayload;
use crate::git;
use crate::types::*;
use crate::util::MutexExt;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::oneshot;

const MAX_ASK_RETRIES: usize = 2;

#[derive(Debug, Deserialize)]
pub(super) struct AskUserBlock {
    pub question: String,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Deserialize)]
struct BlockerVerdict {
    needs_user: bool,
    #[serde(default)]
    auto_answer: Option<String>,
    #[serde(default)]
    user_question: Option<String>,
    #[serde(default)]
    user_context: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
}

/// Find an ASK_USER_BEGIN..ASK_USER_END block at the end of an agent's final
/// text. The body may be wrapped in a ```json fence.
pub(super) fn parse_ask_user_marker(text: &str) -> Option<AskUserBlock> {
    let start = text.find("ASK_USER_BEGIN")?;
    let after = &text[start + "ASK_USER_BEGIN".len()..];
    let end = after.find("ASK_USER_END")?;
    let body = after[..end]
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    serde_json::from_str(body).ok()
}

impl Conductor {
    /// One specialist task end to end. Returns Ok(true) if it merged
    /// successfully, Ok(false) if it failed/aborted.
    #[tracing::instrument(
        skip(self, project, iter, arch, row),
        fields(
            project_id = %self.project_id,
            iteration = %iter.number,
            task_id = %row.id,
            role = ?arch.role,
            title = %arch.title,
        ),
    )]
    pub(super) async fn run_specialist_task(
        self: Arc<Self>,
        project: ProjectRow,
        iter: IterationRow,
        arch: ArchTask,
        row: TaskRow,
    ) -> AppResult<bool> {
        let task_started = std::time::Instant::now();
        let branch = format!("autonomych/iter-{}/{}-{}", iter.number, arch.id, row.id);
        let worktree_path = PathBuf::from(&project.root_path)
            .join(".autonomych")
            .join("worktrees")
            .join(format!("{}-{}", iter.number, arch.id));
        let root = PathBuf::from(&project.root_path);
        tracing::info!(
            task = %row.id,
            iteration = iter.number,
            role = ?arch.role,
            title = %arch.title,
            worktree = %worktree_path.display(),
            "task: started",
        );

        // Clean slate — previous run may have left worktree behind.
        let _ = git::remove_worktree(&root, &worktree_path).await;
        let _ = git::delete_branch(&root, &branch).await;
        if let Err(e) = git::create_worktree(&root, &branch, &worktree_path).await {
            tracing::error!(task = %row.id, error = %e, "task: worktree create failed");
            let _ = self.store.set_task_status(&row.id, TaskStatus::Failed);
            self.emit_for(
                EventPayload::WorktreeFailed {
                    error: e.to_string(),
                },
                Some(iter.id.clone()),
                Some(row.id.clone()),
            );
            return Ok(false);
        }
        tracing::debug!(task = %row.id, "task: worktree created");

        let _ = self.store.set_task_status(&row.id, TaskStatus::InProgress);

        let base_prompt = format!(
            "Задача: {}\n\nОписание / ТЗ:\n{}\n\nКонтекст: ты в выделенном git worktree ({}). Это копия репозитория проекта.\nКогда закончишь — кратко отчитайся текстом. Коммитить не нужно.",
            arch.title,
            arch.description,
            worktree_path.display()
        );

        let mut accumulated_answers: Vec<(String, String)> = Vec::new();
        let mut final_text = String::new();
        let mut agent_error: Option<AppError> = None;

        for attempt in 0..=MAX_ASK_RETRIES {
            let prompt = if accumulated_answers.is_empty() {
                base_prompt.clone()
            } else {
                let answers = accumulated_answers
                    .iter()
                    .map(|(q, a)| format!("Q: {q}\nA: {a}"))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                format!(
                    "{base_prompt}\n\n--- ОТВЕТЫ НА ТВОИ ASK_USER (учти их и продолжи) ---\n{answers}"
                )
            };
            let inv = AgentInvocation {
                role: arch.role,
                system_prompt: crate::agents::specialist_full_prompt(arch.role),
                user_prompt: prompt,
                cwd: worktree_path.clone(),
                model: project.model_specialist.clone(),
                tools: tools_for(arch.role),
                permission_mode: project.permission_mode,
                max_turns: 80,
                claude_code_preset: true,
                cancel: Some(self.cancel_token()),
            };
            let publisher = self.event_publisher();
            let iter_id_s = iter.id.clone();
            let task_id_s = row.id.clone();
            tracing::info!(task = %row.id, attempt, "task: invoking agent");
            let agent_started = std::time::Instant::now();
            let result = run_agent(inv, move |ev| {
                publisher.publish_agent_event(ev, Some(iter_id_s.clone()), Some(task_id_s.clone()));
            })
            .await;
            tracing::info!(
                task = %row.id,
                attempt,
                agent_ms = agent_started.elapsed().as_millis() as u64,
                ok = result.is_ok(),
                "task: agent returned",
            );
            match result {
                Ok(r) => {
                    final_text = r.final_text;
                    if let Some(ask) = parse_ask_user_marker(&final_text) {
                        if attempt < MAX_ASK_RETRIES {
                            tracing::info!(task = %row.id, attempt, "task: agent asked user");
                            let answer = self
                                .handle_ask_user(
                                    ask.question.clone(),
                                    ask.context.clone(),
                                    iter.clone(),
                                    row.id.clone(),
                                    arch.role,
                                )
                                .await
                                .unwrap_or_else(|_| {
                                    "[Blocker Reviewer / user response unavailable]".into()
                                });
                            accumulated_answers.push((ask.question, answer));
                            continue;
                        }
                    }
                    break;
                }
                Err(e) => {
                    tracing::warn!(task = %row.id, attempt, error = %e, "task: agent errored");
                    agent_error = Some(e);
                    break;
                }
            }
        }
        let _ = final_text;

        let outcome = match agent_error {
            None => {
                tracing::debug!(task = %row.id, "task: committing");
                let commit_started = std::time::Instant::now();
                let _ = git::commit_all(
                    &worktree_path,
                    &format!("iter-{} {:?}: {}", iter.number, arch.role, arch.title),
                )
                .await;
                tracing::debug!(task = %row.id, commit_ms = commit_started.elapsed().as_millis() as u64, "task: committed");

                let merged = self
                    .clone()
                    .integrate_branch(&project, &iter, &arch, &row, &worktree_path, &branch, &root)
                    .await;
                let _ = self.store.set_task_status(
                    &row.id,
                    if merged { TaskStatus::Done } else { TaskStatus::Failed },
                );
                Ok(merged)
            }
            Some(e) => {
                let cancelled = self.is_cancelled();
                let is_abort = cancelled || e.to_string().to_lowercase().contains("abort");
                if !is_abort {
                    let _ = self.store.set_task_status(&row.id, TaskStatus::Failed);
                    self.emit_for(
                        EventPayload::AgentError {
                            role: arch.role,
                            message: e.to_string(),
                        },
                        Some(iter.id.clone()),
                        Some(row.id.clone()),
                    );
                }
                Ok(false)
            }
        };

        if !self.is_cancelled() {
            let _ = git::remove_worktree(&root, &worktree_path).await;
            let _ = git::delete_branch(&root, &branch).await;
        }
        tracing::info!(
            task = %row.id,
            iteration = iter.number,
            role = ?arch.role,
            total_ms = task_started.elapsed().as_millis() as u64,
            outcome = ?outcome,
            "task: finished",
        );
        outcome
    }

    /// Integrate a specialist's branch into main.
    ///
    /// Flow:
    /// 1. Acquire the merge lock (only one specialist at a time touches main).
    /// 2. Clean up any leftover state in main from a prior aborted attempt.
    /// 3. Rebase the specialist's branch onto current main, inside their worktree.
    ///    - Clean → trivial fast-forward into main below.
    ///    - Conflict → invoke a Merge Resolver agent which sits in the same
    ///      worktree, resolves the conflicts, and finishes the rebase.
    ///    - Error → abort, mark task failed.
    /// 4. Fast-forward merge into main. After rebase this is guaranteed
    ///    not to conflict (branch is strictly ahead of main).
    ///
    /// Returns `true` if the work landed in main, `false` if anything in
    /// the chain failed (work stays on its own branch — caller decides what
    /// to do with the task status).
    #[allow(clippy::too_many_arguments)]
    async fn integrate_branch(
        self: Arc<Self>,
        project: &ProjectRow,
        iter: &IterationRow,
        arch: &ArchTask,
        row: &TaskRow,
        worktree_path: &std::path::Path,
        branch: &str,
        root: &std::path::Path,
    ) -> bool {
        tracing::debug!(task = %row.id, "merge: acquiring lock");
        let _merge_guard = self.merge_lock.lock().await;
        tracing::debug!(task = %row.id, "merge: lock acquired");
        git::cleanup_for_merge(root).await;

        let rebase_started = std::time::Instant::now();
        let rebase = git::rebase_onto(worktree_path, "main").await;
        tracing::info!(
            task = %row.id,
            rebase_ms = rebase_started.elapsed().as_millis() as u64,
            outcome = ?rebase,
            "merge: rebase onto main",
        );

        match rebase {
            git::RebaseOutcome::Clean => self.finalize_ff_merge(iter, row, branch, root).await,
            git::RebaseOutcome::Conflict { files } => {
                self.emit_for(
                    EventPayload::MergeConflict {
                        files: files.clone(),
                    },
                    Some(iter.id.clone()),
                    Some(row.id.clone()),
                );
                let resolved = self
                    .run_merge_resolver(project, iter, arch, row, worktree_path, branch, &files)
                    .await;
                // After the resolver returns we MUST verify the rebase is
                // actually done — the agent might have given up or only
                // partially resolved.
                if git::is_rebase_in_progress(worktree_path).await {
                    tracing::warn!(
                        task = %row.id,
                        "merge: rebase still in progress after resolver — aborting",
                    );
                    git::rebase_abort(worktree_path).await;
                    self.emit_for(
                        EventPayload::MergeResolved {
                            ok: false,
                            summary: resolved.unwrap_or_else(|| {
                                "Resolver не завершил rebase".into()
                            }),
                        },
                        Some(iter.id.clone()),
                        Some(row.id.clone()),
                    );
                    self.emit_for(
                        EventPayload::MergeFailed {
                            conflict: true,
                            message: "rebase aborted: resolver couldn't finish".into(),
                        },
                        Some(iter.id.clone()),
                        Some(row.id.clone()),
                    );
                    return false;
                }
                self.emit_for(
                    EventPayload::MergeResolved {
                        ok: true,
                        summary: resolved.unwrap_or_default(),
                    },
                    Some(iter.id.clone()),
                    Some(row.id.clone()),
                );
                self.finalize_ff_merge(iter, row, branch, root).await
            }
            git::RebaseOutcome::Error(err) => {
                tracing::error!(task = %row.id, %err, "merge: rebase failed catastrophically");
                self.emit_for(
                    EventPayload::MergeFailed {
                        conflict: false,
                        message: err,
                    },
                    Some(iter.id.clone()),
                    Some(row.id.clone()),
                );
                false
            }
        }
    }

    /// Fast-forward main to the specialist's (now-rebased) branch tip.
    async fn finalize_ff_merge(
        &self,
        iter: &IterationRow,
        row: &TaskRow,
        branch: &str,
        root: &std::path::Path,
    ) -> bool {
        let merge_started = std::time::Instant::now();
        let merge = git::ff_merge(root, branch).await;
        tracing::info!(
            task = %row.id,
            merge_ms = merge_started.elapsed().as_millis() as u64,
            ok = merge.ok,
            "merge: ff merge into main",
        );
        if !merge.ok {
            tracing::warn!(
                task = %row.id,
                message = %merge.message,
                "merge: ff_merge refused — work for this task is NOT in main",
            );
            self.emit_for(
                EventPayload::MergeFailed {
                    conflict: false,
                    message: merge.message,
                },
                Some(iter.id.clone()),
                Some(row.id.clone()),
            );
        }
        merge.ok
    }

    /// Invoke the Merge Resolver agent in the specialist's worktree where a
    /// rebase is paused on conflicts. The agent is responsible for editing
    /// the conflicted files, `git add`-ing them, and running
    /// `git rebase --continue`. Returns the agent's free-form report on
    /// success, or None if the agent crashed.
    #[allow(clippy::too_many_arguments)]
    async fn run_merge_resolver(
        &self,
        project: &ProjectRow,
        iter: &IterationRow,
        arch: &ArchTask,
        row: &TaskRow,
        worktree_path: &std::path::Path,
        branch: &str,
        conflict_files: &[String],
    ) -> Option<String> {
        tracing::info!(
            task = %row.id,
            files = ?conflict_files,
            "merge: invoking Merge Resolver agent",
        );
        let prompt = format!(
            "Ты в worktree специалиста, который выполнил задачу ниже. Сейчас идёт rebase его ветки `{branch}` на main, и есть конфликты — другие специалисты в этой же итерации параллельно правили те же файлы.\n\n--- ЗАДАЧА СПЕЦИАЛИСТА ---\nРоль: {role:?}\nЗаголовок: {title}\nОписание:\n{description}\n\n--- КОНФЛИКТНЫЕ ФАЙЛЫ ---\n{files}\n\nРазреши конфликты по алгоритму из системного промпта, потом `git rebase --continue`. Не торопись, не переписывай функциональность — склей две версии.",
            role = arch.role,
            title = arch.title,
            description = arch.description,
            files = conflict_files.join("\n"),
        );
        let inv = AgentInvocation {
            role: AgentRole::MergeResolver,
            system_prompt: system_prompt(AgentRole::MergeResolver, false, false).to_string(),
            user_prompt: prompt,
            cwd: worktree_path.to_path_buf(),
            model: project.model_specialist.clone(),
            tools: tools_for(AgentRole::MergeResolver),
            permission_mode: project.permission_mode,
            max_turns: 30,
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
        };
        let publisher = self.event_publisher();
        let iter_id = iter.id.clone();
        let task_id = row.id.clone();
        let result = run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, Some(iter_id.clone()), Some(task_id.clone()));
        })
        .await;
        match result {
            Ok(r) => Some(r.final_text.trim().to_string()),
            Err(e) => {
                tracing::warn!(task = %row.id, error = %e, "merge: resolver agent crashed");
                None
            }
        }
    }

    /// Two-stage ask_user handler. First the Blocker Reviewer reads the
    /// question and decides if it can auto-answer; if not, the question is
    /// routed to the user via the UI and we park until they answer.
    async fn handle_ask_user(
        &self,
        question: String,
        context: String,
        iter: IterationRow,
        task_id: String,
        agent_role: AgentRole,
    ) -> AppResult<String> {
        self.emit_for(
            EventPayload::AskUserInvoked {
                question: question.clone(),
                context: context.clone(),
            },
            Some(iter.id.clone()),
            Some(task_id.clone()),
        );

        let project = self.project_snapshot();
        let blocker_prompt = format!(
            "Идея проекта: {}\n\nЗадача специалиста ({:?}).\n\nСпециалист вызвал ASK_USER:\nВопрос: {}\nКонтекст: {}\n\nРеши и верни строго JSON по описанному формату.",
            project.idea, agent_role, question, context
        );
        let inv = AgentInvocation {
            role: AgentRole::BlockerReviewer,
            system_prompt: system_prompt(AgentRole::BlockerReviewer, false, false).to_string(),
            user_prompt: blocker_prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_pm.clone(),
            tools: vec![],
            permission_mode: PermissionMode::Default,
            max_turns: 3,
            claude_code_preset: false,
            cancel: Some(self.cancel_token()),
        };
        let publisher = self.event_publisher();
        let iter_id = iter.id.clone();
        let task_id_inner = task_id.clone();
        let verdict_text = match run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, Some(iter_id.clone()), Some(task_id_inner.clone()));
        })
        .await
        {
            Ok(r) => r.final_text,
            Err(_) => String::new(),
        };
        let verdict: BlockerVerdict = extract_json(&verdict_text).unwrap_or(BlockerVerdict {
            needs_user: true,
            auto_answer: None,
            user_question: None,
            user_context: None,
            reasoning: Some("reviewer_error".into()),
        });

        if !verdict.needs_user {
            let answer = verdict.auto_answer.clone().unwrap_or_default();
            let q = self.store.push_question(
                &project.id,
                Some(iter.id.clone()),
                Some(task_id.clone()),
                Some(agent_role),
                question.clone(),
                context.clone(),
            )?;
            let _ = self.store.resolve_question(
                &q.id,
                QuestionResolution::Reviewer,
                answer.clone(),
                true,
            );
            let preview: String = answer.chars().take(500).collect();
            self.emit_for(
                EventPayload::QuestionAnswered {
                    question_id: q.id.clone(),
                    resolution: QuestionResolution::Reviewer,
                    answer_preview: preview,
                    reasoning: verdict.reasoning,
                },
                Some(iter.id.clone()),
                Some(task_id.clone()),
            );
            return Ok(answer);
        }

        // Need human.
        let user_q = verdict.user_question.unwrap_or(question);
        let user_ctx = verdict.user_context.unwrap_or(context);
        let q = self.store.push_question(
            &project.id,
            Some(iter.id.clone()),
            Some(task_id.clone()),
            Some(agent_role),
            user_q.clone(),
            user_ctx.clone(),
        )?;
        self.emit_for(
            EventPayload::QuestionAsked {
                question_id: q.id.clone(),
                question: user_q,
                context: user_ctx,
                reasoning: verdict.reasoning,
            },
            Some(iter.id.clone()),
            Some(task_id.clone()),
        );
        let (tx, rx) = oneshot::channel();
        self.inner
            .questions
            .lock_or_poisoned()
            .insert(q.id.clone(), tx);
        let cancel = self.cancel_token();
        // Wait for the user (or for stop()) — no polling, single source of truth.
        tokio::select! {
            a = rx => Ok(a.unwrap_or_else(|_| super::CANCEL_ANSWER.into())),
            _ = cancel.cancelled() => {
                self.inner.questions.lock_or_poisoned().remove(&q.id);
                let _ = self.store.cancel_question(&q.id);
                Ok(super::CANCEL_ANSWER.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_in_plain_text() {
        let text = "I am stuck.\n\nASK_USER_BEGIN\n{\"question\":\"db?\",\"context\":\"need redis\"}\nASK_USER_END";
        let ask = parse_ask_user_marker(text).expect("parsed");
        assert_eq!(ask.question, "db?");
        assert_eq!(ask.context, "need redis");
    }

    #[test]
    fn marker_with_json_fences() {
        let text = "stuck\n\nASK_USER_BEGIN\n```json\n{\"question\":\"db?\"}\n```\nASK_USER_END";
        let ask = parse_ask_user_marker(text).expect("parsed");
        assert_eq!(ask.question, "db?");
        assert!(ask.context.is_empty());
    }

    #[test]
    fn marker_without_end_returns_none() {
        let text = "ASK_USER_BEGIN\n{\"question\":\"x\"}";
        assert!(parse_ask_user_marker(text).is_none());
    }

    #[test]
    fn invalid_json_inside_marker_returns_none() {
        let text = "ASK_USER_BEGIN\nnot-json\nASK_USER_END";
        assert!(parse_ask_user_marker(text).is_none());
    }

    #[test]
    fn no_marker_returns_none() {
        assert!(parse_ask_user_marker("just a normal report").is_none());
    }
}
