// Thin wrappers over Tauri's invoke + event bus. Same surface as the TS
// version's window.api so component code ports verbatim.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ChatMessageRow,
  CreateProjectInput,
  DashboardSnapshot,
  EventRow,
  HistoryEntry,
  ProjectRow,
} from "../types";

export const api = {
  listProjects: () => invoke<ProjectRow[]>("list_projects"),
  createProject: (input: CreateProjectInput) =>
    invoke<ProjectRow>("create_project", { input }),
  deleteProject: (id: string, deleteFiles: boolean) =>
    invoke<void>("delete_project", { id, deleteFiles }),
  renameProject: (id: string, name: string, idea: string) =>
    invoke<void>("rename_project", { id, name, idea }),
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
  resume: (projectId: string, message: string, mode: "soft" | "override") =>
    invoke<void>("resume", { projectId, message, mode }),
  stopPreview: (projectId: string) =>
    invoke<void>("stop_preview", { projectId }),
  retryPreview: (projectId: string) =>
    invoke<void>("retry_preview", { projectId }),
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
