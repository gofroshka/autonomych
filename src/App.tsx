// Top-level shell: header + sidebar + main content + modals. All state
// lives in `useApp()` — this component is a layout / wiring concern only.

import { useState } from "react";
import { PanelLeft, PanelRight } from "lucide-react";
import type { ProjectRow } from "./types";
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
import { useApp } from "./state/useApp";

const TOOLTIP_DELAY_MS = 300;

export function App() {
  const app = useApp();
  const { activeProject, state, snapshot, events, projects, activeId } = app;

  const [creating, setCreating] = useState(false);
  const [deletingProject, setDeletingProject] = useState<ProjectRow | null>(null);
  const [renamingProject, setRenamingProject] = useState<ProjectRow | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [leftCollapsed, setLeftCollapsed] = useState(false);
  const [rightCollapsed, setRightCollapsed] = useState(false);

  const showPresenting = state === "PRESENTING" || state === "PREPARING_PREVIEW";
  const firstQuestion = snapshot?.pending_questions?.[0] ?? null;

  return (
    <TooltipProvider delayDuration={TOOLTIP_DELAY_MS}>
      <div className="flex flex-col h-screen overflow-hidden bg-background">
        <header className="h-12 shrink-0 flex items-center gap-3 px-3 border-b border-border bg-card/50">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setLeftCollapsed((v) => !v)}
            title={leftCollapsed ? "Развернуть" : "Свернуть"}
          >
            <PanelLeft className="h-3.5 w-3.5" />
          </Button>
          <div className="flex items-center gap-2">
            <div className="h-5 w-5 rounded-md bg-gradient-to-br from-primary to-info" />
            <span className="font-semibold tracking-tight">Автономыч</span>
          </div>
          <StateBadge state={state} />
          <div className="flex-1" />
          {activeProject && (
            <span className="text-[11px] text-muted-foreground font-mono truncate max-w-[420px]">
              {activeProject.root_path}
            </span>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setRightCollapsed((v) => !v)}
            title={rightCollapsed ? "Развернуть" : "Свернуть"}
          >
            <PanelRight className="h-3.5 w-3.5" />
          </Button>
        </header>

        <main className="flex-1 flex min-h-0">
          {!leftCollapsed && (
            <Sidebar
              projects={projects}
              activeId={activeId}
              onSelect={app.setActiveId}
              onNew={() => setCreating(true)}
              onDelete={setDeletingProject}
              onRename={setRenamingProject}
            />
          )}

          <section className="relative flex-1 flex flex-col min-w-0">
            <Dashboard
              project={activeProject}
              snapshot={snapshot}
              onStart={app.start}
              onStartPresentation={app.startPresentation}
              onStop={app.stop}
              onWrapUp={app.wrapUp}
              onEditProject={() => activeProject && setRenamingProject(activeProject)}
              onShowHistory={() => setShowHistory(true)}
            />
            {showPresenting && activeProject && (
              <PresentingOverlay
                project={activeProject}
                snapshot={snapshot}
                onResume={app.resume}
              />
            )}
          </section>

          {!rightCollapsed && <RightPanel events={events} project={activeProject} />}
        </main>

        {creating && (
          <CreateProjectModal
            onClose={() => setCreating(false)}
            onCreate={async (input) => {
              await app.createProject(input);
              setCreating(false);
            }}
          />
        )}

        {deletingProject && (
          <DeleteProjectModal
            project={deletingProject}
            onClose={() => setDeletingProject(null)}
            onConfirm={async (deleteFiles) => {
              await app.deleteProject(deletingProject.id, deleteFiles);
              setDeletingProject(null);
            }}
          />
        )}

        {renamingProject && (
          <RenameProjectModal
            project={renamingProject}
            onClose={() => setRenamingProject(null)}
            onSave={async (name, idea) => {
              await app.renameProject(renamingProject.id, name, idea);
            }}
          />
        )}

        {showHistory && activeProject && (
          <HistoryDialog project={activeProject} onClose={() => setShowHistory(false)} />
        )}

        {firstQuestion && (
          <QuestionModal
            key={firstQuestion.id}
            question={firstQuestion}
            onAnswer={(answer) => app.answerQuestion(firstQuestion.id, answer)}
          />
        )}
      </div>
    </TooltipProvider>
  );
}
