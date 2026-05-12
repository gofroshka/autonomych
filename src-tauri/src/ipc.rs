//! Tauri command surface — mirrors the IPC contract of the TS version.

use crate::conductor::Conductor;
use crate::error::{AppError, AppResult};
use crate::store::Store;
use crate::types::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;

pub struct AppState {
    pub store: Arc<Store>,
    pub conductors: Mutex<HashMap<String, Arc<Conductor>>>,
}

impl AppState {
    async fn get_or_create(&self, project_id: &str, app: &AppHandle) -> AppResult<Arc<Conductor>> {
        let mut g = self.conductors.lock().await;
        if let Some(c) = g.get(project_id) {
            return Ok(c.clone());
        }
        let project = self
            .store
            .get_project(project_id)
            .ok_or_else(|| AppError::ProjectNotFound(project_id.to_string()))?;
        let c = Arc::new(Conductor::new(project, self.store.clone(), app.clone()));
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
    state.store.rename_project(&id, &name, &idea)
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
    let preview = {
        let g = state.conductors.lock().await;
        if let Some(c) = g.get(&project_id) {
            c.preview.lock().await.status().await
        } else {
            PreviewStatus {
                running: false,
                pid: None,
                url: None,
                command: None,
                setup_steps: vec![],
                notes: String::new(),
                errors: vec![],
                logs_tail: String::new(),
                prepared_at: None,
                prep_error: None,
            }
        }
    };
    Ok(DashboardSnapshot {
        project,
        iteration,
        tasks,
        recent_events,
        pending_steering,
        pending_questions,
        preview,
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
    app: AppHandle,
    project_id: String,
) -> AppResult<()> {
    let c = state.get_or_create(&project_id, &app).await?;
    c.start().await
}

#[tauri::command]
pub async fn start_presentation_only(
    state: State<'_, AppState>,
    app: AppHandle,
    project_id: String,
) -> AppResult<()> {
    let c = state.get_or_create(&project_id, &app).await?;
    c.start_presentation_only().await
}

#[tauri::command]
pub async fn stop_conductor(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    if let Some(c) = state.conductors.lock().await.get(&project_id).cloned() {
        c.stop().await?;
        c.preview.lock().await.fallback_kill().await;
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
        c.preview.lock().await.fallback_kill().await;
        c.resume().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_preview(
    state: State<'_, AppState>,
    project_id: String,
) -> AppResult<()> {
    if let Some(c) = state.conductors.lock().await.get(&project_id).cloned() {
        c.preview.lock().await.fallback_kill().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn retry_preview(
    state: State<'_, AppState>,
    app: AppHandle,
    project_id: String,
) -> AppResult<()> {
    let c = state.get_or_create(&project_id, &app).await?;
    c.preview.lock().await.fallback_kill().await;
    let me = c.clone();
    tokio::spawn(async move {
        // Call private prep via a public wrapper — we re-expose start_presentation_only
        // which handles the case "no resumable iteration" → just preview.
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
