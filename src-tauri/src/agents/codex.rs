//! Bridge to the OpenAI Codex CLI via its `exec --json` JSONL protocol.
//!
//! Codex CLI streams events as line-delimited JSON. The shapes we care about:
//!
//! ```text
//! {"type":"thread.started","thread_id":"..."}
//! {"type":"turn.started"}
//! {"type":"item.completed","item":{"type":"agent_message","text":"..."}}
//! {"type":"item.started","item":{"type":"command_execution","command":"...","status":"in_progress"}}
//! {"type":"item.completed","item":{"type":"command_execution","command":"...","aggregated_output":"...","exit_code":0,"status":"completed"}}
//! {"type":"turn.completed","usage":{...}}
//! ```
//!
//! Codex doesn't have a `--system-prompt` flag, so we prepend the role's
//! system prompt into the user message as a leading block. Codex's
//! sandbox modes (`read-only`, `workspace-write`, `danger-full-access`)
//! map onto our `PermissionMode`.
//!
//! NB: this protocol is internal to Codex CLI; if a new version breaks
//! parsing, update the `CodexEvent` enum below.

use super::{AgentEvent, AgentInvocation, AgentResult};
use crate::error::{AppError, AppResult};
use crate::types::PermissionMode;
use serde::Deserialize;
use serde_json::Value;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const HEARTBEAT_TICK: Duration = Duration::from_secs(60);
const SILENCE_WARN_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const POST_TURN_GRACE: Duration = Duration::from_secs(3);

/// Codex CLI exec event. Top-level discriminator is `type`. Inner item
/// payloads have their own `type` field, modeled with [`CodexItem`].
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted {
        #[allow(dead_code)]
        #[serde(default)]
        thread_id: Option<String>,
    },
    #[serde(rename = "turn.started")]
    TurnStarted,
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        #[allow(dead_code)]
        #[serde(default)]
        usage: Option<Value>,
    },
    #[serde(rename = "item.started")]
    ItemStarted {
        #[serde(default)]
        item: Option<CodexItem>,
    },
    #[serde(rename = "item.completed")]
    ItemCompleted {
        #[serde(default)]
        item: Option<CodexItem>,
    },
    /// API-level / transport error from Codex. `message` is typically a
    /// nested JSON string with `{type, status, error: {message, ...}}`.
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        message: String,
    },
    /// Turn ended with a failure on Codex's side (model error, quota, etc).
    /// We treat this as a hard stop and surface the inner message.
    #[serde(rename = "turn.failed")]
    TurnFailed {
        #[serde(default)]
        error: Option<TurnFailedError>,
    },
    /// Catch-all for events Codex adds in future versions without breaking us.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct TurnFailedError {
    #[serde(default)]
    message: String,
}

/// Codex frequently wraps the human-readable failure inside a JSON-encoded
/// string of the form `{"type":"error","status":400,"error":{"message":"..."}}`.
/// Peel it once if possible; otherwise return the raw string.
fn extract_codex_error_message(raw: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(inner) = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
            return inner.to_string();
        }
        if let Some(top) = v.get("message").and_then(|m| m.as_str()) {
            return top.to_string();
        }
    }
    raw.to_string()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexItem {
    /// Agent's natural-language message to the user.
    #[serde(rename = "agent_message")]
    AgentMessage {
        #[allow(dead_code)]
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        text: String,
    },
    /// Shell command execution. `aggregated_output` is the full stdout+stderr
    /// captured by Codex; `exit_code` is None while running, Some when done.
    #[serde(rename = "command_execution")]
    CommandExecution {
        #[allow(dead_code)]
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        command: String,
        #[serde(default)]
        aggregated_output: String,
        #[serde(default)]
        exit_code: Option<i32>,
        #[allow(dead_code)]
        #[serde(default)]
        status: Option<String>,
    },
    /// Anything else (file_read, file_write, mcp_tool_call, etc.) — surface
    /// generically so the activity log shows something rather than nothing.
    #[serde(other)]
    Other,
}

fn sandbox_arg(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "read-only",
        PermissionMode::AcceptEdits => "workspace-write",
        PermissionMode::BypassPermissions => "danger-full-access",
    }
}

/// Compose Codex's input — system prompt + actual task — since Codex CLI
/// has no separate `--system-prompt` flag.
fn compose_prompt(inv: &AgentInvocation) -> String {
    if inv.system_prompt.trim().is_empty() {
        return inv.user_prompt.clone();
    }
    format!(
        "SYSTEM INSTRUCTIONS (read first, follow strictly):\n\n{}\n\n---\n\nUSER TASK:\n\n{}",
        inv.system_prompt.trim(),
        inv.user_prompt
    )
}

/// Run an agent under Codex CLI to completion. Same event surface as the
/// Claude runner, so the conductor doesn't care which backend it dispatched.
#[tracing::instrument(
    skip(inv, on_event),
    fields(role = ?inv.role, model = %inv.model, cwd = %inv.cwd.display()),
)]
pub(super) async fn run_codex_agent<F>(
    inv: AgentInvocation,
    mut on_event: F,
) -> AppResult<AgentResult>
where
    F: FnMut(AgentEvent) + Send,
{
    let started = Instant::now();
    let role = inv.role;
    tracing::info!(model = %inv.model, perm = ?inv.permission_mode, "codex agent: spawn");
    on_event(AgentEvent::Start { role });

    let mut cmd = Command::new("codex");
    cmd.arg("exec")
        .arg("--json")
        .args(["--model", &inv.model])
        .args(["-C", &inv.cwd.to_string_lossy()])
        // Codex normally refuses to run outside a git repo; specialist
        // worktrees are git, but main project root might not be on first
        // iteration. Skip the check uniformly.
        .arg("--skip-git-repo-check");

    // `--dangerously-bypass-approvals-and-sandbox` already implies "no sandbox,
    // no approvals" and conflicts with `--sandbox` on recent codex versions.
    // Only pass `--sandbox` when we're NOT in full-bypass mode.
    if matches!(inv.permission_mode, PermissionMode::BypassPermissions) {
        cmd.arg("--dangerously-bypass-approvals-and-sandbox");
    } else {
        cmd.args(["--sandbox", sandbox_arg(inv.permission_mode)]);
    }

    cmd.current_dir(&inv.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Codex doesn't honour Claude's tool gating / preset flag; the agent's
    // sandbox handles permissions instead. Suppress unused-field warnings.
    let _ = &inv.tools;
    let _ = inv.claude_code_preset;

    let mut child: Child = cmd
        .spawn()
        .map_err(|e| AppError::Agent(format!("spawn codex: {e}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::Agent("no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Agent("no stdout".into()))?;

    let composed = compose_prompt(&inv);
    let _ = stdin.write_all(composed.as_bytes()).await;
    let _ = stdin.flush().await;
    drop(stdin); // EOF → Codex starts processing

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let stdout_reader = BufReader::new(stdout);
    let tx_clone = tx.clone();
    let stdout_task = tokio::spawn(async move {
        let mut lines = stdout_reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx_clone.send(line);
        }
    });
    if let Some(err) = child.stderr.take() {
        drop(tx);
        tokio::spawn(async move {
            let mut r = BufReader::new(err).lines();
            while let Ok(Some(line)) = r.next_line().await {
                tracing::warn!(target: "codex.stderr", "{line}");
            }
        });
    } else {
        drop(tx);
    }

    let mut turns = 0u32;
    let mut final_text = String::new();
    let mut saw_turn_completed = false;
    // Populated from `error` / `turn.failed` events so we can return a
    // clean error string instead of generic "exited with code N".
    let mut codex_error: Option<String> = None;

    // Inactivity watchdog — same diagnostic role as the Claude runner.
    let last_event_ms = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis()));
    let watchdog_cancel = CancellationToken::new();
    {
        let le = last_event_ms.clone();
        let wd = watchdog_cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = wd.cancelled() => return,
                    _ = tokio::time::sleep(HEARTBEAT_TICK) => {}
                }
                let now = chrono::Utc::now().timestamp_millis();
                let last = le.load(Ordering::Relaxed);
                let silent = Duration::from_millis((now - last).max(0) as u64);
                if silent >= SILENCE_WARN_THRESHOLD {
                    tracing::warn!(
                        ?role,
                        silent_secs = silent.as_secs(),
                        "codex agent silent for over threshold — possibly stuck",
                    );
                }
            }
        });
    }

    let cancel = inv.cancel.clone();
    loop {
        let line_opt: Option<String> = if let Some(tok) = &cancel {
            tokio::select! {
                _ = tok.cancelled() => {
                    tracing::warn!(?role, elapsed_ms = started.elapsed().as_millis() as u64, "codex agent: cancelled by user");
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    on_event(AgentEvent::AgentError { role, message: "aborted".into() });
                    watchdog_cancel.cancel();
                    return Err(AppError::Agent("aborted by user".into()));
                }
                v = rx.recv() => v
            }
        } else {
            rx.recv().await
        };
        let Some(line) = line_opt else { break };
        last_event_ms.store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CodexEvent>(&line) {
            Ok(CodexEvent::ThreadStarted { .. }) => {
                tracing::debug!(?role, "codex: thread.started");
            }
            Ok(CodexEvent::TurnStarted) => {
                turns += 1;
                tracing::debug!(?role, turn = turns, "codex: turn.started");
            }
            Ok(CodexEvent::TurnCompleted { .. }) => {
                saw_turn_completed = true;
                tracing::info!(?role, turns, "codex: turn.completed — agent done");
                // Codex emits exactly one turn.completed per `exec` run; that's
                // our equivalent of Claude's `result` message. Break to clean
                // up subprocess.
                break;
            }
            Ok(CodexEvent::ItemCompleted { item: Some(item) }) => match item {
                CodexItem::AgentMessage { text, .. } => {
                    final_text = text.clone();
                    on_event(AgentEvent::AssistantText { role, text });
                }
                CodexItem::CommandExecution {
                    command,
                    aggregated_output,
                    exit_code,
                    ..
                } => {
                    let is_error = exit_code.map(|c| c != 0).unwrap_or(false);
                    on_event(AgentEvent::ToolResult {
                        role,
                        content: aggregated_output.chars().take(4000).collect(),
                        is_error,
                    });
                    let _ = command;
                }
                CodexItem::Other => {}
            },
            Ok(CodexEvent::ItemStarted { item: Some(item) }) => {
                if let CodexItem::CommandExecution { command, .. } = item {
                    on_event(AgentEvent::ToolUse {
                        role,
                        tool: "Bash".into(),
                        input: serde_json::json!({ "command": command }),
                    });
                }
            }
            Ok(CodexEvent::Error { message }) => {
                let extracted = extract_codex_error_message(&message);
                tracing::warn!(?role, error = %extracted, "codex: error event");
                codex_error = Some(extracted);
            }
            Ok(CodexEvent::TurnFailed { error }) => {
                let raw = error.as_ref().map(|e| e.message.as_str()).unwrap_or("");
                let extracted = extract_codex_error_message(raw);
                tracing::warn!(?role, error = %extracted, "codex: turn.failed");
                // Prefer the more specific turn.failed message over a prior
                // generic `error` event, but don't overwrite with empty.
                if !extracted.trim().is_empty() {
                    codex_error = Some(extracted);
                }
            }
            Ok(CodexEvent::Other) => {
                // Unknown event `type` — surface raw line so we can debug
                // codex version drift instead of silently swallowing.
                tracing::warn!(?role, raw = %line, "codex: unrecognized event");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(?role, raw = %line, error = %e, "codex: non-json line on stdout");
            }
        }
    }
    drop(rx);
    let stream_closed = Instant::now();
    tracing::info!(
        ?role,
        turns,
        saw_turn_completed,
        final_text_len = final_text.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "codex agent: stream loop done — finalizing",
    );

    let exit = match tokio::time::timeout(POST_TURN_GRACE, child.wait()).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!(
                ?role,
                "codex agent: subprocess didn't exit after turn done — force killing",
            );
            let _ = child.start_kill();
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(res) => res,
                Err(_) => {
                    tracing::error!(?role, "codex: subprocess didn't die even after SIGKILL");
                    Ok(std::process::ExitStatus::default())
                }
            }
        }
    };
    stdout_task.abort();
    tracing::info!(
        ?role,
        ?exit,
        cleanup_ms = stream_closed.elapsed().as_millis() as u64,
        total_ms = started.elapsed().as_millis() as u64,
        turns,
        "codex agent: finalized",
    );
    watchdog_cancel.cancel();
    let duration_ms = started.elapsed().as_millis() as u64;

    // Codex failed before completing the turn AND produced no text. Without
    // this guard the conductor sees an "ok" result with an empty body and
    // retries indefinitely. Surface the failure so iteration_failed kicks
    // in and the user actually sees an error.
    let exit_code = exit.as_ref().ok().and_then(|s| s.code());
    let exit_nonzero = exit_code.map(|c| c != 0).unwrap_or(false);
    if !saw_turn_completed && final_text.trim().is_empty() && (exit_nonzero || codex_error.is_some()) {
        let msg = if let Some(err) = codex_error {
            format!("codex (model `{}`) failed: {}", inv.model, err)
        } else {
            format!(
                "codex exited with code {} before completing the turn. Проверь авторизацию `codex login` \
                 и доступность модели `{}` на твоём тарифе.",
                exit_code.unwrap_or(-1),
                inv.model
            )
        };
        on_event(AgentEvent::AgentError {
            role,
            message: msg.clone(),
        });
        return Err(AppError::Agent(msg));
    }

    on_event(AgentEvent::End {
        role,
        final_text: final_text.clone(),
        turns,
        duration_ms,
    });
    Ok(AgentResult {
        final_text,
        turns,
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentRole;

    #[test]
    fn sandbox_arg_maps_permission_modes() {
        assert_eq!(sandbox_arg(PermissionMode::Default), "read-only");
        assert_eq!(sandbox_arg(PermissionMode::AcceptEdits), "workspace-write");
        assert_eq!(
            sandbox_arg(PermissionMode::BypassPermissions),
            "danger-full-access"
        );
    }

    #[test]
    fn compose_prompt_prepends_system_section() {
        let inv = AgentInvocation {
            role: AgentRole::Architect,
            system_prompt: "You are X.".into(),
            user_prompt: "Do Y.".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            model: "gpt-5-codex".into(),
            tools: vec![],
            permission_mode: PermissionMode::Default,
            claude_code_preset: false,
            cancel: None,
            backend: crate::types::AgentBackend::Codex,
        };
        let out = compose_prompt(&inv);
        assert!(out.starts_with("SYSTEM INSTRUCTIONS"));
        assert!(out.contains("You are X."));
        assert!(out.contains("USER TASK"));
        assert!(out.contains("Do Y."));
    }

    #[test]
    fn parses_agent_message_event() {
        let line = r#"{"type":"item.completed","item":{"id":"i0","type":"agent_message","text":"hello"}}"#;
        let ev: CodexEvent = serde_json::from_str(line).unwrap();
        if let CodexEvent::ItemCompleted {
            item: Some(CodexItem::AgentMessage { text, .. }),
        } = ev
        {
            assert_eq!(text, "hello");
        } else {
            panic!("unexpected variant");
        }
    }

    #[test]
    fn parses_command_execution_done() {
        let line = r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"ls","aggregated_output":"file\n","exit_code":0,"status":"completed"}}"#;
        let ev: CodexEvent = serde_json::from_str(line).unwrap();
        if let CodexEvent::ItemCompleted {
            item:
                Some(CodexItem::CommandExecution {
                    command,
                    aggregated_output,
                    exit_code,
                    ..
                }),
        } = ev
        {
            assert_eq!(command, "ls");
            assert_eq!(aggregated_output, "file\n");
            assert_eq!(exit_code, Some(0));
        } else {
            panic!("unexpected variant");
        }
    }

    #[test]
    fn parses_turn_completed() {
        let line = r#"{"type":"turn.completed","usage":{"input_tokens":1}}"#;
        let ev: CodexEvent = serde_json::from_str(line).unwrap();
        assert!(matches!(ev, CodexEvent::TurnCompleted { .. }));
    }

    #[test]
    fn unknown_event_falls_through() {
        let line = r#"{"type":"some.future.event","foo":"bar"}"#;
        let ev: CodexEvent = serde_json::from_str(line).unwrap();
        assert!(matches!(ev, CodexEvent::Other));
    }
}
