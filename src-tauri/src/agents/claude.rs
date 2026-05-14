//! Bridge to the Claude Code CLI via stream-JSON protocol.
//!
//! We spawn `claude --print --output-format stream-json --input-format
//! stream-json` and exchange line-delimited JSON messages over stdio. This
//! mirrors what @anthropic-ai/claude-agent-sdk does under the hood but lets
//! us own the orchestration end without depending on the TS SDK.
//!
//! NB: the stream-JSON message shape is not a stable public API. Anthropic
//! may evolve it; if a new Claude Code version breaks parsing, update the
//! `SdkMessage` enum below.

use super::{AgentEvent, AgentInvocation, AgentResult};
use crate::error::{AppError, AppResult};
use crate::types::PermissionMode;
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// How often the inactivity watchdog wakes up and checks if anything has
/// been heard from the agent recently.
const HEARTBEAT_TICK: Duration = Duration::from_secs(60);
/// Silent gap after which the watchdog starts emitting warnings. Pure
/// diagnostic — we do NOT kill the agent, just surface the suspicion.
const SILENCE_WARN_THRESHOLD: Duration = Duration::from_secs(5 * 60);
/// After the agent explicitly says it's done via the stream-json `result`
/// message, how long we give the subprocess to exit on its own before
/// force-killing it. The agent already finished — this only governs cleanup
/// of an orphaned child process (typically a dev server left running in
/// the background for self-verification) which is holding the stdout pipe
/// open and would otherwise hang us forever.
const POST_RESULT_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum SdkMessage {
    #[serde(rename = "assistant")]
    Assistant {
        message: AssistantMessage,
        #[allow(dead_code)]
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "user")]
    User {
        message: UserMessage,
        #[allow(dead_code)]
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "result")]
    Result {
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        is_error: Option<bool>,
    },
    #[serde(rename = "system")]
    System,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct UserMessage {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: Option<bool>,
    },
    #[serde(other)]
    Other,
}

fn permission_arg(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::BypassPermissions => "bypassPermissions",
    }
}

/// Run an agent under Claude Code CLI to completion. Streams events through
/// `on_event`. Dispatched to from `agents::run_agent` based on
/// `inv.backend == AgentBackend::ClaudeCode`.
#[tracing::instrument(
    skip(inv, on_event),
    fields(role = ?inv.role, model = %inv.model, cwd = %inv.cwd.display()),
)]
pub(super) async fn run_claude_agent<F>(
    inv: AgentInvocation,
    mut on_event: F,
) -> AppResult<AgentResult>
where
    F: FnMut(AgentEvent) + Send,
{
    let started = Instant::now();
    let role = inv.role;
    tracing::info!(tools = ?inv.tools, perm = ?inv.permission_mode, "agent: spawn");
    on_event(AgentEvent::Start { role });

    let mut cmd = Command::new("claude");
    cmd.arg("--print")
        .arg("--verbose") // required when --print + --output-format stream-json
        .args(["--output-format", "stream-json"])
        .args(["--input-format", "stream-json"])
        .args(["--model", &inv.model])
        .args(["--permission-mode", permission_arg(inv.permission_mode)])
        // Isolate from user's MCP / settings. `--strict-mcp-config` makes the
        // CLI ignore every MCP server outside what we pass explicitly via
        // `--mcp-config` (which we don't pass — so no extra tools).
        .arg("--strict-mcp-config");

    if matches!(inv.permission_mode, PermissionMode::BypassPermissions) {
        cmd.arg("--dangerously-skip-permissions");
    }

    // `--tools` is variadic and uses "" to disable all built-in tools.
    if inv.tools.is_empty() {
        cmd.args(["--tools", ""]);
    } else {
        cmd.arg("--tools");
        for t in &inv.tools {
            cmd.arg(t);
        }
    }

    if !inv.claude_code_preset {
        cmd.args(["--system-prompt", &inv.system_prompt]);
    } else {
        cmd.args(["--append-system-prompt", &inv.system_prompt]);
    }
    cmd.current_dir(&inv.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Agents self-terminate per their system prompts; we never bound run
    // time at the CLI level. Codex has no equivalent flag either.

    let mut child: Child = cmd
        .spawn()
        .map_err(|e| AppError::Agent(format!("spawn claude: {e}")))?;

    let mut stdin = child.stdin.take().ok_or_else(|| AppError::Agent("no stdin".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| AppError::Agent("no stdout".into()))?;

    // Push the user prompt as a stream-JSON user message.
    let prompt_msg = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": inv.user_prompt,
        }
    });
    let _ = stdin.write_all(prompt_msg.to_string().as_bytes()).await;
    let _ = stdin.write_all(b"\n").await;
    let _ = stdin.flush().await;
    drop(stdin); // signal EOF — claude will respond and exit

    // Pipe stdout lines into a channel. We keep the handle so we can abort
    // the reader if an orphaned child process (e.g. a dev server left
    // running by the agent) keeps the pipe open after the agent has
    // already signalled done.
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
        // stderr is for diagnostics only; we never forward it to the message
        // channel because it isn't stream-JSON. Dropping `tx` here also lets
        // the stdout reader close `rx` once the child exits.
        drop(tx);
        tokio::spawn(async move {
            let mut r = BufReader::new(err).lines();
            while let Ok(Some(line)) = r.next_line().await {
                tracing::warn!(target: "claude.stderr", "{line}");
            }
        });
    } else {
        drop(tx);
    }

    let mut turns = 0u32;
    let mut final_text = String::new();
    let mut got_result_message = false;

    // Inactivity watchdog. Wakes every minute; if more than
    // SILENCE_WARN_THRESHOLD has passed without an event, emits a warning so
    // we have a marker in the log of "we've been quiet since X". Pure
    // diagnostic, never kills the agent.
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
                        "agent silent for over threshold — possibly stuck after final message",
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
                    tracing::warn!(?role, elapsed_ms = started.elapsed().as_millis() as u64, "agent: cancelled by user");
                    // Hard-kill the underlying process group so dev-server-style
                    // background children also die. kill_on_drop catches the
                    // worst case if Child drops while wait() is in flight.
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
        let parsed: Result<SdkMessage, _> = serde_json::from_str(&line);
        match parsed {
            Ok(SdkMessage::Assistant { message, .. }) => {
                turns += 1;
                tracing::debug!(?role, turn = turns, blocks = message.content.len(), "agent: assistant message");
                for block in message.content {
                    match block {
                        ContentBlock::Text { text } => {
                            final_text = text.clone();
                            on_event(AgentEvent::AssistantText { role, text });
                        }
                        ContentBlock::ToolUse { name, input } => {
                            tracing::debug!(?role, turn = turns, tool = %name, "agent: tool_use");
                            on_event(AgentEvent::ToolUse { role, tool: name, input });
                        }
                        _ => {}
                    }
                }
            }
            Ok(SdkMessage::User { message, .. }) => {
                for block in message.content {
                    if let ContentBlock::ToolResult { content, is_error } = block {
                        let text = match &content {
                            Value::String(s) => s.clone(),
                            Value::Array(arr) => arr
                                .iter()
                                .filter_map(|v| v.get("text").and_then(Value::as_str).map(str::to_string))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            _ => content.to_string(),
                        };
                        let err = is_error.unwrap_or(false);
                        if err {
                            tracing::debug!(?role, turn = turns, "agent: tool_result is_error");
                        }
                        on_event(AgentEvent::ToolResult {
                            role,
                            content: text.chars().take(4000).collect(),
                            is_error: err,
                        });
                    }
                }
            }
            Ok(SdkMessage::Result { result, is_error, .. }) => {
                got_result_message = true;
                tracing::info!(?role, turn = turns, is_error = is_error.unwrap_or(false), "agent: result message — agent indicates done");
                if let Some(t) = result {
                    if is_error.unwrap_or(false) {
                        on_event(AgentEvent::AgentError { role, message: t.clone() });
                    } else if !t.is_empty() {
                        final_text = t;
                    }
                }
                // The `result` message is the protocol's final signal.
                // Reading further is pointless — we already have the final
                // text, the turn count, the error flag. Break out so we can
                // clean up the subprocess; otherwise an orphaned child
                // (typically a stray `npm run dev` background process)
                // keeps the stdout pipe open and we hang forever on rx.recv.
                break;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::trace!("non-json line from claude: {line} ({e})");
            }
        }
    }
    drop(rx);
    let stream_closed = Instant::now();
    tracing::info!(
        ?role,
        turns,
        got_result_message,
        final_text_len = final_text.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "agent: stream loop done — finalizing subprocess",
    );

    // Cleanup phase. The agent has either signalled done (`got_result_message`)
    // or its stdout naturally closed. Give the subprocess a brief grace
    // window to exit on its own; if it doesn't (orphan children holding
    // the pipe open), SIGKILL it. This is NOT a timeout on the agent's
    // work — the agent has already finished.
    let exit = match tokio::time::timeout(POST_RESULT_GRACE, child.wait()).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!(
                ?role,
                grace_ms = POST_RESULT_GRACE.as_millis() as u64,
                "agent: subprocess didn't exit after result — force killing (orphan child likely)",
            );
            let _ = child.start_kill();
            // After SIGKILL, claude itself exits near-instantly. We still
            // bound this wait so a kernel-level stuck state doesn't hang us.
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(res) => res,
                Err(_) => {
                    tracing::error!(?role, "agent: subprocess didn't die even after SIGKILL");
                    Ok(std::process::ExitStatus::default())
                }
            }
        }
    };
    // The stdout reader may still be blocked reading from the pipe (an
    // orphan child holds the write end). Abort it so we don't leak the task.
    stdout_task.abort();
    let wait_duration = stream_closed.elapsed();
    tracing::info!(
        ?role,
        ?exit,
        cleanup_ms = wait_duration.as_millis() as u64,
        total_ms = started.elapsed().as_millis() as u64,
        turns,
        "agent: subprocess finalized",
    );
    watchdog_cancel.cancel();
    let duration_ms = started.elapsed().as_millis() as u64;
    on_event(AgentEvent::End { role, final_text: final_text.clone(), turns, duration_ms });
    Ok(AgentResult { final_text, turns, duration_ms })
}

/// Strip ```json fences and parse the first top-level JSON value from agent
/// final text. Tolerates trailing prose after the JSON value.
pub fn extract_json<T: for<'de> Deserialize<'de>>(text: &str) -> AppResult<T> {
    let stripped = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let first = stripped
        .find(['{', '['])
        .ok_or_else(|| AppError::Agent("no JSON in agent output".into()))?;
    let slice = &stripped[first..];
    // Try progressively shorter suffixes so trailing chatter ("Hope that
    // helps!") doesn't defeat the parser. Skip byte offsets that aren't on a
    // valid UTF-8 char boundary, otherwise slicing panics on multi-byte
    // characters like em-dashes.
    for end in (1..=slice.len()).rev() {
        if !slice.is_char_boundary(end) {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<T>(&slice[..end]) {
            return Ok(v);
        }
    }
    Err(AppError::Agent(
        "could not parse JSON from agent output".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Sample {
        a: u32,
        #[serde(default)]
        b: String,
    }

    #[test]
    fn parses_plain_json() {
        let v: Sample = extract_json(r#"{"a":1,"b":"x"}"#).unwrap();
        assert_eq!(v, Sample { a: 1, b: "x".into() });
    }

    #[test]
    fn strips_json_fences() {
        let v: Sample = extract_json("```json\n{\"a\":2}\n```").unwrap();
        assert_eq!(v.a, 2);
    }

    #[test]
    fn strips_bare_fences() {
        let v: Sample = extract_json("```\n{\"a\":3}\n```").unwrap();
        assert_eq!(v.a, 3);
    }

    #[test]
    fn tolerates_trailing_prose() {
        let v: Sample =
            extract_json(r#"Sure! Here you go: {"a":4} — that's the answer."#).unwrap();
        assert_eq!(v.a, 4);
    }

    #[test]
    fn finds_json_array() {
        let v: Vec<u32> = extract_json("Some intro [1, 2, 3] tail").unwrap();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn fails_on_no_json() {
        let r: AppResult<Sample> = extract_json("no json here at all");
        assert!(r.is_err());
    }
}

