import { useEffect, useState } from "react";
import { ChevronDown, Clock, FolderOpen, Monitor, Pause, Pencil, Play, Square, Sparkles } from "lucide-react";
import type { BacklogCategory, CooldownInfo, DashboardSnapshot, ProjectRow } from "../types";
import { api } from "../lib/api";
import { IterationHeader } from "./IterationHeader";
import { TaskGraph } from "./TaskGraph";
import { Button } from "./ui/button";
import { Textarea } from "./ui/input";
import { cn } from "../lib/cn";
import { CategorySelector } from "./BacklogPanel";

export function Dashboard({
  project, snapshot, onStart, onStartPresentation, onStop, onWrapUp, onEditProject, onSettings, onShowHistory, onBacklogChanged,
}: {
  project: ProjectRow | null;
  snapshot: DashboardSnapshot | null;
  onStart: () => void;
  onStartPresentation: () => void;
  onStop: () => void;
  onWrapUp: () => void;
  onEditProject: () => void;
  onSettings: () => void;
  onShowHistory: () => void;
  onBacklogChanged?: () => void;
}) {
  // "Add to backlog" form is collapsed by default. Acts like the BacklogPanel's
  // add form but lives here so the user can add an item without opening the
  // right panel before pressing Start.
  const [showSteering, setShowSteering] = useState(false);
  const [initialSteering, setInitialSteering] = useState("");
  const [initialCategory, setInitialCategory] = useState<BacklogCategory>("feature");

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
  const state = project.state ?? "IDLE";
  const isRunning = state === "RUNNING" || state === "WRAPPING_UP" || state === "RESUMING" || state === "PREPARING_PREVIEW";
  const isPaused = state === "PAUSED";
  const canWrapUp = state === "RUNNING" || state === "RESUMING";
  const hasIterations = (snapshot?.iteration?.number ?? 0) > 0;
  const pausedIterationNumber = isPaused ? snapshot?.iteration?.number : undefined;
  const cooldown = snapshot?.cooldown ?? null;
  const openFolder = () => api.openExternal(project.root_path);

  const handleStart = async () => {
    const trimmed = initialSteering.trim();
    // If the user typed something in the "add to backlog" form but didn't
    // press the explicit "+ В беклог" button before pressing Start, flush
    // it now so nothing is lost.
    if (trimmed && project) {
      await api.addBacklogItem(project.id, trimmed, { category: initialCategory });
      setInitialSteering("");
      setShowSteering(false);
      onBacklogChanged?.();
    }
    onStart();
  };

  const hint = (() => {
    if (state === "WRAPPING_UP") return "Итерация добегает до конца. Потом ревью, потом подготовка демо.";
    if (state === "PREPARING_PREVIEW") return "Presenter-агент готовит демо: ставит зависимости, мигрирует, запускает dev-сервер.";
    if (isRunning) return "Цикл крутится. «Завершить и показать» — после этой итерации перейдём к демо.";
    if (isPaused && cooldown)
      return `Итерация ${pausedIterationNumber ?? "?"} ждёт восстановления лимита провайдера — продолжим автоматически. Можешь нажать «Продолжить» чтобы попробовать раньше.`;
    if (isPaused) return `Итерация ${pausedIterationNumber ?? "?"} на паузе. Можешь сменить модель / CLI через шестерёнку и нажать «Продолжить итерацию».`;
    if (state === "ERROR") return "Цикл остановился с ошибкой. Посмотри события и нажми «Запустить» чтобы попробовать снова.";
    if (state === "PRESENTING") return "Презентация активна — открой оверлей чтобы посмотреть и ввести правки.";
    return hasIterations
      ? "Можешь продолжить разработку или сразу собрать демо по текущему состоянию кода."
      : "Запустить — PO и Architect соберут backlog, дальше специалисты. Если в папке уже есть запускаемый код (например, тебе передали проект) — «Только посмотреть» поднимет демо без итерации.";
  })();

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <IterationHeader
        project={project}
        iteration={snapshot?.iteration ?? null}
        tasks={snapshot?.tasks ?? []}
        isRunning={isRunning}
        onEditProject={onEditProject}
        onSettings={onSettings}
        onShowHistory={onShowHistory}
      />
      <div className="flex-1 min-h-0 min-w-0">
        <TaskGraph tasks={snapshot?.tasks ?? []} />
      </div>
      {cooldown && <CooldownBanner cooldown={cooldown} />}
      <div className="border-t border-border bg-card/40 px-6 py-3 shrink-0">
        <div className="flex items-center gap-2 flex-wrap">
          {!isRunning ? (
            <>
              <Button onClick={handleStart} className="gap-1.5">
                <Play className="h-3.5 w-3.5" />
                {isPaused
                  ? `Продолжить итерацию ${pausedIterationNumber ?? ""}`.trim()
                  : hasIterations
                    ? "Продолжить работу"
                    : "Запустить"}
              </Button>
              <Button
                variant="outline"
                onClick={onStartPresentation}
                className="gap-1.5"
                title={
                  hasIterations
                    ? "Подготовить и показать демо по текущему состоянию кода"
                    : "Запустить Presenter-агента на текущей папке без итерации (если в проекте уже есть запускаемый код)"
                }
              >
                <Monitor className="h-3.5 w-3.5" />
                {hasIterations ? "Запустить демо" : "Только посмотреть"}
              </Button>
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

        {!isRunning && (
          <div className="mt-3">
            <button
              type="button"
              onClick={() => setShowSteering((v) => !v)}
              className="text-[11px] text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 transition-colors"
              title="Добавить айтем в беклог прямо здесь — PO разберётся с ним когда запустится"
            >
              <ChevronDown
                className={cn("h-3 w-3 transition-transform", showSteering && "rotate-180")}
              />
              <Pencil className="h-3 w-3" />
              {initialSteering.trim()
                ? `Добавить в беклог — ${initialSteering.trim().slice(0, 50)}${initialSteering.trim().length > 50 ? "…" : ""}`
                : "Добавить в беклог (необязательно)"}
            </button>
            {showSteering && (
              <div className="mt-2 space-y-2 border-l-2 border-border pl-3 ml-1">
                <Textarea
                  rows={2}
                  value={initialSteering}
                  onChange={(e) => setInitialSteering(e.target.value)}
                  placeholder="Например: «Используй TypeScript и Vite» или «Сфокусируйся на бэкенде»"
                  className="resize-none text-[12px]"
                />
                <CategorySelector value={initialCategory} onChange={setInitialCategory} size="sm" />
                <div className="flex items-center justify-between">
                  <p className="text-[10px] text-muted-foreground/80 leading-relaxed">
                    Добавится в беклог. PO учтёт при запуске.
                  </p>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={async () => {
                      const t = initialSteering.trim();
                      if (!t || !project) return;
                      await api.addBacklogItem(project.id, t, { category: initialCategory });
                      setInitialSteering("");
                      setShowSteering(false);
                      onBacklogChanged?.();
                    }}
                    disabled={!initialSteering.trim()}
                    className="gap-1"
                  >
                    + В беклог
                  </Button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

/** Live countdown shown while the conductor is waiting out a provider
 *  rate-limit cooldown. Recomputes every second; when retry_at_ms is in
 *  the past, we just say "вот-вот продолжим" since the backend should
 *  flip the iteration back to RUNNING any moment now. */
function CooldownBanner({ cooldown }: { cooldown: CooldownInfo }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const remaining = Math.max(0, cooldown.retry_at_ms - now);
  const mm = Math.floor(remaining / 60_000);
  const ss = Math.floor((remaining % 60_000) / 1000);
  const countdown = remaining > 0
    ? `${mm.toString().padStart(2, "0")}:${ss.toString().padStart(2, "0")}`
    : "вот-вот";
  const ts = new Date(cooldown.retry_at_ms);
  const hh = ts.getHours().toString().padStart(2, "0");
  const mmAbs = ts.getMinutes().toString().padStart(2, "0");
  return (
    <div className="border-t border-warning/30 bg-warning/5 px-6 py-2.5 shrink-0">
      <div className="flex items-start gap-3">
        <Clock className="h-4 w-4 mt-0.5 text-warning shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium text-foreground">
            Лимит провайдера — продолжим автоматически через{" "}
            <span className="font-mono">{countdown}</span>
            <span className="text-muted-foreground"> (≈ {hh}:{mmAbs})</span>
          </div>
          <div className="text-[11px] text-muted-foreground line-clamp-2 mt-0.5">
            {cooldown.reason}
          </div>
        </div>
      </div>
    </div>
  );
}
