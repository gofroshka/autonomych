//! Preview prep + shutdown. Both stages just invoke the Presenter agent
//! and store whatever free-form text it returns. The agent owns the entire
//! dev-server lifecycle — we never touch PIDs, ports, or manifests.

use super::Conductor;
use crate::agents::{run_agent, system_prompt, tools_for, AgentInvocation};
use crate::error::AppResult;
use crate::events::EventPayload;
use crate::types::*;
use std::path::PathBuf;
use std::sync::Arc;

impl Conductor {
    #[tracing::instrument(skip(self), fields(project_id = %self.project_id))]
    pub(super) async fn run_preview_prep(&self) -> AppResult<()> {
        let project = self.project_snapshot();
        self.preview.lock().await.reset_prep();
        let iter = self.store.current_iteration(&project.id);

        let prompt = format!(
            "Идея проекта: {}\nКорень проекта: {} — ты в нём.\n\nЗапусти проект так, чтобы пользователь мог его потыкать в браузере / в нужном клиенте. В финальном сообщении расскажи пользователю человеческим языком, как и где смотреть результат.",
            project.idea, project.root_path
        );
        let inv = self.presenter_invocation(
            &project,
            prompt,
            system_prompt(AgentRole::Presenter, false, false).to_string(),
            tools_for(AgentRole::Presenter),
            40,
        );
        let publisher = self.event_publisher();
        let iter_id = iter.as_ref().map(|i| i.id.clone());
        let res = run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, iter_id.clone(), None);
        })
        .await?;

        let text = res.final_text.trim().to_string();
        {
            let mut p = self.preview.lock().await;
            if text.is_empty() {
                p.prep_error = Some("Presenter ничего не вернул".into());
                tracing::warn!("preview: agent returned empty final_text");
            } else {
                p.instructions = Some(text.clone());
                p.prep_error = None;
            }
            p.prepared_at = Some(chrono::Utc::now().timestamp_millis());
        }
        tracing::info!(chars = text.len(), "preview: prep done, instructions stored");
        self.emit(EventPayload::PreviewPrepDone);
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(project_id = %self.project_id))]
    pub(super) async fn run_preview_shutdown(&self) -> AppResult<()> {
        let project = self.project_snapshot();
        let prev = self.preview.lock().await.instructions.clone();
        let Some(prev) = prev else {
            tracing::info!("preview: no prior instructions, skipping shutdown");
            self.emit(EventPayload::PreviewShutdownSkipped {
                reason: "no_instructions".into(),
            });
            return Ok(());
        };

        let prompt = format!(
            "Корень проекта: {} — ты в нём.\n\nРаньше ты запустил демо и сказал пользователю:\n---\n{}\n---\n\nСейчас останови всё, что ты запустил (dev-сервер, фоновые процессы, контейнеры если поднимал). Используй любые удобные средства (kill, pkill, lsof, docker compose down, что угодно). В финальном сообщении кратко напиши пользователю что остановил.",
            project.root_path, prev
        );
        let inv = self.presenter_invocation(
            &project,
            prompt,
            system_prompt(AgentRole::Presenter, false, true).to_string(),
            vec!["Read".into(), "Bash".into()],
            10,
        );
        let publisher = self.event_publisher();
        let res = run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, None, None);
        })
        .await;

        let text = match res {
            Ok(r) => r.final_text.trim().to_string(),
            Err(e) => {
                tracing::warn!(error = %e, "preview: shutdown agent failed");
                format!("(shutdown agent failed: {e})")
            }
        };
        {
            let mut p = self.preview.lock().await;
            p.shutdown_message = if text.is_empty() { None } else { Some(text) };
            // Clear instructions so a subsequent prep starts clean and so a
            // second shutdown invocation is a no-op.
            p.instructions = None;
            p.prepared_at = None;
        }
        self.emit(EventPayload::PreviewShutdownDone);
        Ok(())
    }

    /// Path B: skip new feature work — go straight to preview on the current
    /// code, then fall into the normal loop after the user presses Продолжаем.
    pub(super) async fn run_presentation_only(self: Arc<Self>) -> AppResult<()> {
        self.set_state(ConductorState::PreparingPreview)?;
        self.emit(EventPayload::PresentationOnly);
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
        // After Path B's presentation, fall into the normal cycle.
        self.run_loop(None).await
    }

    /// Build the AgentInvocation shape used by both Presenter calls.
    fn presenter_invocation(
        &self,
        project: &ProjectRow,
        user_prompt: String,
        system_prompt: String,
        tools: Vec<String>,
        max_turns: u32,
    ) -> AgentInvocation {
        AgentInvocation {
            role: AgentRole::Presenter,
            system_prompt,
            user_prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_specialist.clone(),
            tools,
            permission_mode: project.permission_mode,
            max_turns,
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
        }
    }
}
