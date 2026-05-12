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

use crate::error::{AppError, AppResult};
use crate::types::{AgentRole, PermissionMode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub role: AgentRole,
    pub system_prompt: String,
    pub user_prompt: String,
    pub cwd: PathBuf,
    pub model: String,
    pub tools: Vec<String>,
    pub permission_mode: PermissionMode,
    pub max_turns: u32,
    /// When true, base on the claude_code system preset (file edits etc).
    /// When false, the system prompt is replaced entirely.
    pub claude_code_preset: bool,
    /// Cancellation token. When the conductor signals stop, every running
    /// agent observes the cancellation and kills its `claude` subprocess.
    pub cancel: Option<CancellationToken>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum AgentEvent {
    Start { role: AgentRole },
    AssistantText { role: AgentRole, text: String },
    ToolUse { role: AgentRole, tool: String, input: Value },
    ToolResult { role: AgentRole, content: String, is_error: bool },
    End { role: AgentRole, final_text: String, turns: u32, duration_ms: u64 },
    AgentError { role: AgentRole, message: String },
}

#[derive(Debug)]
pub struct AgentResult {
    pub final_text: String,
    pub turns: u32,
    pub duration_ms: u64,
}

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
        #[serde(default)]
        subtype: Option<String>,
    },
    #[serde(rename = "system")]
    System {
        #[serde(default)]
        subtype: Option<String>,
    },
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

/// Run the agent to completion. Streams events through `on_event`.
pub async fn run_agent<F>(inv: AgentInvocation, mut on_event: F) -> AppResult<AgentResult>
where
    F: FnMut(AgentEvent) + Send,
{
    let started = std::time::Instant::now();
    on_event(AgentEvent::Start { role: inv.role });

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
    let _ = inv.max_turns; // CLI has no explicit max-turns flag; budget via --max-budget-usd if needed.

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

    // Pipe stdout lines into a channel so we can interleave with stderr.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let stdout_reader = BufReader::new(stdout);
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut lines = stdout_reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx_clone.send(line);
        }
    });
    let stderr = child.stderr.take();
    if let Some(err) = stderr {
        let tx2 = tx;
        tokio::spawn(async move {
            let mut r = BufReader::new(err).lines();
            while let Ok(Some(line)) = r.next_line().await {
                tracing::warn!("[claude stderr] {line}");
                // Don't pipe stderr to message channel — it's not JSON.
                let _ = &tx2;
            }
        });
    }

    let mut turns = 0u32;
    let mut final_text = String::new();
    let role = inv.role;

    let cancel = inv.cancel.clone();
    loop {
        let line_opt: Option<String> = if let Some(tok) = &cancel {
            tokio::select! {
                _ = tok.cancelled() => {
                    // Hard-kill the underlying process group so dev-server-style
                    // background children also die. The Child handle still has
                    // kill_on_drop, so the SIGTERM below is the precise hit.
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    on_event(AgentEvent::AgentError { role: inv.role, message: "aborted".into() });
                    return Err(AppError::Agent("aborted by user".into()));
                }
                v = rx.recv() => v
            }
        } else {
            rx.recv().await
        };
        let Some(line) = line_opt else { break };
        if line.trim().is_empty() {
            continue;
        }
        let parsed: Result<SdkMessage, _> = serde_json::from_str(&line);
        match parsed {
            Ok(SdkMessage::Assistant { message, .. }) => {
                turns += 1;
                for block in message.content {
                    match block {
                        ContentBlock::Text { text } => {
                            final_text = text.clone();
                            on_event(AgentEvent::AssistantText { role, text });
                        }
                        ContentBlock::ToolUse { name, input } => {
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
                        on_event(AgentEvent::ToolResult {
                            role,
                            content: text.chars().take(4000).collect(),
                            is_error: is_error.unwrap_or(false),
                        });
                    }
                }
            }
            Ok(SdkMessage::Result { result, is_error, .. }) => {
                if let Some(t) = result {
                    if is_error.unwrap_or(false) {
                        on_event(AgentEvent::AgentError { role, message: t.clone() });
                    } else if !t.is_empty() {
                        final_text = t;
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::trace!("non-json line from claude: {line} ({e})");
            }
        }
    }
    drop(rx);

    let _ = child.wait().await;
    let duration_ms = started.elapsed().as_millis() as u64;
    on_event(AgentEvent::End { role, final_text: final_text.clone(), turns, duration_ms });
    Ok(AgentResult { final_text, turns, duration_ms })
}

/// Strip ```json fences and parse the first top-level JSON value from agent
/// final text.
pub fn extract_json<T: for<'de> Deserialize<'de>>(text: &str) -> AppResult<T> {
    let stripped = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let first = stripped
        .find(|c: char| c == '{' || c == '[')
        .ok_or_else(|| AppError::Agent("no JSON in agent output".into()))?;
    let slice = &stripped[first..];
    // Try progressively shorter suffixes to tolerate trailing prose.
    for end in (1..=slice.len()).rev() {
        if let Ok(v) = serde_json::from_str::<T>(&slice[..end]) {
            return Ok(v);
        }
    }
    Err(AppError::Agent("could not parse JSON from agent output".into()))
}

// Suppress unused-import warnings when stdin gets moved.
#[allow(dead_code)]
fn _type_check_stdin(_: &mut ChildStdin) {}
