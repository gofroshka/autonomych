//! The Conductor — main state machine and iteration loop.
//!
//! One Conductor per project. Spawning is lazy: a project's Conductor is
//! created on first `start()` / `start_presentation_only()` and lives for the
//! rest of the app session. The conductor drives the loop:
//!
//!   PO → Architect → Specialist waves → Reviewer → optional preview prep
//!   → Presenting → user resume → next iteration.
//!
//! State is persisted via [`Store`] so a crash in the middle of work is
//! recoverable: `start()` notices a resumable iteration and re-enters from
//! the right stage.
//!
//! Sub-modules:
//! - [`state`]        — split-by-access-pattern Conductor state
//! - [`events`]       — event log + emit to renderer
//! - [`outputs`]      — JSON shapes returned by agents
//! - [`iteration`]    — PO + Architect + Reviewer stages
//! - [`wave`]         — DAG scheduler for specialist tasks
//! - [`task_runner`]  — single specialist task lifecycle + ask_user routing
//! - [`preview_flow`] — preview prep / shutdown / Path B presentation-only
//! - [`preview`]      — preview state holder & crash recovery

use crate::error::AppResult;
use crate::events::{EventBus, EventPayload};
use crate::store::Store;
use crate::types::*;
use crate::util::MutexExt;
use state::Inner;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

mod events;
mod iteration;
mod outputs;
mod preview_flow;
mod state;
mod task_runner;
mod wave;

pub mod preview;
use preview::PreviewState;

/// How long to back off between consecutive failed iterations, multiplied by
/// the consecutive-failure count.
const BACKOFF_STEP: Duration = Duration::from_millis(4000);

/// How many iterations may fail back-to-back before the conductor parks
/// itself in `Error`.
const MAX_CONSECUTIVE_FAILURES: usize = 3;

/// Sentinel returned in place of a real answer when the conductor is stopped
/// while a specialist is waiting on `ask_user`.
pub(super) const CANCEL_ANSWER: &str = "[cancelled — operator stopped the cycle]";

/// Public handle to a project's conductor. State is split by access pattern
/// (see [`state`]): the conductor itself only holds immutable references
/// plus the shared cancel token.
pub struct Conductor {
    pub project_id: String,
    store: Arc<Store>,
    bus: Arc<dyn EventBus>,
    inner: Inner,
    pub preview: Arc<Mutex<PreviewState>>,
    /// Root cancellation token for the current run. `stop()` cancels it,
    /// which propagates to every running agent through `AgentInvocation.cancel`
    /// and kills their `claude` subprocesses. Wrapped in std::sync::Mutex
    /// (not tokio's) for cheap sync access.
    cancel: std::sync::Mutex<CancellationToken>,
    /// Handle to the most recent background loop task. We abort it on `stop()`
    /// so dropping the Conductor doesn't leave orphan tasks running.
    loop_task: std::sync::Mutex<Option<JoinHandle<()>>>,
    /// Serializes `git merge` into the project's main worktree. Specialists
    /// run in parallel (each in its own worktree) but every merge mutates
    /// the shared root — without this lock concurrent merges race and git
    /// refuses with "your local changes would be overwritten by merge".
    pub(super) merge_lock: Mutex<()>,
}

impl Conductor {
    pub fn new(project: ProjectRow, store: Arc<Store>, bus: Arc<dyn EventBus>) -> Self {
        let project_id = project.id.clone();
        Self {
            project_id,
            store,
            bus,
            inner: Inner::new(project),
            preview: Arc::new(Mutex::new(PreviewState::default())),
            cancel: std::sync::Mutex::new(CancellationToken::new()),
            loop_task: std::sync::Mutex::new(None),
            merge_lock: Mutex::new(()),
        }
    }

    // ---- Public lifecycle ----

    /// Start the main iteration loop. Picks up a resumable iteration if one
    /// is sitting in the store from a previous crash.
    #[tracing::instrument(skip(self), fields(project_id = %self.project_id))]
    pub async fn start(self: Arc<Self>) -> AppResult<()> {
        let project = self.project_snapshot();
        crate::git::ensure_repo(
            &PathBuf::from(&project.root_path),
            &project.name,
            &project.idea,
        )
        .await?;

        let resumable = self.store.find_resumable_iteration(&project.id);
        self.reset_cancel();
        self.inner.wrap_up_requested.store(
            resumable
                .as_ref()
                .is_some_and(|i| matches!(i.mode, Some(IterationMode::Wrapup))),
            Ordering::Relaxed,
        );

        let initial_state = match resumable.as_ref() {
            Some(i) if matches!(i.mode, Some(IterationMode::Wrapup)) => {
                ConductorState::WrappingUp
            }
            _ => ConductorState::Running,
        };
        self.set_state(initial_state)?;

        self.spawn_loop(move |me| me.run_loop(resumable));
        Ok(())
    }

    /// Skip new feature work and go straight to preview prep. If there's a
    /// crashed iteration, replay it as wrap-up first.
    #[tracing::instrument(skip(self), fields(project_id = %self.project_id))]
    pub async fn start_presentation_only(self: Arc<Self>) -> AppResult<()> {
        let project = self.project_snapshot();
        crate::git::ensure_repo(
            &PathBuf::from(&project.root_path),
            &project.name,
            &project.idea,
        )
        .await?;
        self.reset_cancel();

        let resumable = self.store.find_resumable_iteration(&project.id);
        if let Some(it) = resumable {
            self.store.set_iteration_meta(
                &it.id,
                None,
                None,
                None,
                None,
                Some(IterationMode::Wrapup),
            )?;
            self.inner.wrap_up_requested.store(true, Ordering::Relaxed);
            self.set_state(ConductorState::WrappingUp)?;
            self.emit(EventPayload::ResumeForPreview {
                iteration: it.number,
            });
            self.spawn_loop(move |me| me.run_loop(Some(it)));
        } else {
            self.spawn_loop(|me| me.run_presentation_only());
        }
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(project_id = %self.project_id))]
    pub async fn stop(&self) -> AppResult<()> {
        // Cancel the root token → every running agent observes it on its
        // next `tokio::select!` and SIGKILL's its claude subprocess. Every
        // poll-on-stopped consumer also selects on this token so they wake
        // up immediately.
        self.cancel_all();
        // Drop the resume waker; anyone parked in `await_resume` selects on
        // the cancel token directly.
        if let Some(w) = self.inner.waker.lock_or_poisoned().take() {
            let _ = w.send(());
        }
        // Resolve every pending question with a cancellation sentinel so
        // specialists waiting on them unblock and exit cleanly.
        for (_id, sender) in self.inner.questions.lock_or_poisoned().drain() {
            let _ = sender.send(CANCEL_ANSWER.into());
        }
        Ok(())
    }

    pub async fn request_wrap_up(&self) {
        let state = self.current_state();
        if matches!(
            state,
            ConductorState::Presenting
                | ConductorState::WrappingUp
                | ConductorState::PreparingPreview
        ) {
            return;
        }
        self.inner.wrap_up_requested.store(true, Ordering::Relaxed);
        self.emit(EventPayload::WrapUpRequested);
        if matches!(state, ConductorState::Running) {
            let _ = self.set_state(ConductorState::WrappingUp);
        }
    }

    pub async fn resume(&self) {
        if let Some(w) = self.inner.waker.lock_or_poisoned().take() {
            let _ = w.send(());
        }
    }

    pub async fn answer_question(&self, question_id: &str, answer: String) {
        let _ = self.store.resolve_question(
            question_id,
            QuestionResolution::User,
            answer.clone(),
            false,
        );
        let preview: String = answer.chars().take(500).collect();
        self.emit(EventPayload::QuestionAnswered {
            question_id: question_id.into(),
            resolution: QuestionResolution::User,
            answer_preview: preview,
            reasoning: None,
        });
        let sender = self
            .inner
            .questions
            .lock_or_poisoned()
            .remove(question_id);
        if let Some(sender) = sender {
            let _ = sender.send(answer);
        }
    }

    // ---- Main loop ----

    pub(crate) async fn run_loop(
        self: Arc<Self>,
        mut resume_iter: Option<IterationRow>,
    ) -> AppResult<()> {
        let mut consecutive_failures = 0usize;
        loop {
            if self.is_cancelled() {
                self.set_state(ConductorState::Idle)?;
                return Ok(());
            }
            let state = self.current_state();
            if !matches!(state, ConductorState::WrappingUp) {
                self.set_state(ConductorState::Running)?;
            }

            let iter = if let Some(r) = resume_iter.take() {
                if matches!(r.mode, Some(IterationMode::Wrapup)) {
                    self.inner.wrap_up_requested.store(true, Ordering::Relaxed);
                }
                r
            } else {
                let it = self.store.create_iteration(&self.project_id)?;
                self.store.set_iteration_meta(
                    &it.id,
                    None,
                    None,
                    None,
                    None,
                    Some(IterationMode::Normal),
                )?;
                self.emit_for(
                    EventPayload::IterationStart {
                        number: it.number,
                        mode: IterationMode::Normal,
                    },
                    Some(it.id.clone()),
                    None,
                );
                it
            };

            let mut failed = false;
            if let Err(e) = self.clone().run_iteration(iter.clone()).await {
                failed = true;
                tracing::warn!("iteration {} failed: {e}", iter.number);
                self.emit_for(
                    EventPayload::IterationError {
                        error: e.to_string(),
                    },
                    Some(iter.id.clone()),
                    None,
                );
                self.store.set_iteration_status(
                    &iter.id,
                    IterationStatus::Failed,
                    Some(&format!("Error: {e}")),
                )?;
            }

            // Wrap-up requested during this iteration → go to preview, no new iter.
            let was_wrap = self.inner.wrap_up_requested.swap(false, Ordering::Relaxed);
            if was_wrap {
                self.store
                    .set_iteration_meta(
                        &iter.id,
                        None,
                        None,
                        None,
                        None,
                        Some(IterationMode::Wrapup),
                    )
                    .ok();
            }

            if failed {
                consecutive_failures += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    self.emit(EventPayload::TooManyFailures {
                        consecutive: consecutive_failures,
                    });
                    self.set_state(ConductorState::Error)?;
                    return Ok(());
                }
                let backoff = BACKOFF_STEP * consecutive_failures as u32;
                self.emit(EventPayload::Backoff {
                    duration_ms: backoff.as_millis() as u64,
                    consecutive: consecutive_failures,
                });
                tokio::time::sleep(backoff).await;
            } else {
                consecutive_failures = 0;
            }

            if was_wrap {
                self.set_state(ConductorState::PreparingPreview)?;
                if let Err(e) = self.run_preview_prep().await {
                    let mut p = self.preview.lock().await;
                    p.prep_error = Some(e.to_string());
                    self.emit(EventPayload::PreviewPrepFailed {
                        error: e.to_string(),
                    });
                }
                self.set_state(ConductorState::Presenting)?;
                self.await_resume().await;
                if self.is_cancelled() {
                    self.set_state(ConductorState::Idle)?;
                    return Ok(());
                }
                let _ = self.run_preview_shutdown().await;
                self.set_state(ConductorState::Resuming)?;
                self.emit(EventPayload::Resumed);
            }
        }
    }

    // ---- Small accessors / state helpers ----

    /// Cheap snapshot of the project. Returns an `Arc` so callers never need
    /// to clone the row itself.
    pub(super) fn project_snapshot(&self) -> Arc<ProjectRow> {
        self.inner.project_snapshot()
    }

    pub(super) fn current_state(&self) -> ConductorState {
        *self.inner.state.borrow()
    }

    /// `true` iff `stop()` has been called since the last `start()`. Cheap,
    /// non-async — uses the cancellation token as the single source of truth.
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel.lock_or_poisoned().is_cancelled()
    }

    pub(super) fn set_state(&self, s: ConductorState) -> AppResult<()> {
        self.inner.state.send_replace(s);
        self.store.set_project_state(&self.project_id, s)?;
        self.emit(EventPayload::StateChange { state: s });
        Ok(())
    }

    /// Park until the user presses Продолжаем or the conductor is stopped.
    /// Both unblock the wait — cancellation comes through the root token so
    /// no boolean flag needs to be polled.
    pub(super) async fn await_resume(&self) {
        if self.is_cancelled() {
            return;
        }
        let rx = {
            let (tx, rx) = oneshot::channel();
            *self.inner.waker.lock_or_poisoned() = Some(tx);
            rx
        };
        let cancel = self.cancel_token();
        tokio::select! {
            _ = rx => {}
            _ = cancel.cancelled() => {}
        }
    }

    // ---- Cancellation ----

    /// Cheap clone of the current cancel token. Hand to every spawned agent.
    pub(super) fn cancel_token(&self) -> CancellationToken {
        self.cancel.lock_or_poisoned().clone()
    }

    fn reset_cancel(&self) {
        *self.cancel.lock_or_poisoned() = CancellationToken::new();
    }

    fn cancel_all(&self) {
        self.cancel.lock_or_poisoned().cancel();
    }

    // ---- Structured concurrency ----

    /// Spawn the main loop, owning the handle so we can abort it cleanly.
    /// If a prior loop is still running for this conductor we abort it first.
    fn spawn_loop<F, Fut>(self: &Arc<Self>, body: F)
    where
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = AppResult<()>> + Send + 'static,
    {
        let me = self.clone();
        let handle = tokio::spawn(async move {
            let me_for_err = me.clone();
            if let Err(e) = body(me).await {
                tracing::error!(error = %e, "conductor loop failed");
                let _ = me_for_err.set_state(ConductorState::Error);
                me_for_err.emit(EventPayload::LoopError {
                    error: e.to_string(),
                });
            }
        });
        let mut slot = self.loop_task.lock_or_poisoned();
        if let Some(prev) = slot.replace(handle) {
            prev.abort();
        }
    }
}

impl Drop for Conductor {
    fn drop(&mut self) {
        // Best-effort: cancel the root token so any in-flight subprocesses
        // get killed, and abort the loop task to prevent leaks.
        self.cancel.lock_or_poisoned().cancel();
        if let Some(handle) = self.loop_task.lock_or_poisoned().take() {
            handle.abort();
        }
    }
}
