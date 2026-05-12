//! Preview state holder + crash recovery. Agent does the actual launching
//! and shutdown through Bash; we just track manifests on disk and provide
//! a fallback kill so we never leak processes.

use crate::error::AppResult;
use crate::types::PreviewStatus;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewManifest {
    pub pid: u32,
    #[serde(default)]
    pub url: Option<String>,
    pub command: String,
    #[serde(default)]
    pub log_file: Option<String>,
    #[serde(default)]
    pub shutdown_steps: Vec<String>,
    pub started_at: i64,
}

#[derive(Debug, Default)]
pub struct PreviewState {
    pub setup_steps: Vec<String>,
    pub notes: String,
    pub errors: Vec<String>,
    pub prepared_at: Option<i64>,
    pub prep_error: Option<String>,
    pub manifest: Option<PreviewManifest>,
    pub root_path: Option<PathBuf>,
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(".autonomych").join("preview.json")
}

fn log_path(root: &Path, m: &Option<PreviewManifest>) -> PathBuf {
    if let Some(m) = m.as_ref() {
        if let Some(p) = &m.log_file {
            return root.join(p);
        }
    }
    root.join(".autonomych").join("preview.log")
}

impl PreviewState {
    pub fn reset_prep(&mut self) {
        self.setup_steps.clear();
        self.notes.clear();
        self.errors.clear();
        self.prepared_at = None;
        self.prep_error = None;
    }

    pub async fn refresh_manifest(&mut self, root: &Path) {
        self.root_path = Some(root.to_path_buf());
        let p = manifest_path(root);
        match fs::read_to_string(&p).await {
            Ok(s) => self.manifest = serde_json::from_str(&s).ok(),
            Err(_) => self.manifest = None,
        }
    }

    pub fn is_alive(&self) -> bool {
        let Some(m) = &self.manifest else { return false };
        // SIGNAL 0 to check liveness without changing anything.
        // SAFETY: just a kill(0, pid) — std::process doesn't expose this so
        // we shell out.
        std::process::Command::new("kill")
            .args(["-0", &m.pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub async fn tail_log(&self) -> String {
        let Some(root) = &self.root_path else { return String::new() };
        let path = log_path(root, &self.manifest);
        match fs::read_to_string(&path).await {
            Ok(s) => {
                if s.len() > 4000 {
                    s[s.len() - 4000..].to_string()
                } else {
                    s
                }
            }
            Err(_) => String::new(),
        }
    }

    /// Best-effort fallback kill. Sends SIGTERM to the process group then
    /// drops the manifest file.
    pub async fn fallback_kill(&mut self) {
        if let Some(m) = self.manifest.take() {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &format!("-{}", m.pid)])
                .status();
            // SIGTERM to single PID as fallback if -pgid didn't apply.
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &m.pid.to_string()])
                .status();
        }
        if let Some(root) = &self.root_path {
            let _ = fs::remove_file(manifest_path(root)).await;
        }
    }

    pub async fn status(&self) -> PreviewStatus {
        PreviewStatus {
            running: self.is_alive(),
            pid: self.manifest.as_ref().map(|m| m.pid),
            url: self.manifest.as_ref().and_then(|m| m.url.clone()),
            command: self.manifest.as_ref().map(|m| m.command.clone()),
            logs_tail: self.tail_log().await,
            setup_steps: self.setup_steps.clone(),
            notes: self.notes.clone(),
            errors: self.errors.clone(),
            prepared_at: self.prepared_at,
            prep_error: self.prep_error.clone(),
        }
    }
}

/// Reap a stale preview manifest left over by a previous crash. Called once
/// at app startup per project.
pub async fn reap_orphans(root: &Path) -> AppResult<()> {
    let p = manifest_path(root);
    if !p.exists() {
        return Ok(());
    }
    if let Ok(content) = fs::read_to_string(&p).await {
        if let Ok(m) = serde_json::from_str::<PreviewManifest>(&content) {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &format!("-{}", m.pid)])
                .status();
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &m.pid.to_string()])
                .status();
        }
    }
    let _ = fs::remove_file(&p).await;
    Ok(())
}
