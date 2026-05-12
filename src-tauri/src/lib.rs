use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

mod agents;
mod conductor;
mod error;
mod git;
mod ipc;
mod store;
mod types;

use ipc::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,autonomych_lib=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map(|p| p.join("data"))
                .unwrap_or_else(|_| std::env::temp_dir().join("autonomych-data"));
            let store = Arc::new(store::Store::open(data_dir)?);
            let reaped = store.reset_stale_states()?;
            tracing::info!(
                "Autonomych: reaped {} iterations, {} tasks, {} questions",
                reaped.0, reaped.1, reaped.2
            );
            // Best-effort cleanup of orphan worktrees and preview manifests.
            for p in store.list_projects() {
                let root = std::path::PathBuf::from(p.root_path.clone());
                let r1 = root.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = git::cleanup_orphan_worktrees(&r1).await;
                });
                tauri::async_runtime::spawn(async move {
                    let _ = conductor::preview::reap_orphans(&root).await;
                });
            }
            app.manage(AppState {
                store,
                conductors: Mutex::new(std::collections::HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::list_projects,
            ipc::create_project,
            ipc::delete_project,
            ipc::rename_project,
            ipc::open_project,
            ipc::get_snapshot,
            ipc::get_events,
            ipc::start_conductor,
            ipc::start_presentation_only,
            ipc::stop_conductor,
            ipc::request_wrap_up,
            ipc::resume,
            ipc::stop_preview,
            ipc::retry_preview,
            ipc::answer_question,
            ipc::get_chat_history,
            ipc::send_chat_message,
            ipc::get_iteration_history,
            ipc::pick_directory,
            ipc::open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
