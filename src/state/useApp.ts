// Top-level app state hook. Encapsulates:
//   - project list + selected project
//   - dashboard snapshot + live event feed for the active project
//   - subscriptions to the backend "event" stream
//   - high-level actions (start/stop/wrap-up/resume/...) that refresh state
//
// Components consume this through `useApp()` so App.tsx stays a layout
// component.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type UpdateProjectSettingsInput } from "../lib/api";
import { isStructural } from "../lib/events";
import type {
  ConductorState,
  CreateProjectInput,
  DashboardSnapshot,
  EventRow,
  ProjectRow,
} from "../types";

/** Max events we keep in memory before dropping oldest. */
const EVENT_BUFFER_MAX = 600;

/** Debounce for non-structural event-driven refreshes. */
const DEBOUNCE_REFRESH_MS = 250;

export interface AppState {
  projects: ProjectRow[];
  activeId: string | null;
  setActiveId: (id: string | null) => void;
  snapshot: DashboardSnapshot | null;
  events: EventRow[];
  state: ConductorState;
  activeProject: ProjectRow | null;

  refreshProjects: () => Promise<void>;
  refreshSnapshot: (id: string) => Promise<void>;

  // Lifecycle
  createProject: (input: CreateProjectInput) => Promise<ProjectRow>;
  deleteProject: (id: string, deleteFiles: boolean) => Promise<void>;
  renameProject: (id: string, name: string, idea: string) => Promise<void>;
  updateProjectSettings: (id: string, s: UpdateProjectSettingsInput) => Promise<void>;

  // Conductor actions (no-op when no active project)
  start: () => Promise<void>;
  startPresentation: () => Promise<void>;
  stop: () => Promise<void>;
  wrapUp: () => Promise<void>;
  resume: () => Promise<void>;
  answerQuestion: (questionId: string, answer: string) => Promise<void>;
}

export function useApp(): AppState {
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [events, setEvents] = useState<EventRow[]>([]);

  const activeProject =
    snapshot?.project ?? projects.find((p) => p.id === activeId) ?? null;
  const state: ConductorState = activeProject?.state ?? "IDLE";

  const refreshProjects = useCallback(async () => {
    const list = await api.listProjects();
    setProjects(list);
    setActiveId((current) => {
      if (current && list.some((p) => p.id === current)) return current;
      return list[0]?.id ?? null;
    });
  }, []);

  const refreshSnapshot = useCallback(async (id: string) => {
    const snap = await api.getSnapshot(id);
    setSnapshot(snap);
    setEvents(snap.recent_events);
  }, []);

  useEffect(() => {
    refreshProjects();
  }, [refreshProjects]);

  useEffect(() => {
    if (activeId) refreshSnapshot(activeId);
  }, [activeId, refreshSnapshot]);

  // Live event stream. Debounce non-structural refreshes so we don't
  // hammer the backend on every assistant_text fragment.
  const debounceRef = useRef<number | null>(null);
  useEffect(() => {
    if (!activeId) return;
    const scheduleRefresh = () => {
      if (debounceRef.current !== null) return;
      debounceRef.current = window.setTimeout(() => {
        debounceRef.current = null;
        refreshSnapshot(activeId);
      }, DEBOUNCE_REFRESH_MS);
    };
    const off = api.onEvent((ev) => {
      if (ev.project_id !== activeId) return;
      setEvents((prev) => {
        if (prev.some((p) => p.id === ev.id)) return prev;
        return [ev, ...prev].slice(0, EVENT_BUFFER_MAX);
      });
      if (isStructural(ev.payload)) refreshSnapshot(activeId);
      else scheduleRefresh();
    });
    return () => {
      if (debounceRef.current !== null) {
        clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      off();
    };
  }, [activeId, refreshSnapshot]);

  // ---- Project lifecycle ----
  const createProject = useCallback(
    async (input: CreateProjectInput) => {
      const proj = await api.createProject(input);
      await refreshProjects();
      setActiveId(proj.id);
      return proj;
    },
    [refreshProjects]
  );

  const deleteProject = useCallback(
    async (id: string, deleteFiles: boolean) => {
      await api.deleteProject(id, deleteFiles);
      if (activeId === id) {
        setActiveId(null);
        setSnapshot(null);
        setEvents([]);
      }
      await refreshProjects();
    },
    [activeId, refreshProjects]
  );

  const renameProject = useCallback(
    async (id: string, name: string, idea: string) => {
      await api.renameProject(id, name, idea);
      await refreshProjects();
      if (id === activeId) await refreshSnapshot(id);
    },
    [activeId, refreshProjects, refreshSnapshot]
  );

  const updateProjectSettings = useCallback(
    async (id: string, s: UpdateProjectSettingsInput) => {
      await api.updateProjectSettings(id, s);
      await refreshProjects();
      if (id === activeId) await refreshSnapshot(id);
    },
    [activeId, refreshProjects, refreshSnapshot]
  );

  // ---- Conductor actions ----
  const withActive = useCallback(
    async (fn: (id: string) => Promise<void>) => {
      if (!activeId) return;
      await fn(activeId);
      await refreshSnapshot(activeId);
    },
    [activeId, refreshSnapshot]
  );

  /**
   * Begin (or resume) the cycle on the active project. User-side feedback
   * goes through the backlog now, not through a pre-Start steering arg —
   * see Dashboard's `+ В беклог` form, BacklogPanel, and PresentingOverlay.
   */
  const start = useCallback(async () => {
    if (!activeId) return;
    await api.startConductor(activeId);
    await refreshSnapshot(activeId);
  }, [activeId, refreshSnapshot]);
  const startPresentation = useCallback(
    () => withActive(api.startPresentationOnly),
    [withActive]
  );
  const stop = useCallback(() => withActive(api.stopConductor), [withActive]);
  const wrapUp = useCallback(async () => {
    if (activeId) await api.requestWrapUp(activeId);
  }, [activeId]);
  const resume = useCallback(async () => {
    if (!activeId) return;
    await api.resume(activeId);
    await refreshSnapshot(activeId);
  }, [activeId, refreshSnapshot]);
  const answerQuestion = useCallback(
    async (questionId: string, answer: string) => {
      await api.answerQuestion(questionId, answer);
      if (activeId) await refreshSnapshot(activeId);
    },
    [activeId, refreshSnapshot]
  );

  return {
    projects,
    activeId,
    setActiveId,
    snapshot,
    events,
    state,
    activeProject,
    refreshProjects,
    refreshSnapshot,
    createProject,
    deleteProject,
    renameProject,
    updateProjectSettings,
    start,
    startPresentation,
    stop,
    wrapUp,
    resume,
    answerQuestion,
  };
}
