//! Preview prep + shutdown. Both stages just invoke the Presenter agent
//! and store whatever free-form text it returns. The agent owns the entire
//! dev-server lifecycle — we never touch PIDs, ports, or manifests.

use super::Conductor;
use crate::agents::{presenter_chat_prompt, run_agent, system_prompt, tools_for, AgentInvocation};
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
            self.finalize_pause_or_idle()?;
            return Ok(());
        }
        let _ = self.run_preview_shutdown().await;
        self.set_state(ConductorState::Resuming)?;
        self.emit(EventPayload::Resumed);
        // After Path B's presentation, fall into the normal cycle.
        self.run_loop(None).await
    }

    /// Mid-demo chat with the Presenter agent. The user reports an issue
    /// with the running demo (wrong API URL, server didn't come up, etc.);
    /// the agent decides:
    ///   - launch-side problem → fixes via Bash (restart, env, port)
    ///   - code-side bug → drafts a steering note via DRAFT_STEERING marker
    ///   - unclear → asks back
    pub async fn presenter_chat(
        self: Arc<Self>,
        user_message: String,
    ) -> AppResult<PresenterChatReply> {
        let project = self.project_snapshot();
        let prev = self
            .preview
            .lock()
            .await
            .instructions
            .clone()
            .unwrap_or_else(|| "(нет данных о прошлом запуске)".into());

        let prompt = format!(
            "Корень проекта: {} — ты в нём.\n\n--- ТВОЁ ПРОШЛОЕ СООБЩЕНИЕ ПОЛЬЗОВАТЕЛЮ ПРИ ЗАПУСКЕ ДЕМО ---\n{prev}\n\n--- ПОЛЬЗОВАТЕЛЬ ПИШЕТ СЕЙЧАС ---\n{user_message}\n\nРазберись, действуй по системному промпту.",
            project.root_path,
        );

        let inv = AgentInvocation {
            role: AgentRole::Presenter,
            system_prompt: presenter_chat_prompt().to_string(),
            user_prompt: prompt,
            cwd: PathBuf::from(&project.root_path),
            model: project.model_specialist.clone(),
            tools: tools_for(AgentRole::Presenter),
            permission_mode: project.permission_mode,
            max_turns: 20,
            claude_code_preset: true,
            cancel: Some(self.cancel_token()),
            backend: project.agent_backend,
        };
        let publisher = self.event_publisher();
        let res = run_agent(inv, move |ev| {
            publisher.publish_agent_event(ev, None, None);
        })
        .await?;

        let raw = res.final_text.trim().to_string();
        let (reply, draft_steering) = split_draft_steering(&raw);
        Ok(PresenterChatReply {
            reply,
            draft_steering,
        })
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
            backend: project.agent_backend,
        }
    }
}

/// Split the Presenter's chat reply into (markdown-for-user, optional draft
/// steering). The marker block, if present, is removed from the user-facing
/// text — UI shows the suggestion as a separate "apply to steering" badge.
fn split_draft_steering(text: &str) -> (String, Option<String>) {
    const BEGIN: &str = "DRAFT_STEERING_BEGIN";
    const END: &str = "DRAFT_STEERING_END";
    let Some(b) = text.find(BEGIN) else {
        return (text.trim().to_string(), None);
    };
    let after_begin = &text[b + BEGIN.len()..];
    let Some(e) = after_begin.find(END) else {
        // Malformed — agent opened the marker but didn't close it. Keep the
        // full text visible so nothing is silently lost.
        return (text.trim().to_string(), None);
    };
    let draft = after_begin[..e].trim().to_string();
    let before = text[..b].trim();
    let after = after_begin[e + END.len()..].trim();
    let reply = match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (false, true) => before.to_string(),
        (true, false) => after.to_string(),
        (false, false) => format!("{before}\n\n{after}"),
    };
    let draft_opt = (!draft.is_empty()).then_some(draft);
    (reply, draft_opt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_marker_passes_through() {
        let (reply, draft) = split_draft_steering("просто текст");
        assert_eq!(reply, "просто текст");
        assert_eq!(draft, None);
    }

    #[test]
    fn extracts_marker_block() {
        let text = "Это баг в коде.\n\nDRAFT_STEERING_BEGIN\nИсправь API URL в client.ts\nDRAFT_STEERING_END";
        let (reply, draft) = split_draft_steering(text);
        assert_eq!(reply, "Это баг в коде.");
        assert_eq!(draft.as_deref(), Some("Исправь API URL в client.ts"));
    }

    #[test]
    fn malformed_marker_keeps_full_text() {
        let (reply, draft) = split_draft_steering("оборванный DRAFT_STEERING_BEGIN без конца");
        assert_eq!(reply, "оборванный DRAFT_STEERING_BEGIN без конца");
        assert_eq!(draft, None);
    }
}
