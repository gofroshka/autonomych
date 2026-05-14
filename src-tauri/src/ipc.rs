//! Tauri command surface — mirrors the IPC contract of the TS version.

use crate::conductor::Conductor;
use crate::error::{AppError, AppResult};
use crate::events::EventBus;
use crate::store::Store;
use crate::types::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;

pub struct AppState {
    pub store: Arc<Store>,
    pub bus: Arc<dyn EventBus>,
    pub conductors: Mutex<HashMap<String, Arc<Conductor>>>,
}

impl AppState {
    async fn get_or_create(&self, project_id: &str) -> AppResult<Arc<Conductor>> {
        let mut g = self.conductors.lock().await;
        if let Some(c) = g.get(project_id) {
            return Ok(c.clone());
        }
        let project = self
            .store
            .get_project(project_id)
            .ok_or_else(|| AppError::ProjectNotFound(project_id.to_string()))?;
        let c = Arc::new(Conductor::new(
            project,
            self.store.clone(),
            self.bus.clone(),
        ));
        g.insert(project_id.to_string(), c.clone());
        Ok(c)
    }
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> AppResult<Vec<ProjectRow>> {
    Ok(state.store.list_projects())
}

#[tauri::command]
pub async fn create_project(
    state: State<'_, AppState>,
    input: CreateProjectInput,
) -> AppResult<ProjectRow> {
    state.store.create_project(input)
}

#[tauri::command]
pub async fn delete_project(
    state: State<'_, AppState>,
    id: String,
    delete_files: bool,
) -> AppResult<()> {
    {
        let mut g = state.conductors.lock().await;
        if let Some(c) = g.remove(&id) {
            let _ = c.stop().await;
        }
    }
    let project = state.store.get_project(&id);
    state.store.delete_project(&id)?;
    if delete_files {
        if let Some(p) = project {
            let _ = tokio::fs::remove_dir_all(&p.root_path).await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn rename_project(
    state: State<'_, AppState>,
    id: String,
    name: String,
    idea: String,
) -> AppResult<()> {
    state.store.rename_project(&id, &name, &idea)?;
    if let Some(c) = state.conductors.lock().await.get(&id) {
        if let Some(p) = state.store.get_project(&id) {
            c.refresh_project(p);
        }
    }
    Ok(())
}

/// Update the project's CLI / models / permission mode. Blocked while a
/// run is in flight — the rest of the iteration would mix old and new
/// values otherwise. If a conductor is parked (Idle / Presenting / Error),
/// hot-swap its cached `ProjectRow` so the next iteration picks them up.
#[tauri::command]
pub async fn update_project_settings(
    state: State<'_, AppState>,
    id: String,
    model_pm: String,
    model_specialist: String,
    permission_mode: PermissionMode,
    agent_backend: AgentBackend,
) -> AppResult<ProjectRow> {
    if let Some(p) = state.store.get_project(&id) {
        match p.state {
            ConductorState::Running
            | ConductorState::WrappingUp
            | ConductorState::Resuming
            | ConductorState::PreparingPreview => {
                return Err(AppError::Other(
                    "проект сейчас работает — настройки можно менять после остановки".into(),
                ));
            }
            _ => {}
        }
    }
    let updated = state.store.update_project_settings(
        &id,
        &model_pm,
        &model_specialist,
        permission_mode,
        agent_backend,
    )?;
    if let Some(c) = state.conductors.lock().await.get(&id) {
        c.refresh_project(updated.clone());
    }
    Ok(updated)
}

#[tauri::command]
pub async fn open_project(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<ProjectRow>> {
    Ok(state.store.get_project(&id))
}

#[tauri::command]
pub async fn get_snapshot(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<DashboardSnapshot> {
    let project = state.store.get_project(&project_id);
    let iteration = project
        .as_ref()
        .and_then(|p| state.store.current_iteration(&p.id));
    let tasks = iteration
        .as_ref()
        .map(|i| state.store.iteration_tasks(&i.id))
        .unwrap_or_default();
    let recent_events = project
        .as_ref()
        .map(|p| state.store.recent_events(&p.id, 100, 0))
        .unwrap_or_default();
    let pending_steering = project.as_ref().and_then(|p| state.store.pending_steering(&p.id));
    let pending_questions = project
        .as_ref()
        .map(|p| state.store.pending_questions(&p.id))
        .unwrap_or_default();
    let (preview, cooldown) = {
        let g = state.conductors.lock().await;
        if let Some(c) = g.get(&project_id) {
            (c.preview.lock().await.status(), c.cooldown_info())
        } else {
            (
                PreviewStatus {
                    instructions: None,
                    prepared_at: None,
                    prep_error: None,
                },
                None,
            )
        }
    };
    // Snapshot only the items PO/UI cares about (active + recently closed
    // last few). Full archive comes through the dedicated list_backlog cmd.
    let backlog = state.store.active_backlog(&project_id);
    Ok(DashboardSnapshot {
        project,
        iteration,
        tasks,
        recent_events,
        pending_steering,
        pending_questions,
        preview,
        cooldown,
        backlog,
    })
}

#[tauri::command]
pub async fn get_events(
    state: State<'_, AppState>,
    project_id: String,
    since_ts: Option<i64>,
) -> AppResult<Vec<EventRow>> {
    Ok(state.store.recent_events(&project_id, 500, since_ts.unwrap_or(0)))
}

#[tauri::command]
pub async fn start_conductor(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    let c = state.get_or_create(&project_id).await?;
    // If we're currently sleeping out a provider cooldown, the user
    // pressing Start means "продолжить сейчас" — wake the existing
    // run_loop instead of spinning up a competing one.
    if c.skip_cooldown() {
        return Ok(());
    }
    c.start().await
}

#[tauri::command]
pub async fn start_presentation_only(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    let c = state.get_or_create(&project_id).await?;
    c.start_presentation_only().await
}

#[tauri::command]
pub async fn stop_conductor(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    if let Some(c) = state.conductors.lock().await.get(&project_id).cloned() {
        c.stop().await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn request_wrap_up(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    if let Some(c) = state.conductors.lock().await.get(&project_id).cloned() {
        c.request_wrap_up().await;
    }
    Ok(())
}

/// Queue a steering message for the next iteration's PO without waking any
/// parked conductor. Used from the dashboard when the user wants to give
/// initial direction *before* pressing Start, or in the Idle state more
/// generally. `resume` also pushes steering as a side-effect, but it
/// additionally fires the resume waker — which is wrong when we're not
/// already in Presenting.
#[tauri::command]
pub async fn push_steering(
    state: State<'_, AppState>,
    project_id: String,
    message: String,
    mode: String,
) -> AppResult<()> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let m = match mode.as_str() {
        "override" => SteeringMode::Override,
        _ => SteeringMode::Soft,
    };
    state.store.push_steering(&project_id, trimmed, m)?;
    Ok(())
}

#[tauri::command]
pub async fn resume(
    state: State<'_, AppState>,
    project_id: String,
    message: String,
    mode: String,
) -> AppResult<()> {
    let trimmed = message.trim();
    if !trimmed.is_empty() {
        let m = match mode.as_str() {
            "override" => SteeringMode::Override,
            _ => SteeringMode::Soft,
        };
        let _ = state.store.push_steering(&project_id, trimmed, m);
    }
    if let Some(c) = state.conductors.lock().await.get(&project_id).cloned() {
        c.resume().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_preview(_state: State<'_, AppState>, _project_id: String) -> AppResult<()> {
    // No-op in the agent-driven preview model: stopping the demo is what the
    // shutdown agent does, which fires when the user presses "Продолжаем" or
    // when the conductor is fully stopped. Kept as a callable command so the
    // frontend's existing call sites compile.
    Ok(())
}

#[tauri::command]
pub async fn presenter_chat(
    state: State<'_, AppState>,
    project_id: String,
    text: String,
) -> AppResult<PresenterChatReply> {
    let c = state
        .conductors
        .lock()
        .await
        .get(&project_id)
        .cloned()
        .ok_or_else(|| AppError::ProjectNotFound(project_id.clone()))?;
    c.presenter_chat(text).await
}

#[tauri::command]
pub async fn retry_preview(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    let c = state.get_or_create(&project_id).await?;
    let me = c.clone();
    tokio::spawn(async move {
        // start_presentation_only handles the "no resumable iteration" case
        // → just re-invokes preview prep. The launch agent is told to clean
        // up any prior server on the same port.
        let _ = me.start_presentation_only().await;
    });
    Ok(())
}

#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    question_id: String,
    answer: String,
) -> AppResult<()> {
    let q = state.store.get_question(&question_id);
    if let Some(q) = q {
        if let Some(c) = state.conductors.lock().await.get(&q.project_id).cloned() {
            c.answer_question(&question_id, answer).await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_chat_history(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<Vec<ChatMessageRow>> {
    Ok(state.store.chat_history(&project_id))
}

#[tauri::command]
pub async fn send_chat_message(
    state: State<'_, AppState>,
    project_id: String,
    text: String,
) -> AppResult<ChatMessageRow> {
    // Push user message first.
    let _ = state.store.push_chat(&project_id, ChatRole::User, text.clone(), None)?;
    let project = state
        .store
        .get_project(&project_id)
        .ok_or_else(|| AppError::ProjectNotFound(project_id.clone()))?;

    let history = state.store.chat_history(&project_id);
    let recent_iters = state.store.recent_iterations(&project_id, 6);
    let current_iter = state.store.current_iteration(&project_id);
    let tasks = current_iter
        .as_ref()
        .map(|i| state.store.iteration_tasks(&i.id))
        .unwrap_or_default();

    let context = format!(
        "Идея проекта: {}\nКорень проекта: {}\nТекущее состояние цикла: {:?}\n\n--- История последних итераций ---\n{}\n\n--- Текущая итерация и задачи ---\n{}\n\n--- История чата ---\n{}\n\n--- Сообщение пользователя ---\n{}",
        project.idea,
        project.root_path,
        project.state,
        recent_iters
            .iter()
            .map(|i| format!("#{} [{:?}]\n{}", i.number, i.status, i.summary.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n\n"),
        match &current_iter {
            Some(i) => format!(
                "#{} [{:?}]\n{}",
                i.number,
                i.status,
                tasks
                    .iter()
                    .map(|t| format!("- [{:?}] {:?}: {}", t.status, t.role, t.title))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            None => "(нет активной итерации)".into(),
        },
        history
            .iter()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .iter()
            .rev()
            .map(|m| format!(
                "{}: {}",
                if matches!(m.role, ChatRole::User) { "USER" } else { "ОТВЕТ" },
                m.text
            ))
            .collect::<Vec<_>>()
            .join("\n\n"),
        text,
    );

    let inv = crate::agents::AgentInvocation {
        role: AgentRole::Overseer,
        system_prompt: crate::agents::system_prompt(AgentRole::Overseer, false, false).to_string(),
        user_prompt: context,
        cwd: PathBuf::from(&project.root_path),
        model: project.model_pm.clone(),
        tools: crate::agents::tools_for(AgentRole::Overseer),
        permission_mode: PermissionMode::AcceptEdits,
        max_turns: 8,
        claude_code_preset: true,
        cancel: None,
        backend: project.agent_backend,
    };
    let answer_text = match crate::agents::run_agent(inv, |_| {}).await {
        Ok(r) => {
            let t = r.final_text.trim().to_string();
            if t.is_empty() {
                "(пустой ответ)".into()
            } else {
                t
            }
        }
        Err(e) => format!("Извини, не получилось ответить: {e}"),
    };
    state.store.push_chat(&project_id, ChatRole::Assistant, answer_text, None)
}

#[tauri::command]
pub async fn get_iteration_history(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<Vec<HistoryEntry>> {
    let iters = state.store.iterations_by_project(&project_id);
    let out = iters
        .into_iter()
        .map(|i| HistoryEntry {
            tasks: state.store.iteration_tasks(&i.id),
            iteration: i,
        })
        .collect();
    Ok(out)
}

#[tauri::command]
pub async fn pick_directory(app: AppHandle) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Где разместить проект")
        .pick_folder(move |path| {
            let _ = tx.send(path.and_then(|p| p.into_path().ok().map(|p| p.to_string_lossy().into_owned())));
        });
    Ok(rx.await.unwrap_or(None))
}

#[tauri::command]
pub async fn open_external(app: AppHandle, path: String) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if path.starts_with("http://") || path.starts_with("https://") {
        let _ = app.opener().open_url(&path, None::<&str>);
    } else {
        let _ = app.opener().open_path(&path, None::<&str>);
    }
    Ok(())
}

// ---- Backlog ----

/// Full backlog including done/dismissed history. The dashboard snapshot
/// only carries active items — UI can fetch the whole archive on demand
/// (e.g. when the user opens a "show all" view).
#[tauri::command]
pub async fn list_backlog(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<Vec<BacklogItem>> {
    Ok(state.store.list_backlog(&project_id))
}

#[tauri::command]
pub async fn add_backlog_item(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
    details: Option<String>,
    category: Option<BacklogCategory>,
    priority: Option<BacklogPriority>,
) -> AppResult<BacklogItem> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::Other("заголовок не может быть пустым".into()));
    }
    state.store.add_backlog(
        &project_id,
        NewBacklogItem {
            title: trimmed.into(),
            details: details.unwrap_or_default(),
            source: BacklogSource::UserSteering,
            category: category.unwrap_or_default(),
            priority: priority.unwrap_or_default(),
            origin_iteration_id: None,
            origin_task_id: None,
        },
    )
}

#[tauri::command]
pub async fn update_backlog_item(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
    details: Option<String>,
    priority: Option<BacklogPriority>,
) -> AppResult<()> {
    state.store.update_backlog(&id, title, details, priority)
}

#[tauri::command]
pub async fn dismiss_backlog_item(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.store.dismiss_backlog(&id)
}
