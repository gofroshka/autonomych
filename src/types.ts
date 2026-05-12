// Mirror of src-tauri/src/types.rs. serde naming maps these directly.

export type ConductorState =
  | "IDLE"
  | "RUNNING"
  | "WRAPPING_UP"
  | "PREPARING_PREVIEW"
  | "PRESENTING"
  | "RESUMING"
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
  | "presenter";

export type TaskStatus = "pending" | "in_progress" | "done" | "skipped" | "failed";

export type IterationStatus =
  | "running"
  | "wrapping_up"
  | "presented"
  | "completed"
  | "failed";

export type PermissionMode = "default" | "acceptEdits" | "bypassPermissions";

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
  ended_at: number | null;
  architect_id: string | null;
  depends_on: string[];
}

export type EventType =
  | "agent_start"
  | "agent_message"
  | "agent_tool_use"
  | "agent_tool_result"
  | "agent_end"
  | "agent_error"
  | "state_change"
  | "iteration_start"
  | "iteration_end"
  | "directive"
  | "question_asked"
  | "question_answered"
  | "system";

export interface EventRow {
  id: string;
  project_id: string;
  iteration_id: string | null;
  task_id: string | null;
  agent_role: AgentRole | null;
  type: EventType;
  payload: string;
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
export type QuestionResolution = "user" | "reviewer";

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
  running: boolean;
  pid: number | null;
  url: string | null;
  command: string | null;
  setup_steps: string[];
  notes: string;
  errors: string[];
  logs_tail: string;
  prepared_at: number | null;
  prep_error: string | null;
}

export interface DashboardSnapshot {
  project: ProjectRow | null;
  iteration: IterationRow | null;
  tasks: TaskRow[];
  recent_events: EventRow[];
  pending_steering: SteeringRow | null;
  pending_questions: QuestionRow[];
  preview: PreviewStatus;
}

export interface CreateProjectInput {
  name: string;
  idea: string;
  root_path: string;
  model_pm?: string;
  model_specialist?: string;
  permission_mode?: PermissionMode;
}

export interface HistoryEntry {
  iteration: IterationRow;
  tasks: TaskRow[];
}
