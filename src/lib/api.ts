// Thin wrappers over Tauri's invoke + event bus. Same surface as the TS
// version's window.api so component code ports verbatim.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentBackend,
  BacklogCategory,
  BacklogItem,
  BacklogPriority,
  ChatMessageRow,
  CreateProjectInput,
  DashboardSnapshot,
  EventRow,
  HistoryEntry,
  PermissionMode,
  PresenterChatReply,
  ProjectRow,
} from "../types";

export interface UpdateProjectSettingsInput {
  modelPm: string;
  modelSpecialist: string;
  permissionMode: PermissionMode;
  agentBackend: AgentBackend;
}

export const api = {
  listProjects: () => invoke<ProjectRow[]>("list_projects"),
  createProject: (input: CreateProjectInput) =>
    invoke<ProjectRow>("create_project", { input }),
  deleteProject: (id: string, deleteFiles: boolean) =>
    invoke<void>("delete_project", { id, deleteFiles }),
  renameProject: (id: string, name: string, idea: string) =>
    invoke<void>("rename_project", { id, name, idea }),
  updateProjectSettings: (id: string, s: UpdateProjectSettingsInput) =>
    invoke<ProjectRow>("update_project_settings", {
      id,
      modelPm: s.modelPm,
      modelSpecialist: s.modelSpecialist,
      permissionMode: s.permissionMode,
      agentBackend: s.agentBackend,
    }),
  openProject: (id: string) => invoke<ProjectRow | null>("open_project", { id }),
  getSnapshot: (projectId: string) =>
    invoke<DashboardSnapshot>("get_snapshot", { projectId }),
  getEvents: (projectId: string, sinceTs?: number) =>
    invoke<EventRow[]>("get_events", { projectId, sinceTs }),
  startConductor: (projectId: string) =>
    invoke<void>("start_conductor", { projectId }),
  startPresentationOnly: (projectId: string) =>
    invoke<void>("start_presentation_only", { projectId }),
  stopConductor: (projectId: string) =>
    invoke<void>("stop_conductor", { projectId }),
  requestWrapUp: (projectId: string) =>
    invoke<void>("request_wrap_up", { projectId }),
  /** Queue steering for the next iteration without waking a parked
   *  conductor. Use this from Idle/Error states — `resume` is only for
   *  the Presenting state. */
  pushSteering: (projectId: string, message: string, mode: "soft" | "override") =>
    invoke<void>("push_steering", { projectId, message, mode }),
  resume: (projectId: string, message: string, mode: "soft" | "override") =>
    invoke<void>("resume", { projectId, message, mode }),
  stopPreview: (projectId: string) =>
    invoke<void>("stop_preview", { projectId }),
  retryPreview: (projectId: string) =>
    invoke<void>("retry_preview", { projectId }),
  /** Mid-demo chat with the Presenter — report an issue, get a triage
   *  reply and optionally a pre-filled steering suggestion. */
  presenterChat: (projectId: string, text: string) =>
    invoke<PresenterChatReply>("presenter_chat", { projectId, text }),
  answerQuestion: (questionId: string, answer: string) =>
    invoke<void>("answer_question", { questionId, answer }),
  getChatHistory: (projectId: string) =>
    invoke<ChatMessageRow[]>("get_chat_history", { projectId }),
  sendChatMessage: (projectId: string, text: string) =>
    invoke<ChatMessageRow>("send_chat_message", { projectId, text }),
  getIterationHistory: (projectId: string) =>
    invoke<HistoryEntry[]>("get_iteration_history", { projectId }),
  pickDirectory: () => invoke<string | null>("pick_directory"),
  openExternal: (path: string) => invoke<void>("open_external", { path }),

  // ---- Backlog ----
  listBacklog: (projectId: string) =>
    invoke<BacklogItem[]>("list_backlog", { projectId }),
  addBacklogItem: (
    projectId: string,
    title: string,
    opts?: { details?: string; category?: BacklogCategory; priority?: BacklogPriority }
  ) =>
    invoke<BacklogItem>("add_backlog_item", {
      projectId,
      title,
      details: opts?.details ?? null,
      category: opts?.category ?? null,
      priority: opts?.priority ?? null,
    }),
  updateBacklogItem: (
    id: string,
    patch: { title?: string; details?: string; priority?: BacklogPriority }
  ) =>
    invoke<void>("update_backlog_item", {
      id,
      title: patch.title ?? null,
      details: patch.details ?? null,
      priority: patch.priority ?? null,
    }),
  dismissBacklogItem: (id: string) =>
    invoke<void>("dismiss_backlog_item", { id }),

  /** Subscribe to backend "event" stream. Returns an unsubscribe function. */
  onEvent(cb: (e: EventRow) => void): () => void {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen<EventRow>("event", (e) => cb(e.payload)).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  },
};
