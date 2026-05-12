use std::path::Path;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod agents;
mod conductor;
mod error;
mod events;
mod git;
mod ipc;
mod store;
mod types;
mod util;

use events::TauriEventBus;
use ipc::AppState;

/// Configure the tracing pipeline: human-readable stderr + structured JSON
/// rolling-by-day file under `<data_dir>/logs/`. The file layer is the one
/// you grep after the app has been running unattended for hours.
fn init_tracing(data_dir: &Path) {
    let logs_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&logs_dir);

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "autonomych.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // Worker guard must outlive the program — leak it intentionally.
    Box::leak(Box::new(guard));

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,autonomych_lib=debug".into());

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true);
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false);

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map(|p| p.join("data"))
                .unwrap_or_else(|_| std::env::temp_dir().join("autonomych-data"));
            init_tracing(&data_dir);
            tracing::info!(
                ?data_dir,
                "Autonomych starting; logs at {}",
                data_dir.join("logs").display()
            );

            let store = Arc::new(store::Store::open(data_dir)?);
            let reaped = store.reset_stale_states()?;
            tracing::info!(
                iters = reaped.0,
                tasks = reaped.1,
                questions = reaped.2,
                "reset stale states on startup"
            );
            // Best-effort cleanup of orphan worktrees from prior runs. Any
            // dev-servers left running by a previous Presenter agent are NOT
            // killed automatically — the user can re-enter Presenting and
            // let the agent clean them up via its own shutdown logic.
            for p in store.list_projects() {
                let root = std::path::PathBuf::from(p.root_path);
                tauri::async_runtime::spawn(async move {
                    let _ = git::cleanup_orphan_worktrees(&root).await;
                });
            }
            let bus = TauriEventBus::arced(app.handle().clone());
            app.manage(AppState {
                store,
                bus,
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
        .unwrap_or_else(|e| {
            tracing::error!("tauri runtime crashed: {e}");
            std::process::exit(1);
        });
}
