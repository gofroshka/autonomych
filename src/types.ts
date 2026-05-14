// Mirror of src-tauri/src/types.rs + events.rs. serde naming maps these
// directly: snake_case variant tags, snake_case field names.

export type ConductorState =
  | "IDLE"
  | "RUNNING"
  | "WRAPPING_UP"
  | "PREPARING_PREVIEW"
  | "PRESENTING"
  | "RESUMING"
  | "PAUSED"
  | "ERROR";

export type AgentRole =
  | "product_owner"
  | "architect"
  | "specialist_backend"
  | "specialist_frontend"
  | "specialist_devops"
  | "reviewer"
  | "blocker_reviewer"
  | "overseer"
  | "presenter"
  | "merge_resolver"
  | "documenter";

export type TaskStatus = "pending" | "in_progress" | "done" | "skipped" | "failed";

export type IterationStatus =
  | "running"
  | "wrapping_up"
  | "presented"
  | "completed"
  | "failed";

export type PermissionMode = "default" | "acceptEdits" | "bypassPermissions";

/** Which agent CLI to spawn for this project's roles. `claude_code` is
 *  Claude Code; `codex` is OpenAI Codex CLI. Decided at project creation. */
export type AgentBackend = "claude_code" | "codex";

export type IterationMode = "normal" | "wrapup";

export interface ProjectRow {
  id: string;
  name: string;
  idea: string;
  root_path: string;
  state: ConductorState;
  created_at: number;
  model_pm: string;
  model_specialist: string;
  permission_mode: PermissionMode;
  /** Defaults to `claude_code` for pre-existing projects stored before the
   *  Codex backend existed. */
  agent_backend?: AgentBackend;
}

export interface IterationStory {
  title: string;
  as_a?: string;
  i_want?: string;
  so_that?: string;
  acceptance_criteria?: string[];
}

export interface IterationRow {
  id: string;
  project_id: string;
  number: number;
  status: IterationStatus;
  started_at: number;
  ended_at: number | null;
  summary: string | null;
  theme: string | null;
  rationale: string | null;
  stories: IterationStory[];
  stack_notes: string | null;
  mode: IterationMode | null;
}

export interface TaskRow {
  id: string;
  iteration_id: string;
  role: AgentRole;
  title: string;
  description: string;
  status: TaskStatus;
  worktree_path: string | null;
  branch: string | null;
  created_at: number;
  /** Timestamp when the task first transitioned to in_progress. Used by the
   *  UI to drive a live elapsed timer. May be null for old data. */
  started_at: number | null;
  ended_at: number | null;
  architect_id: string | null;
  depends_on: string[];
}

export type QuestionResolution = "user" | "reviewer";

// ---- Event payload — discriminated union mirrored from EventPayload in
// src-tauri/src/events.rs. The `type` tag is set by serde at the Rust side.

export type EventPayload =
  // Agent runtime
  | { type: "agent_start"; role: AgentRole }
  | { type: "agent_message"; role: AgentRole; text: string }
  | { type: "agent_tool_use"; role: AgentRole; tool: string; input: unknown }
  | { type: "agent_tool_result"; role: AgentRole; content: string; is_error: boolean }
  | { type: "agent_end"; role: AgentRole; turns: number; duration_ms: number }
  | { type: "agent_error"; role: AgentRole; message: string }
  // Conductor state machine
  | { type: "state_change"; state: ConductorState }
  // Iteration boundaries
  | { type: "iteration_start"; number: number; mode: IterationMode }
  | { type: "iteration_end"; mode: IterationMode; demoable: boolean | null; summary: string }
  | { type: "iteration_error"; error: string }
  // Iteration stages (diagnostics)
  | {
      type: "resume_iteration";
      number: number;
      po_done: boolean;
      arch_done: boolean;
      tasks_pending: number;
      summary_done: boolean;
    }
  | { type: "po_skipped_resume"; theme: string }
  | { type: "po_done"; theme: string; stories: number }
  | { type: "arch_skipped_resume"; tasks: number }
  | { type: "arch_done"; tasks: number; stack: string }
  | { type: "reviewer_failed"; error: string }
  // Wave runner
  | { type: "wave_started"; size: number }
  | { type: "tasks_skipped"; count: number; reason: string }
  | { type: "graph_deadlock" }
  // Worktree / merge
  | { type: "worktree_failed"; error: string }
  | { type: "merge_failed"; conflict: boolean; message: string }
  | { type: "merge_conflict"; files: string[] }
  | { type: "merge_resolved"; ok: boolean; summary: string }
  | { type: "docs_updated"; summary: string }
  // ask_user routing
  | { type: "ask_user_invoked"; question: string; context: string }
  | {
      type: "question_asked";
      question_id: string;
      question: string;
      context: string;
      reasoning: string | null;
    }
  | {
      type: "question_answered";
      question_id: string;
      resolution: QuestionResolution;
      answer_preview: string;
      reasoning: string | null;
    }
  // User directives
  | { type: "wrap_up_requested" }
  | { type: "presentation_only" }
  | { type: "resume_for_preview"; iteration: number }
  | { type: "resumed" }
  // Preview lifecycle
  | { type: "preview_prep_done" }
  | { type: "preview_prep_failed"; error: string }
  | { type: "preview_shutdown_done" }
  | { type: "preview_shutdown_skipped"; reason: string }
  // Loop / runtime errors
  | { type: "backoff"; duration_ms: number; consecutive: number }
  | { type: "too_many_failures"; consecutive: number }
  | { type: "loop_error"; error: string }
  // Provider rate-limit cooldown
  | { type: "cooldown_started"; retry_at_ms: number; reason: string }
  | { type: "cooldown_ended"; skipped_by_user: boolean }
  | { type: "cooldown_cancelled" };

export type EventType = EventPayload["type"];

export interface EventRow {
  id: string;
  project_id: string;
  iteration_id: string | null;
  task_id: string | null;
  agent_role: AgentRole | null;
  payload: EventPayload;
  ts: number;
}

export type SteeringMode = "soft" | "override";

export interface SteeringRow {
  id: string;
  project_id: string;
  message: string;
  mode: SteeringMode;
  applied_iteration_id: string | null;
  ts: number;
}

export type QuestionStatus = "pending" | "auto_answered" | "answered" | "cancelled";

export interface QuestionRow {
  id: string;
  project_id: string;
  iteration_id: string | null;
  task_id: string | null;
  agent_role: AgentRole | null;
  question: string;
  context: string;
  status: QuestionStatus;
  resolution: QuestionResolution | null;
  answer: string | null;
  created_at: number;
  answered_at: number | null;
}

export type ChatRole = "user" | "assistant";

export interface ChatMessageRow {
  id: string;
  project_id: string;
  role: ChatRole;
  text: string;
  ts: number;
  error?: string | null;
}

export interface PreviewStatus {
  /** Free-form, markdown-friendly text from the Presenter LAUNCH agent.
   *  Tells the user where/how to test the project. The frontend renders it
   *  verbatim, auto-linkifying URLs. `null` means "not prepared yet". */
  instructions: string | null;
  prepared_at: number | null;
  prep_error: string | null;
}

// ===========================================================================
// Backlog — see src-tauri/src/types.rs for the canonical definitions.
// ===========================================================================

export type BacklogStatus = "pending" | "in_iteration" | "done" | "dismissed";

export type BacklogSource =
  | "user_steering"
  | "reviewer_risk"
  | "failed_task"
  | "skipped_task"
  | "presenter_bug"
  | "po_carryover";

export type BacklogPriority = "high" | "normal" | "low";

export type BacklogCategory = "critical" | "bug" | "tech_debt" | "feature" | "wish";

export interface BacklogItem {
  id: string;
  project_id: string;
  title: string;
  details: string;
  source: BacklogSource;
  category: BacklogCategory;
  priority: BacklogPriority;
  status: BacklogStatus;
  created_at: number;
  picked_in_iteration_id?: string;
  origin_iteration_id?: string;
  origin_task_id?: string;
  completed_at?: number;
}

export interface CooldownInfo {
  /** Unix-ms when the conductor plans to resume the paused iteration. */
  retry_at_ms: number;
  /** Truncated original agent error that triggered the cooldown. */
  reason: string;
  /** Iteration this cooldown is attached to. */
  iteration_id?: string;
}

export interface DashboardSnapshot {
  project: ProjectRow | null;
  iteration: IterationRow | null;
  tasks: TaskRow[];
  recent_events: EventRow[];
  pending_steering: SteeringRow | null;
  pending_questions: QuestionRow[];
  preview: PreviewStatus;
  /** Non-null while the conductor is sleeping out a rate-limit cooldown. */
  cooldown?: CooldownInfo | null;
  /** Pending + currently-in-iteration backlog items for the project. */
  backlog?: BacklogItem[];
}

export interface CreateProjectInput {
  name: string;
  idea: string;
  root_path: string;
  model_pm?: string;
  model_specialist?: string;
  permission_mode?: PermissionMode;
  agent_backend?: AgentBackend;
}

/** Reply from the Presenter agent's mid-demo chat triage. `reply` is the
 *  markdown for the user; `draft_steering` (when set) is a pre-filled
 *  steering suggestion when the agent decided the issue is a code bug. */
export interface PresenterChatReply {
  reply: string;
  draft_steering: string | null;
}

export interface HistoryEntry {
  iteration: IterationRow;
  tasks: TaskRow[];
}
