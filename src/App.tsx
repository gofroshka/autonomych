import { useCallback, useEffect, useState } from "react";
import { PanelLeft, PanelRight } from "lucide-react";
import type { ConductorState, DashboardSnapshot, EventRow, ProjectRow } from "./types";
import { api } from "./lib/api";
import { Sidebar } from "./components/Sidebar";
import { Dashboard } from "./components/Dashboard";
import { RightPanel } from "./components/RightPanel";
import { CreateProjectModal } from "./components/CreateProjectModal";
import { PresentingOverlay } from "./components/PresentingOverlay";
import { StateBadge } from "./components/StateBadge";
import { QuestionModal } from "./components/QuestionModal";
import { DeleteProjectModal } from "./components/DeleteProjectModal";
import { RenameProjectModal } from "./components/RenameProjectModal";
import { HistoryDialog } from "./components/HistoryDialog";
import { TooltipProvider } from "./components/ui/tooltip";
import { Button } from "./components/ui/button";

const EVENT_BUFFER_MAX = 600;

export function App() {
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [events, setEvents] = useState<EventRow[]>([]);
  const [creating, setCreating] = useState(false);
  const [deletingProject, setDeletingProject] = useState<ProjectRow | null>(null);
  const [renamingProject, setRenamingProject] = useState<ProjectRow | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [leftCollapsed, setLeftCollapsed] = useState(false);
  const [rightCollapsed, setRightCollapsed] = useState(false);

  const activeProject = snapshot?.project ?? projects.find((p) => p.id === activeId) ?? null;
  const state: ConductorState = (activeProject?.state as ConductorState) ?? "IDLE";

  const refreshProjects = useCallback(async () => {
    const list = await api.listProjects();
    setProjects(list);
    if (!activeId && list.length > 0) setActiveId(list[0].id);
  }, [activeId]);

  const refreshSnapshot = useCallback(async (id: string) => {
    const snap = await api.getSnapshot(id);
    setSnapshot(snap);
    setEvents(snap.recent_events);
  }, []);

  useEffect(() => { refreshProjects(); }, [refreshProjects]);
  useEffect(() => { if (activeId) refreshSnapshot(activeId); }, [activeId, refreshSnapshot]);

  useEffect(() => {
    if (!activeId) return;
    let timer: number | null = null;
    const scheduleRefresh = () => {
      if (timer !== null) return;
      timer = window.setTimeout(() => { timer = null; refreshSnapshot(activeId); }, 250);
    };
    const off = api.onEvent((ev) => {
      if (ev.project_id !== activeId) return;
      setEvents((prev) => {
        if (prev.some((p) => p.id === ev.id)) return prev;
        return [ev, ...prev].slice(0, EVENT_BUFFER_MAX);
      });
      if (["state_change", "iteration_start", "iteration_end", "agent_start", "agent_end", "agent_error", "question_asked", "question_answered"].includes(ev.type)) {
        refreshSnapshot(activeId);
      } else scheduleRefresh();
    });
    return () => { if (timer !== null) clearTimeout(timer); off(); };
  }, [activeId, refreshSnapshot]);

  const handleCreate = async (input: any) => {
    const proj = await api.createProject(input);
    setCreating(false);
    await refreshProjects();
    setActiveId(proj.id);
  };
  const handleStart = async () => { if (activeId) { await api.startConductor(activeId); refreshSnapshot(activeId); } };
  const handleStartPresentation = async () => { if (activeId) { await api.startPresentationOnly(activeId); refreshSnapshot(activeId); } };
  const handleStop = async () => { if (activeId) { await api.stopConductor(activeId); refreshSnapshot(activeId); } };
  const handleWrapUp = async () => { if (activeId) await api.requestWrapUp(activeId); };
  const handleResume = async (msg: string, mode: "soft" | "override") => {
    if (activeId) { await api.resume(activeId, msg, mode); refreshSnapshot(activeId); }
  };

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex flex-col h-screen overflow-hidden bg-background">
        <header className="h-12 shrink-0 flex items-center gap-3 px-3 border-b border-border bg-card/50">
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setLeftCollapsed((v) => !v)} title={leftCollapsed ? "Развернуть" : "Свернуть"}>
            <PanelLeft className="h-3.5 w-3.5" />
          </Button>
          <div className="flex items-center gap-2">
            <div className="h-5 w-5 rounded-md bg-gradient-to-br from-primary to-info" />
            <span className="font-semibold tracking-tight">Автономыч</span>
          </div>
          <StateBadge state={state} />
          <div className="flex-1" />
          {activeProject && (
            <span className="text-[11px] text-muted-foreground font-mono truncate max-w-[420px]">{activeProject.root_path}</span>
          )}
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setRightCollapsed((v) => !v)} title={rightCollapsed ? "Развернуть" : "Свернуть"}>
            <PanelRight className="h-3.5 w-3.5" />
          </Button>
        </header>
        <main className="flex-1 flex min-h-0">
          {!leftCollapsed && (
            <Sidebar
              projects={projects}
              activeId={activeId}
              onSelect={setActiveId}
              onNew={() => setCreating(true)}
              onDelete={setDeletingProject}
              onRename={setRenamingProject}
            />
          )}
          <section className="relative flex-1 flex flex-col min-w-0">
            <Dashboard
              project={activeProject}
              snapshot={snapshot}
              onStart={handleStart}
              onStartPresentation={handleStartPresentation}
              onStop={handleStop}
              onWrapUp={handleWrapUp}
              onEditProject={() => activeProject && setRenamingProject(activeProject)}
              onShowHistory={() => setShowHistory(true)}
            />
            {(state === "PRESENTING" || state === "PREPARING_PREVIEW") && activeProject && (
              <PresentingOverlay project={activeProject} snapshot={snapshot} onResume={handleResume} />
            )}
          </section>
          {!rightCollapsed && <RightPanel events={events} project={activeProject} />}
        </main>
        {creating && <CreateProjectModal onClose={() => setCreating(false)} onCreate={handleCreate} />}
        {deletingProject && (
          <DeleteProjectModal
            project={deletingProject}
            onClose={() => setDeletingProject(null)}
            onConfirm={async (deleteFiles) => {
              const id = deletingProject.id;
              await api.deleteProject(id, deleteFiles);
              setDeletingProject(null);
              if (activeId === id) { setActiveId(null); setSnapshot(null); setEvents([]); }
              await refreshProjects();
            }}
          />
        )}
        {renamingProject && (
          <RenameProjectModal
            project={renamingProject}
            onClose={() => setRenamingProject(null)}
            onSave={async (name, idea) => {
              await api.renameProject(renamingProject.id, name, idea);
              await refreshProjects();
              if (renamingProject.id === activeId) await refreshSnapshot(renamingProject.id);
            }}
          />
        )}
        {showHistory && activeProject && (
          <HistoryDialog project={activeProject} onClose={() => setShowHistory(false)} />
        )}
        {snapshot?.pending_questions && snapshot.pending_questions.length > 0 && (
          <QuestionModal
            key={snapshot.pending_questions[0].id}
            question={snapshot.pending_questions[0]}
            onAnswer={async (answer) => {
              await api.answerQuestion(snapshot.pending_questions[0].id, answer);
              if (activeId) await refreshSnapshot(activeId);
            }}
          />
        )}
      </div>
    </TooltipProvider>
  );
}
