import { FolderOpen, Monitor, Pause, Play, Square, Sparkles } from "lucide-react";
import type { ConductorState, DashboardSnapshot, ProjectRow } from "../types";
import { api } from "../lib/api";
import { IterationHeader } from "./IterationHeader";
import { TaskGraph } from "./TaskGraph";
import { Button } from "./ui/button";

export function Dashboard({
  project, snapshot, onStart, onStartPresentation, onStop, onWrapUp, onEditProject, onShowHistory,
}: {
  project: ProjectRow | null;
  snapshot: DashboardSnapshot | null;
  onStart: () => void;
  onStartPresentation: () => void;
  onStop: () => void;
  onWrapUp: () => void;
  onEditProject: () => void;
  onShowHistory: () => void;
}) {
  if (!project) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center text-center px-8 py-20">
        <Sparkles className="h-10 w-10 text-muted-foreground/30 mb-5" />
        <h2 className="text-xl font-semibold mb-2">Выбери или создай проект</h2>
        <p className="text-sm text-muted-foreground max-w-md">
          Дай идею — Автономыч соберёт команду из ИИ-агентов и начнёт строить.
        </p>
      </div>
    );
  }
  const state = (project.state ?? "IDLE") as ConductorState;
  const isRunning = state === "RUNNING" || state === "WRAPPING_UP" || state === "RESUMING" || state === "PREPARING_PREVIEW";
  const canWrapUp = state === "RUNNING" || state === "RESUMING";
  const hasIterations = (snapshot?.iteration?.number ?? 0) > 0;
  const openFolder = () => api.openExternal(project.root_path);

  const hint = (() => {
    if (state === "WRAPPING_UP") return "Итерация добегает до конца. Потом ревью, потом подготовка демо.";
    if (state === "PREPARING_PREVIEW") return "Presenter-агент готовит демо: ставит зависимости, мигрирует, запускает dev-сервер.";
    if (isRunning) return "Цикл крутится. «Завершить и показать» — после этой итерации перейдём к демо.";
    if (state === "ERROR") return "Цикл остановился с ошибкой. Посмотри события и нажми «Запустить» чтобы попробовать снова.";
    if (state === "PRESENTING") return "Презентация активна — открой оверлей чтобы посмотреть и ввести правки.";
    return hasIterations
      ? "Можешь продолжить разработку или сразу собрать демо по текущему состоянию кода."
      : "PO и Architect соберут backlog, потом начнут специалисты.";
  })();

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <IterationHeader
        project={project}
        iteration={snapshot?.iteration ?? null}
        tasks={snapshot?.tasks ?? []}
        isRunning={isRunning}
        onEditProject={onEditProject}
        onShowHistory={onShowHistory}
      />
      <div className="flex-1 min-h-0 min-w-0">
        <TaskGraph tasks={snapshot?.tasks ?? []} />
      </div>
      <div className="border-t border-border bg-card/40 px-6 py-3 shrink-0">
        <div className="flex items-center gap-2 flex-wrap">
          {!isRunning ? (
            <>
              <Button onClick={onStart} className="gap-1.5">
                <Play className="h-3.5 w-3.5" />
                {hasIterations ? "Продолжить работу" : "Запустить"}
              </Button>
              {hasIterations && (
                <Button variant="outline" onClick={onStartPresentation} className="gap-1.5" title="Подготовить и показать демо по текущему состоянию кода">
                  <Monitor className="h-3.5 w-3.5" />
                  Запустить демо
                </Button>
              )}
            </>
          ) : (
            <Button onClick={onWrapUp} disabled={!canWrapUp} className="gap-1.5">
              <Pause className="h-3.5 w-3.5" />
              Завершить и показать
            </Button>
          )}
          <Button variant="outline" onClick={openFolder} className="gap-1.5">
            <FolderOpen className="h-3.5 w-3.5" />
            Папка
          </Button>
          <div className="flex-1" />
          {isRunning && (
            <Button variant="destructive" onClick={onStop} className="gap-1.5">
              <Square className="h-3.5 w-3.5 fill-current" />
              Стоп
            </Button>
          )}
        </div>
        <p className="text-[11px] text-muted-foreground mt-2 leading-relaxed">{hint}</p>
      </div>
    </div>
  );
}
