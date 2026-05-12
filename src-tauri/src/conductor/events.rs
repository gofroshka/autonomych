//! Event emission — store-and-publish pipeline used everywhere in the
//! conductor.
//!
//! Sync from end to end. The store append is a single buffered `writeln!`
//! and the publish is a sync `app.emit` (queued by Tauri internally), so we
//! don't need to spawn a task per event. That removes a couple thousand
//! `tokio::spawn` allocations from chatty agent sessions.

use super::Conductor;
use crate::agents::AgentEvent;
use crate::events::{EventBus, EventPayload};
use crate::store::Store;
use std::sync::Arc;

const AGENT_MESSAGE_LIMIT: usize = 2000;
const TOOL_RESULT_LIMIT: usize = 4000;

impl Conductor {
    /// Persist + publish a top-level event (no iteration/task context).
    pub(super) fn emit(&self, payload: EventPayload) {
        self.emit_for(payload, None, None);
    }

    /// Persist + publish an event scoped to a specific iteration/task.
    pub(super) fn emit_for(
        &self,
        payload: EventPayload,
        iteration_id: Option<String>,
        task_id: Option<String>,
    ) {
        match self
            .store
            .insert_event(&self.project_id, payload, iteration_id, task_id)
        {
            Ok(row) => self.bus.publish(&row),
            Err(e) => tracing::warn!(error = %e, "event insert failed"),
        }
    }

    /// Detachable, `'static + Send + Clone` publisher for use in agent
    /// callbacks. Owns its own clones of the dependencies so it doesn't tie
    /// the callback to the Conductor's lifetime.
    pub(super) fn event_publisher(&self) -> EventPublisher {
        EventPublisher {
            project_id: self.project_id.clone(),
            store: self.store.clone(),
            bus: self.bus.clone(),
        }
    }
}

/// Owns just enough state to insert + publish events. Constructed via
/// [`Conductor::event_publisher`]; cheap to clone (`Arc`s under the hood).
#[derive(Clone)]
pub(super) struct EventPublisher {
    project_id: String,
    store: Arc<Store>,
    bus: Arc<dyn EventBus>,
}

impl EventPublisher {
    fn emit(&self, payload: EventPayload, iteration_id: Option<String>, task_id: Option<String>) {
        match self
            .store
            .insert_event(&self.project_id, payload, iteration_id, task_id)
        {
            Ok(row) => self.bus.publish(&row),
            Err(e) => tracing::warn!(error = %e, "event insert failed"),
        }
    }

    /// Convert an agent-runtime event into a typed payload and emit it.
    pub fn publish_agent_event(
        &self,
        ev: AgentEvent,
        iteration_id: Option<String>,
        task_id: Option<String>,
    ) {
        let payload = match ev {
            AgentEvent::Start { role } => EventPayload::AgentStart { role },
            AgentEvent::AssistantText { role, text } => EventPayload::AgentMessage {
                role,
                text: truncate_chars(&text, AGENT_MESSAGE_LIMIT),
            },
            AgentEvent::ToolUse { role, tool, input } => {
                EventPayload::AgentToolUse { role, tool, input }
            }
            AgentEvent::ToolResult {
                role,
                content,
                is_error,
            } => EventPayload::AgentToolResult {
                role,
                content: truncate_chars(&content, TOOL_RESULT_LIMIT),
                is_error,
            },
            AgentEvent::End {
                role,
                turns,
                duration_ms,
                ..
            } => EventPayload::AgentEnd {
                role,
                turns,
                duration_ms,
            },
            AgentEvent::AgentError { role, message } => {
                EventPayload::AgentError { role, message }
            }
        };
        self.emit(payload, iteration_id, task_id);
    }
}

/// Truncate a string to at most `max` characters (not bytes). Avoids the
/// classic UTF-8 boundary panic when slicing.
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
