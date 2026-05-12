//! The Conductor — main state machine and iteration loop.
//!
//! One Conductor per project. Spawning is lazy: a project's Conductor is
//! created on first `start()` / `start_presentation_only()` and lives for
//! the rest of the app session.
//!
//! The loop drives: PO → Architect → Specialist waves → Reviewer → optional
//! preview prep & presenting. State is persisted via Store; on crash the
//! resetStaleStates pass lets `start()` pick up where it left off.

use crate::agents::{
    extract_json, run_agent, system_prompt, tools_for, AgentEvent, AgentInvocation,
};
use crate::error::{AppError, AppResult};
use crate::git;
use crate::store::Store;
use crate::types::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;

pub mod preview;
use preview::PreviewState;

/// Public handle to a project's conductor.
pub struct Conductor {
    pub project_id: String,
    inner: Arc<Mutex<Inner>>,
    pub preview: Arc<Mutex<PreviewState>>,
    /// Root cancellation token for the current run. `stop()` cancels it,
    /// which propagates to every running agent through `AgentInvocation.cancel`
    /// and kills their `claude` subprocesses. Held outside `Inner` so any
    /// caller can clone the current token without awaiting the mutex.
    /// Wrapped in std::sync::Mutex (not tokio's) for cheap sync access.
    cancel: std::sync::Mutex<CancellationToken>,
}

impl Conductor {
    /// Snapshot of the current cancel token. Cloning is cheap (internally
    /// Arc-counted). Use in every AgentInvocation so `stop()` kills agents.
    fn cancel_token(&self) -> CancellationToken {
        self.cancel.lock().unwrap().clone()
    }

    fn reset_cancel(&self) {
        *self.cancel.lock().unwrap() = CancellationToken::new();
    }

    fn cancel_all(&self) {
        self.cancel.lock().unwrap().cancel();
    }
}

struct Inner {
    project: ProjectRow,
    store: Arc<Store>,
    app: AppHandle,
    state: ConductorState,
    wrap_up_requested: bool,
    stopped: bool,
    resume_waker: Option<oneshot::Sender<()>>,
    question_resolvers: HashMap<String, oneshot::Sender<String>>,
}

#[derive(Debug, Deserialize)]
struct PoOutput {
    iteration_theme: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    stories: Vec<IterationStory>,
}

#[derive(Debug, Deserialize)]
struct ArchOutput {
    #[serde(default)]
    stack_notes: String,
    #[serde(default)]
    tasks: Vec<ArchTask>,
}

#[derive(Debug, Deserialize, Clone)]
struct ArchTask {
    id: String,
    role: AgentRole,
    title: String,
    description: String,
    #[serde(default)]
    depends_on: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReviewerOutput {
    demoable: Option<bool>,
    #[serde(default)]
    changelog: String,
    #[serde(default)]
    risks: String,
}

#[derive(Debug, Deserialize)]
struct AskUserBlock {
    question: String,
    #[serde(default)]
    context: String,
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

/// Find ASK_USER_BEGIN ... ASK_USER_END block in agent's final text. Returns
/// parsed block + the index where the marker starts (caller can strip it
/// from the summary).
fn parse_ask_user_marker(text: &str) -> Option<AskUserBlock> {
    let start = text.find("ASK_USER_BEGIN")?;
    let after = &text[start + "ASK_USER_BEGIN".len()..];
    let end = after.find("ASK_USER_END")?;
    let body = after[..end].trim();
    // body may be wrapped in ```json fences
    let body = body.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    serde_json::from_str(body).ok()
}

#[derive(Debug, Deserialize)]
struct PresenterLaunchOutput {
    #[serde(default)]
    ready: bool,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    setup_steps: Vec<String>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    errors: Vec<String>,
}

impl Conductor {
    pub fn new(project: ProjectRow, store: Arc<Store>, app: AppHandle) -> Self {
        let inner = Inner {
            project,
            store,
            app,
            state: ConductorState::Idle,
            wrap_up_requested: false,
            stopped: false,
            resume_waker: None,
            question_resolvers: HashMap::new(),
        };
        let project_id = inner.project.id.clone();
        Self {
            project_id,
            inner: Arc::new(Mutex::new(inner)),
            preview: Arc::new(Mutex::new(PreviewState::default())),
            cancel: std::sync::Mutex::new(CancellationToken::new()),
        }
    }

    /// Start the main iteration loop. Picks up a resumable iteration if one
    /// is sitting in the store from a previous crash.
    pub async fn start(self: Arc<Self>) -> AppResult<()> {
        let (project, store) = {
            let g = self.inner.lock().await;
            (g.project.clone(), g.store.clone())
        };
        git::ensure_repo(
            &PathBuf::from(&project.root_path),
            &project.name,
            &project.idea,
        )
        .await?;

        let resumable = store.find_resumable_iteration(&project.id);
        self.reset_cancel();
        {
            let mut g = self.inner.lock().await;
            g.stopped = false;
            g.wrap_up_requested = resumable
                .as_ref()
                .map(|i| matches!(i.mode, Some(IterationMode::Wrapup)))
                .unwrap_or(false);
        }
        let initial_state = match resumable.as_ref() {
            Some(i) if matches!(i.mode, Some(IterationMode::Wrapup)) => {
                ConductorState::WrappingUp
            }
            _ => ConductorState::Running,
        };
        self.set_state(initial_state).await?;

        let me = self.clone();
        tokio::spawn(async move {
            let me2 = me.clone();
            if let Err(e) = me.run_loop(resumable).await {
                tracing::error!("conductor loop failed: {e}");
                let _ = me2.set_state(ConductorState::Error).await;
                let _ = me2.log_system(json!({"loop_error": e.to_string()})).await;
            }
        });
        Ok(())
    }

    /// Skip new feature work and go straight to preview prep. If there's a
    /// crashed iteration, replay it as wrap-up first.
    pub async fn start_presentation_only(self: Arc<Self>) -> AppResult<()> {
        let (project, store) = {
            let g = self.inner.lock().await;
            (g.project.clone(), g.store.clone())
        };
        git::ensure_repo(
            &PathBuf::from(&project.root_path),
            &project.name,
            &project.idea,
        )
        .await?;
        self.reset_cancel();
        {
            let mut g = self.inner.lock().await;
            g.stopped = false;
        }
        let resumable = store.find_resumable_iteration(&project.id);
        if let Some(it) = resumable {
            store.set_iteration_meta(
                &it.id,
                None,
                None,
                None,
                None,
                Some(IterationMode::Wrapup),
            )?;
            {
                let mut g = self.inner.lock().await;
                g.wrap_up_requested = true;
            }
            self.set_state(ConductorState::WrappingUp).await?;
            self.log_directive(json!({"kind": "resume_for_preview", "iteration": it.number}))
                .await;
            let me = self.clone();
            tokio::spawn(async move {
                let me2 = me.clone();
                if let Err(e) = me.run_loop(Some(it)).await {
                    tracing::error!("presentation resume failed: {e}");
                    let _ = me2.set_state(ConductorState::Error).await;
                }
            });
        } else {
            let me = self.clone();
            tokio::spawn(async move {
                let me2 = me.clone();
                if let Err(e) = me.run_presentation_only().await {
                    tracing::error!("presentation-only failed: {e}");
                    let _ = me2.set_state(ConductorState::Error).await;
                }
            });
        }
        Ok(())
    }

    pub async fn stop(&self) -> AppResult<()> {
        // Cancel the root token → every running agent observes it on its
        // next `tokio::select!` and SIGKILL's its claude subprocess.
        self.cancel_all();
        let mut g = self.inner.lock().await;
        g.stopped = true;
        if let Some(w) = g.resume_waker.take() {
            let _ = w.send(());
        }
        // Resolve every pending question with a cancellation sentinel so
        // specialists waiting on them unblock and exit cleanly.
        let resolvers: Vec<_> = g.question_resolvers.drain().collect();
        for (_id, sender) in resolvers {
            let _ = sender.send("[cancelled — operator stopped the cycle]".into());
        }
        Ok(())
    }

    pub async fn request_wrap_up(&self) {
        let mut g = self.inner.lock().await;
        if matches!(
            g.state,
            ConductorState::Presenting
                | ConductorState::WrappingUp
                | ConductorState::PreparingPreview
        ) {
            return;
        }
        g.wrap_up_requested = true;
        let app = g.app.clone();
        let pid = g.project.id.clone();
        let was_running = matches!(g.state, ConductorState::Running);
        drop(g);
        let _ = self
            .insert_and_emit_event(
                EventType::Directive,
                json!({"kind": "wrap_up_requested"}),
                None,
                None,
                None,
            )
            .await;
        if was_running {
            // Surface intent immediately.
            let _ = self.set_state(ConductorState::WrappingUp).await;
        }
        let _ = (app, pid);
    }

    pub async fn resume(&self) {
        let mut g = self.inner.lock().await;
        if let Some(w) = g.resume_waker.take() {
            let _ = w.send(());
        }
    }

    pub async fn answer_question(&self, question_id: &str, answer: String) {
        let store = {
            let g = self.inner.lock().await;
            g.store.clone()
        };
        let _ = store.resolve_question(question_id, QuestionResolution::User, answer.clone(), false);
        let project_id = self.inner.lock().await.project.id.clone();
        let _ = self
            .insert_and_emit_event(
                EventType::QuestionAnswered,
                json!({"question_id": question_id, "resolution": "user", "answer": &answer.chars().take(500).collect::<String>()}),
                None,
                None,
                None,
            )
            .await;
        let _ = project_id;
        let mut g = self.inner.lock().await;
        if let Some(sender) = g.question_resolvers.remove(question_id) {
            let _ = sender.send(answer);
        }
    }

    /// Single specialist task lifecycle: create worktree, run agent, handle
    /// ASK_USER markers (escalate through Blocker Reviewer / user), commit,
    /// merge. Up to MAX_ASK_RETRIES re-runs with user answers in prompt.
    async fn run_specialist_task(
        self: Arc<Self>,
        project: ProjectRow,
        iter: IterationRow,
        arch: ArchTask,
        row: TaskRow,
    ) -> AppResult<bool> {
        const MAX_ASK_RETRIES: usize = 2;
        let branch = format!("autonomych/iter-{}/{}-{}", iter.number, arch.id, row.id);
        let worktree_path = PathBuf::from(&project.root_path)
            .join(".autonomych")
            .join("worktrees")
            .join(format!("{}-{}", iter.number, arch.id));
        let root = PathBuf::from(&project.root_path);

        let _ = git::remove_worktree(&root, &worktree_path).await;
        let _ = git::delete_branch(&root, &branch).await;
        if let Err(e) = git::create_worktree(&root, &branch, &worktree_path).await {
            let _ = self.store().await.set_task_status(&row.id, TaskStatus::Failed);
            self.log_event(
                EventType::System,
                json!({"error": format!("worktree failed: {e}")}),
                Some(iter.id.clone()),
                Some(row.id.clone()),
                None,
            )
            .await;
            return Ok(false);
        }

        let _ = self.store().await.set_task_status(&row.id, TaskStatus::InProgress);

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
                format!("{base_prompt}\n\n--- ОТВЕТЫ НА ТВОИ ASK_USER (учти их и продолжи) ---\n{answers}")
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
            let handle = self.clone_handle();
            let iter_id_s = iter.id.clone();
            let task_id_s = row.id.clone();
            let agent_res = run_agent(inv, move |ev| {
                let h = handle.clone();
                let i = iter_id_s.clone();
                let t = task_id_s.clone();
                tokio::spawn(h.forward_agent_event(ev, Some(i), Some(t)));
            })
            .await;
            match agent_res {
                Ok(r) => {
                    final_text = r.final_text;
                    if let Some(ask) = parse_ask_user_marker(&final_text) {
                        if attempt < MAX_ASK_RETRIES {
                            let answer = self
                                .handle_ask_user(
                                    ask.question.clone(),
                                    ask.context.clone(),
                                    iter.clone(),
                                    row.id.clone(),
                                    arch.role,
                                )
                                .await
                                .unwrap_or_else(|_| "[Blocker Reviewer / user response unavailable]".into());
                            accumulated_answers.push((ask.question, answer));
                            continue;
                        }
                    }
                    break;
                }
                Err(e) => {
                    agent_error = Some(e);
                    break;
                }
            }
        }
        let _ = final_text;

        let outcome = match agent_error {
            None => {
                let _ = git::commit_all(
                    &worktree_path,
                    &format!("iter-{} {:?}: {}", iter.number, arch.role, arch.title),
                )
                .await;
                let merge = git::merge_branch(&root, &branch).await;
                if !merge.ok {
                    self.log_event(
                        EventType::System,
                        json!({"error": format!("merge failed (conflict={}): {}", merge.conflict, merge.message)}),
                        Some(iter.id.clone()),
                        Some(row.id.clone()),
                        None,
                    )
                    .await;
                }
                let _ = self.store().await.set_task_status(&row.id, TaskStatus::Done);
                Ok(true)
            }
            Some(e) => {
                let stopped = self.inner.lock().await.stopped;
                let is_abort = stopped || e.to_string().to_lowercase().contains("abort");
                if !is_abort {
                    let _ = self.store().await.set_task_status(&row.id, TaskStatus::Failed);
                    self.log_event(
                        EventType::AgentError,
                        json!({"error": e.to_string()}),
                        Some(iter.id.clone()),
                        Some(row.id.clone()),
                        Some(arch.role),
                    )
                    .await;
                }
                Ok(false)
            }
        };

        if !self.inner.lock().await.stopped {
            let _ = git::remove_worktree(&root, &worktree_path).await;
            let _ = git::delete_branch(&root, &branch).await;
        }
        outcome
    }

    /// Two-stage ask_user handler. Returns the answer that should be fed back
    /// to the specialist on its retry. Either auto-answered by Blocker
    /// Reviewer or waited from the human user via the UI.
    async fn handle_ask_user(
        &self,
        question: String,
        context: String,
        iter: IterationRow,
        task_id: String,
        agent_role: AgentRole,
    ) -> AppResult<String> {
        self.log_event(
            EventType::System,
            json!({"stage": "ask_user_invoked", "question": question, "context": context}),
            Some(iter.id.clone()),
            Some(task_id.clone()),
            Some(agent_role),
        )
        .await;

        let project = self.inner.lock().await.project.clone();
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
        let me = self.clone_handle();
        let iter_id = iter.id.clone();
        let task_id_inner = task_id.clone();
        let verdict_text = match run_agent(inv, move |ev| {
            let h = me.clone();
            let i = iter_id.clone();
            let t = task_id_inner.clone();
            tokio::spawn(h.forward_agent_event(ev, Some(i), Some(t)));
        })
        .await
        {
            Ok(r) => r.final_text,
            Err(_) => {
                // Reviewer failed → safer default: escalate to user.
                String::new()
            }
        };
        let verdict: BlockerVerdict = extract_json(&verdict_text).unwrap_or(BlockerVerdict {
            needs_user: true,
            auto_answer: None,
            user_question: None,
            user_context: None,
            reasoning: Some("reviewer_error".into()),
        });

        let store = self.store().await;
        if !verdict.needs_user {
            let answer = verdict.auto_answer.clone().unwrap_or_default();
            let q = store.push_question(
                &project.id,
                Some(iter.id.clone()),
                Some(task_id.clone()),
                Some(agent_role),
                question.clone(),
                context.clone(),
            )?;
            let _ = store.resolve_question(&q.id, QuestionResolution::Reviewer, answer.clone(), true);
            self.log_event(
                EventType::QuestionAnswered,
                json!({"question_id": q.id, "resolution": "reviewer", "reasoning": verdict.reasoning, "answer": answer.chars().take(500).collect::<String>()}),
                Some(iter.id.clone()),
                Some(task_id.clone()),
                Some(agent_role),
            )
            .await;
            return Ok(answer);
        }

        // Need human.
        let user_q = verdict.user_question.unwrap_or(question);
        let user_ctx = verdict.user_context.unwrap_or(context);
        let q = store.push_question(
            &project.id,
            Some(iter.id.clone()),
            Some(task_id.clone()),
            Some(agent_role),
            user_q.clone(),
            user_ctx.clone(),
        )?;
        self.log_event(
            EventType::QuestionAsked,
            json!({"question_id": q.id, "question": user_q, "context": user_ctx, "reasoning": verdict.reasoning}),
            Some(iter.id.clone()),
            Some(task_id.clone()),
            Some(agent_role),
        )
        .await;
        let (tx, mut rx) = oneshot::channel();
        {
            let mut g = self.inner.lock().await;
            g.question_resolvers.insert(q.id.clone(), tx);
        }
        // Wait for the user (or cancel on stop). Poll stopped flag periodically.
        loop {
            tokio::select! {
                a = &mut rx => {
                    return Ok(a.unwrap_or_else(|_| "[cancelled — operator stopped the cycle]".into()));
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                    if self.is_stopped().await {
                        let mut g = self.inner.lock().await;
                        g.question_resolvers.remove(&q.id);
                        let _ = store.cancel_question(&q.id);
                        return Ok("[cancelled — operator stopped the cycle]".into());
                    }
                }
            }
        }
    }

    pub async fn current_state(&self) -> ConductorState {
        self.inner.lock().await.state
    }

    // ---- Main loop ----
    async fn run_loop(self: Arc<Self>, mut resume_iter: Option<IterationRow>) -> AppResult<()> {
        let mut consecutive_failures = 0;
        loop {
            if self.is_stopped().await {
                self.set_state(ConductorState::Idle).await?;
                return Ok(());
            }
            let state = self.current_state().await;
            if !matches!(state, ConductorState::WrappingUp) {
                self.set_state(ConductorState::Running).await?;
            }

            let iter = if let Some(r) = resume_iter.take() {
                if matches!(r.mode, Some(IterationMode::Wrapup)) {
                    self.inner.lock().await.wrap_up_requested = true;
                }
                r
            } else {
                let store = self.store().await;
                let it = store.create_iteration(&self.project_id_ref().await)?;
                store.set_iteration_meta(&it.id, None, None, None, None, Some(IterationMode::Normal))?;
                self.log_event(
                    EventType::IterationStart,
                    json!({"number": it.number, "mode": "normal"}),
                    Some(it.id.clone()),
                    None,
                    None,
                )
                .await;
                it
            };

            let mut failed = false;
            if let Err(e) = self.clone().run_iteration(iter.clone()).await {
                failed = true;
                tracing::warn!("iteration {} failed: {e}", iter.number);
                self.log_event(
                    EventType::System,
                    json!({"error": e.to_string()}),
                    Some(iter.id.clone()),
                    None,
                    None,
                )
                .await;
                let store = self.store().await;
                store.set_iteration_status(
                    &iter.id,
                    IterationStatus::Failed,
                    Some(&format!("Error: {e}")),
                )?;
            }

            // Wrap-up requested during this iteration → go to preview, no new iter.
            let was_wrap = {
                let mut g = self.inner.lock().await;
                let w = g.wrap_up_requested;
                g.wrap_up_requested = false;
                if w {
                    g.store
                        .set_iteration_meta(&iter.id, None, None, None, None, Some(IterationMode::Wrapup))
                        .ok();
                }
                w
            };

            if failed {
                consecutive_failures += 1;
                if consecutive_failures >= 3 {
                    self.log_system(json!({"error": "3 итерации подряд упали — останавливаюсь."}))
                        .await;
                    self.set_state(ConductorState::Error).await?;
                    return Ok(());
                }
                let backoff_ms = 4000 * consecutive_failures;
                self.log_system(json!({"backoff_ms": backoff_ms, "failures": consecutive_failures}))
                    .await;
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            } else {
                consecutive_failures = 0;
            }

            if was_wrap {
                self.set_state(ConductorState::PreparingPreview).await?;
                if let Err(e) = self.run_preview_prep().await {
                    let mut p = self.preview.lock().await;
                    p.prep_error = Some(e.to_string());
                    self.log_system(json!({"stage": "preview_prep_failed", "error": e.to_string()}))
                        .await;
                }
                self.set_state(ConductorState::Presenting).await?;
                self.await_resume().await;
                if self.is_stopped().await {
                    self.set_state(ConductorState::Idle).await?;
                    return Ok(());
                }
                let _ = self.run_preview_shutdown().await;
                self.preview.lock().await.fallback_kill().await;
                self.set_state(ConductorState::Resuming).await?;
                self.log_directive(json!({"kind": "resume"})).await;
            }
        }
    }

    /// Path B: no iteration, just preview.
    async fn run_presentation_only(self: Arc<Self>) -> AppResult<()> {
        self.set_state(ConductorState::PreparingPreview).await?;
        self.log_directive(json!({"kind": "presentation_only"})).await;
        if let Err(e) = self.run_preview_prep().await {
            let mut p = self.preview.lock().await;
            p.prep_error = Some(e.to_string());
            self.log_system(json!({"stage": "preview_prep_failed", "error": e.to_string()}))
                .await;
        }
        self.set_state(ConductorState::Presenting).await?;
        self.await_resume().await;
        if self.is_stopped().await {
            self.set_state(ConductorState::Idle).await?;
            return Ok(());
        }
        let _ = self.run_preview_shutdown().await;
        self.preview.lock().await.fallback_kill().await;
        self.set_state(ConductorState::Resuming).await?;
        self.log_directive(json!({"kind": "resume"})).await;
        // After Path B's presentation, fall into the normal cycle.
        self.run_loop(None).await
    }

    // ---- Single iteration with resume awareness ----
    async fn run_iteration(self: Arc<Self>, iter: IterationRow) -> AppResult<()> {
        let (project, store) = {
            let g = self.inner.lock().await;
            (g.project.clone(), g.store.clone())
        };
        let mode = iter.mode.unwrap_or(IterationMode::Normal);
        let mode_is_wrapup = matches!(mode, IterationMode::Wrapup);

        let existing_tasks = store.iteration_tasks(&iter.id);
        let is_resume = !existing_tasks.is_empty() || iter.theme.is_some();
        if is_resume {
            self.log_event(
                EventType::System,
                json!({
                    "stage": "resume_iteration",
                    "number": iter.number,
                    "po_done": iter.theme.is_some(),
                    "arch_done": !existing_tasks.is_empty(),
                    "tasks_pending": existing_tasks.iter().filter(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)).count(),
                    "summary_done": iter.summary.is_some(),
                }),
                Some(iter.id.clone()),
                None,
                None,
            )
            .await;
        } else if let Some(s) = store.pending_steering(&project.id) {
            let _ = store.consume_steering(&s.id, &iter.id);
        }

        let project_context = self.snapshot_project_files(&PathBuf::from(&project.root_path)).await;

        // ---- 1. Product Owner ----
        let po_output: PoOutput = if let (Some(theme), false) =
            (iter.theme.clone(), iter.stories.is_empty())
        {
            self.log_event(
                EventType::System,
                json!({"stage": "po_skipped_resume", "theme": theme}),
                Some(iter.id.clone()),
                None,
                Some(AgentRole::ProductOwner),
            )
            .await;
            PoOutput {
                iteration_theme: theme,
                rationale: iter.rationale.clone().unwrap_or_default(),
                stories: iter.stories.clone(),
            }
        } else {
            let steering = store.pending_steering(&project.id);
            let history = self.build_history_summary(&store, &project.id);
            let po_prompt = format!(
                "Идея проекта: {}\nИмя проекта: {}\n\nТекущая итерация: #{} (mode={})\n\n--- История последних итераций ---\n{}\n\n--- Снапшот файлов проекта ---\n{}\n\n{}\nВерни строго JSON по описанному формату.",
                project.idea,
                project.name,
                iter.number,
                if mode_is_wrapup { "wrapup" } else { "normal" },
                if history.is_empty() { "(нет — это первая итерация)".into() } else { history },
                if project_context.is_empty() { "(пусто, проект ещё не создан)".into() } else { project_context.clone() },
                steering
                    .as_ref()
                    .map(|s| format!("--- USER_STEERING ({:?}) ---\n{}\n", s.mode, s.message))
                    .unwrap_or_default()
            );
            let raw = self
                .run_json_agent(AgentRole::ProductOwner, mode_is_wrapup, po_prompt, &project, &iter)
                .await?;
            let parsed: PoOutput = extract_json(&raw).unwrap_or(PoOutput {
                iteration_theme: "(без темы)".into(),
                rationale: String::new(),
                stories: vec![],
            });
            store.set_iteration_meta(
                &iter.id,
                Some(parsed.iteration_theme.clone()),
                Some(parsed.rationale.clone()),
                Some(parsed.stories.clone()),
                None,
                None,
            )?;
            self.log_event(
                EventType::System,
                json!({"stage": "po_done", "theme": parsed.iteration_theme, "stories": parsed.stories.len()}),
                Some(iter.id.clone()),
                None,
                Some(AgentRole::ProductOwner),
            )
            .await;
            if parsed.stories.is_empty() {
                return Err(AppError::Conductor("PO не вернул ни одной story".into()));
            }
            parsed
        };

        // ---- 2. Architect ----
        let arch_output: ArchOutput = {
            let reloaded = store.iteration_tasks(&iter.id);
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
                self.log_event(
                    EventType::System,
                    json!({"stage": "arch_skipped_resume", "tasks": tasks.len()}),
                    Some(iter.id.clone()),
                    None,
                    Some(AgentRole::Architect),
                )
                .await;
                ArchOutput {
                    stack_notes: iter.stack_notes.clone().unwrap_or_default(),
                    tasks,
                }
            } else {
                let arch_prompt = format!(
                    "Тема итерации: {}\nОбоснование: {}\n\n--- User stories ---\n{}\n\n--- Снапшот проекта ---\n{}\n\nВерни строго JSON.",
                    po_output.iteration_theme,
                    po_output.rationale,
                    serde_json::to_string_pretty(&po_output.stories).unwrap_or_default(),
                    if project_context.is_empty() { "(проект пустой)".into() } else { project_context.clone() }
                );
                let raw = self
                    .run_json_agent(AgentRole::Architect, mode_is_wrapup, arch_prompt, &project, &iter)
                    .await?;
                let parsed: ArchOutput = extract_json(&raw)
                    .unwrap_or(ArchOutput { stack_notes: String::new(), tasks: vec![] });
                store.set_iteration_meta(
                    &iter.id,
                    None,
                    None,
                    None,
                    Some(parsed.stack_notes.clone()),
                    None,
                )?;
                self.log_event(
                    EventType::System,
                    json!({"stage": "arch_done", "tasks": parsed.tasks.len(), "stack": parsed.stack_notes}),
                    Some(iter.id.clone()),
                    None,
                    Some(AgentRole::Architect),
                )
                .await;
                if parsed.tasks.is_empty() {
                    return Err(AppError::Conductor("Architect не вернул задач".into()));
                }
                let _ = git::tag(
                    &PathBuf::from(&project.root_path),
                    &format!("autonomych/pre-iter-{}", iter.number),
                )
                .await;
                for t in &parsed.tasks {
                    store.create_task(
                        &iter.id,
                        t.role,
                        t.title.clone(),
                        t.description.clone(),
                        Some(t.id.clone()),
                        t.depends_on.clone(),
                    )?;
                }
                parsed
            }
        };

        // ---- 3. Specialists (wave runner) ----
        self.clone().execute_specialist_waves(&arch_output.tasks, &iter, &project).await;

        // ---- 4. Reviewer ----
        let reviewer_prompt = {
            let task_rows = store.iteration_tasks(&iter.id);
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
            format!(
                "Итерация #{}. Тема: {}\nStories:\n{}\n\nВыполненные задачи:\n{}\n\nКорень проекта: {}. Ты можешь читать файлы и запускать команды.\nСделай проверку и верни строго JSON.",
                iter.number, po_output.iteration_theme, stories_list, tasks_list, project.root_path
            )
        };
        let reviewer = match self
            .run_json_agent(AgentRole::Reviewer, mode_is_wrapup, reviewer_prompt, &project, &iter)
            .await
        {
            Ok(text) => extract_json::<ReviewerOutput>(&text).ok(),
            Err(e) => {
                self.log_event(
                    EventType::AgentError,
                    json!({"error": format!("reviewer failed: {e}")}),
                    Some(iter.id.clone()),
                    None,
                    Some(AgentRole::Reviewer),
                )
                .await;
                None
            }
        };

        let summary = match &reviewer {
            Some(r) => format!(
                "{} {}\n\n{}\n\nRisks: {}",
                if r.demoable.unwrap_or(false) { "✓" } else { "✗" },
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
        store.set_iteration_status(&iter.id, final_status, Some(&summary))?;
        self.log_event(
            EventType::IterationEnd,
            json!({"mode": format!("{:?}", mode), "demoable": reviewer.as_ref().and_then(|r| r.demoable), "summary": summary}),
            Some(iter.id.clone()),
            None,
            None,
        )
        .await;
        Ok(())
    }

    // ---- Wave-based specialist execution ----
    async fn execute_specialist_waves(
        self: Arc<Self>,
        tasks: &[ArchTask],
        iter: &IterationRow,
        project: &ProjectRow,
    ) {
        const MAX_CONCURRENCY: usize = 3;
        let store = self.store().await;
        let by_id: HashMap<String, ArchTask> =
            tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();

        // Build initial task_row map (architect_id → TaskRow), seeded from store.
        let mut task_rows: HashMap<String, TaskRow> = HashMap::new();
        for r in store.iteration_tasks(&iter.id) {
            if let Some(aid) = &r.architect_id {
                task_rows.insert(aid.clone(), r);
            }
        }

        let mut completed = HashSet::<String>::new();
        let mut failed = HashSet::<String>::new();
        let mut skipped = HashSet::<String>::new();
        let mut remaining = HashSet::<String>::new();
        for t in tasks {
            if let Some(row) = task_rows.get(&t.id) {
                match row.status {
                    TaskStatus::Done => { completed.insert(t.id.clone()); }
                    TaskStatus::Failed => { failed.insert(t.id.clone()); }
                    TaskStatus::Skipped => { skipped.insert(t.id.clone()); }
                    _ => { remaining.insert(t.id.clone()); }
                }
            } else {
                remaining.insert(t.id.clone());
            }
        }

        while !remaining.is_empty() && !self.is_stopped().await {
            // Find ready set + cascade-skip dependents of failed/skipped deps.
            let mut ready = Vec::<ArchTask>::new();
            let mut skip_now = Vec::<String>::new();
            for id in &remaining {
                let t = by_id.get(id).unwrap();
                let deps = &t.depends_on;
                if deps.iter().any(|d| failed.contains(d) || skipped.contains(d)) {
                    skip_now.push(id.clone());
                    continue;
                }
                if deps.iter().all(|d| completed.contains(d) || !by_id.contains_key(d)) {
                    ready.push(t.clone());
                }
            }
            for id in &skip_now {
                remaining.remove(id);
                skipped.insert(id.clone());
                if let Some(row) = task_rows.get(id) {
                    let _ = store.set_task_status(&row.id, TaskStatus::Skipped);
                }
            }
            if !skip_now.is_empty() {
                self.log_event(
                    EventType::System,
                    json!({"skipped": skip_now.len(), "reason": "dependency_failed"}),
                    Some(iter.id.clone()),
                    None,
                    None,
                )
                .await;
            }
            if ready.is_empty() {
                if !remaining.is_empty() {
                    for id in remaining.iter() {
                        if let Some(row) = task_rows.get(id) {
                            let _ = store.set_task_status(&row.id, TaskStatus::Skipped);
                        }
                    }
                    self.log_system(json!({"error": "task graph deadlock — orphan dependencies"}))
                        .await;
                }
                break;
            }

            ready.truncate(MAX_CONCURRENCY);
            self.log_event(
                EventType::System,
                json!({"wave_size": ready.len()}),
                Some(iter.id.clone()),
                None,
                None,
            )
            .await;

            // Run wave in parallel, serialize merges.
            let mut handles = Vec::new();
            for t in ready.iter().cloned() {
                let row = task_rows.get(&t.id).cloned().unwrap();
                let project = project.clone();
                let iter = iter.clone();
                let me = self.clone();
                let arch_id = t.id.clone();
                handles.push(tokio::spawn(async move {
                    let r = me.run_specialist_task(project, iter, t, row.clone()).await;
                    (arch_id, r)
                }));
            }
            for h in handles {
                let (arch_id, res) = h.await.unwrap_or((String::new(), Ok(false)));
                remaining.remove(&arch_id);
                match res {
                    Ok(true) => { completed.insert(arch_id); }
                    _ => { failed.insert(arch_id); }
                }
            }

            if self.inner.lock().await.wrap_up_requested {
                // user pressed wrap-up — but per design, finish remaining
                // waves of THIS iteration too; the wrap-up only stops new
                // iterations starting after.
            }
        }
    }

    // ---- Preview prep + shutdown via Presenter agent ----
    async fn run_preview_prep(&self) -> AppResult<()> {
        let project = self.inner.lock().await.project.clone();
        {
            let mut p = self.preview.lock().await;
            p.reset_prep();
            p.refresh_manifest(&PathBuf::from(&project.root_path)).await;
        }
        let iter = self.store().await.current_iteration(&project.id);

        let prompt = format!(
            "Идея проекта: {}\nКорень проекта: {} — ты уже находишься здесь.\n\nПодготовь и запусти dev-сервер по алгоритму. Запиши манифест .autonomych/preview.json через Write. В финальном сообщении верни строго JSON.",
            project.idea, project.root_path
        );
        let inv = AgentInvocation {
            role: AgentRole::Presenter,
            system_prompt: system_prompt(AgentRole::Presenter, false, false).to_string(),
            user_prompt: prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_specialist.clone(),
            tools: tools_for(AgentRole::Presenter),
            permission_mode: project.permission_mode,
            max_turns: 40,
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
        };
        let me = self.clone_handle();
        let iter_id = iter.as_ref().map(|i| i.id.clone());
        let res = run_agent(inv, move |ev| {
            let h = me.clone();
            let i = iter_id.clone();
            tokio::spawn(h.forward_agent_event(ev, i, None));
        })
        .await?;

        {
            let mut p = self.preview.lock().await;
            p.refresh_manifest(&PathBuf::from(&project.root_path)).await;
            if let Ok(parsed) = extract_json::<PresenterLaunchOutput>(&res.final_text) {
                p.setup_steps = parsed.setup_steps;
                p.notes = parsed.notes;
                p.errors = parsed.errors.clone();
                p.prepared_at = Some(chrono::Utc::now().timestamp_millis());
                if !parsed.ready && !parsed.errors.is_empty() {
                    p.prep_error = Some(parsed.errors.join("; "));
                }
                let _ = parsed.command;
                let _ = parsed.url;
                let _ = parsed.pid;
            } else {
                p.prep_error = Some("Presenter returned non-JSON output".into());
            }
        }
        self.log_system(json!({"stage": "preview_prep_done"})).await;
        Ok(())
    }

    async fn run_preview_shutdown(&self) -> AppResult<()> {
        let project = self.inner.lock().await.project.clone();
        let manifest = {
            let mut p = self.preview.lock().await;
            p.refresh_manifest(&PathBuf::from(&project.root_path)).await;
            p.manifest.clone()
        };
        let Some(m) = manifest else {
            self.log_system(json!({"stage": "preview_shutdown_skipped", "reason": "no_manifest"}))
                .await;
            return Ok(());
        };
        let prompt = format!(
            "Останови демо по манифесту:\n{}\n\nКорень проекта: {} — ты в нём.\nПосле — удали .autonomych/preview.json и .autonomych/preview.pid.\nВерни строго JSON с полями ok, steps, errors.",
            serde_json::to_string_pretty(&m).unwrap_or_default(),
            project.root_path
        );
        let inv = AgentInvocation {
            role: AgentRole::Presenter,
            system_prompt: system_prompt(AgentRole::Presenter, false, true).to_string(),
            user_prompt: prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_specialist.clone(),
            tools: vec!["Read".into(), "Bash".into()],
            permission_mode: project.permission_mode,
            max_turns: 10,
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
        };
        let me = self.clone_handle();
        let _ = run_agent(inv, move |ev| {
            let h = me.clone();
            tokio::spawn(h.forward_agent_event(ev, None, None));
        })
        .await;
        self.log_system(json!({"stage": "preview_shutdown_done"})).await;
        Ok(())
    }

    // ---- Helpers ----
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
            max_turns: if matches!(role, AgentRole::Reviewer) { 15 } else { 5 },
            claude_code_preset: matches!(role, AgentRole::Reviewer),
            cancel: Some(self.cancel_token()),
        };
        let me = self.clone_handle();
        let iter_id = iter.id.clone();
        let res = run_agent(inv, move |ev| {
            let h = me.clone();
            let i = iter_id.clone();
            tokio::spawn(h.forward_agent_event(ev, Some(i), None));
        })
        .await?;
        Ok(res.final_text)
    }

    fn clone_handle(&self) -> ConductorHandle {
        ConductorHandle {
            inner: self.inner.clone(),
        }
    }

    async fn store(&self) -> Arc<Store> {
        self.inner.lock().await.store.clone()
    }
    async fn project_id_ref(&self) -> String {
        self.inner.lock().await.project.id.clone()
    }

    async fn snapshot_project_files(&self, root: &PathBuf) -> String {
        // Cheap minimal snapshot — README + package.json + a couple of source
        // files. Mirrors TS behavior but kept short to avoid token bloat.
        use tokio::fs;
        let mut chunks = Vec::new();
        for cand in &["README.md", "package.json", "tsconfig.json", "pyproject.toml", "docker-compose.yml"] {
            let p = root.join(cand);
            if let Ok(content) = fs::read_to_string(&p).await {
                let snippet: String = content.chars().take(800).collect();
                chunks.push(format!("### {cand}\n```\n{snippet}\n```"));
            }
        }
        chunks.join("\n\n")
    }

    fn build_history_summary(&self, store: &Store, project_id: &str) -> String {
        store
            .recent_iterations(project_id, 5)
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

    async fn set_state(&self, s: ConductorState) -> AppResult<()> {
        let (project_id, app, store) = {
            let mut g = self.inner.lock().await;
            g.state = s;
            (g.project.id.clone(), g.app.clone(), g.store.clone())
        };
        store.set_project_state(&project_id, s)?;
        let _ = self
            .insert_and_emit_event(EventType::StateChange, json!({"state": s}), None, None, None)
            .await;
        let _ = app;
        Ok(())
    }

    async fn is_stopped(&self) -> bool {
        self.inner.lock().await.stopped
    }

    async fn await_resume(&self) {
        let rx = {
            let mut g = self.inner.lock().await;
            if g.stopped { return; }
            let (tx, rx) = oneshot::channel();
            g.resume_waker = Some(tx);
            rx
        };
        let _ = rx.await;
    }

    async fn log_event(
        &self,
        r#type: EventType,
        payload: serde_json::Value,
        iteration_id: Option<String>,
        task_id: Option<String>,
        agent_role: Option<AgentRole>,
    ) {
        let _ = self
            .insert_and_emit_event(r#type, payload, iteration_id, task_id, agent_role)
            .await;
    }
    async fn log_system(&self, payload: serde_json::Value) {
        self.log_event(EventType::System, payload, None, None, None).await
    }
    async fn log_directive(&self, payload: serde_json::Value) {
        self.log_event(EventType::Directive, payload, None, None, None).await
    }

    async fn insert_and_emit_event(
        &self,
        r#type: EventType,
        payload: serde_json::Value,
        iteration_id: Option<String>,
        task_id: Option<String>,
        agent_role: Option<AgentRole>,
    ) -> AppResult<()> {
        let (project_id, app, store) = {
            let g = self.inner.lock().await;
            (g.project.id.clone(), g.app.clone(), g.store.clone())
        };
        let row = store.insert_event(&project_id, r#type, payload, iteration_id, task_id, agent_role)?;
        let _ = app.emit("event", &row);
        Ok(())
    }
}

/// Lightweight clone of the conductor identity used to forward agent events
/// from async task closures without holding the Conductor itself.
#[derive(Clone)]
struct ConductorHandle {
    inner: Arc<Mutex<Inner>>,
}

impl ConductorHandle {
    async fn forward_agent_event(
        self,
        ev: AgentEvent,
        iteration_id: Option<String>,
        task_id: Option<String>,
    ) {
        let (project_id, app, store) = {
            let g = self.inner.lock().await;
            (g.project.id.clone(), g.app.clone(), g.store.clone())
        };
        let (etype, payload, role) = match &ev {
            AgentEvent::Start { role } => (EventType::AgentStart, json!({}), Some(*role)),
            AgentEvent::AssistantText { role, text } => (
                EventType::AgentMessage,
                json!({"text": text.chars().take(2000).collect::<String>()}),
                Some(*role),
            ),
            AgentEvent::ToolUse { role, tool, input } => (
                EventType::AgentToolUse,
                json!({"tool": tool, "input": input}),
                Some(*role),
            ),
            AgentEvent::ToolResult { role, content, is_error } => (
                EventType::AgentToolResult,
                json!({"content": content, "is_error": is_error}),
                Some(*role),
            ),
            AgentEvent::End { role, turns, duration_ms, .. } => (
                EventType::AgentEnd,
                json!({"turns": turns, "durationMs": duration_ms}),
                Some(*role),
            ),
            AgentEvent::AgentError { role, message } => (
                EventType::AgentError,
                json!({"message": message}),
                Some(*role),
            ),
        };
        if let Ok(row) = store.insert_event(&project_id, etype, payload, iteration_id, task_id, role)
        {
            let _ = app.emit("event", &row);
        }
    }
}

// (run_specialist_task moved to Conductor::run_specialist_task)
