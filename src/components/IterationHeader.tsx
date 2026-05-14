import { useMemo, useState } from "react";
import { ChevronDown, ChevronUp, Clock, Hash, History, Lock, Pencil, Settings, Target } from "lucide-react";
import type { IterationRow, ProjectRow, TaskRow } from "../types";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { cn } from "../lib/cn";
import { formatDuration } from "../lib/format";

export function IterationHeader({
  project, iteration, tasks, isRunning, onEditProject, onSettings, onShowHistory,
}: {
  project: ProjectRow;
  iteration: IterationRow | null;
  tasks: TaskRow[];
  isRunning: boolean;
  onEditProject: () => void;
  onSettings: () => void;
  onShowHistory: () => void;
}) {
  const [ideaOpen, setIdeaOpen] = useState(false);
  const [storiesOpen, setStoriesOpen] = useState(true);

  const stats = useMemo(() => ({
    total: tasks.length,
    done: tasks.filter((t) => t.status === "done").length,
    failed: tasks.filter((t) => t.status === "failed").length,
    running: tasks.filter((t) => t.status === "in_progress").length,
  }), [tasks]);

  const duration = useMemo(() => {
    if (!iteration) return null;
    const end = iteration.ended_at ?? Date.now();
    return formatDuration(end - iteration.started_at);
  }, [iteration]);

  return (
    <div className="border-b border-border bg-card/40">
      <div className="flex items-start gap-3 px-6 pt-4 pb-2">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-0.5">
            <h1 className="text-base font-semibold truncate">{project.name}</h1>
            <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onEditProject} disabled={isRunning} title={isRunning ? "Останови цикл, чтобы редактировать" : "Изменить идею"}>
              {isRunning ? <Lock className="h-3 w-3" /> : <Pencil className="h-3 w-3" />}
            </Button>
            <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onSettings} disabled={isRunning} title={isRunning ? "Останови цикл, чтобы менять CLI и модели" : "CLI и модели"}>
              <Settings className="h-3 w-3" />
            </Button>
            <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onShowHistory} title="История проекта">
              <History className="h-3 w-3" />
            </Button>
          </div>
          <button onClick={() => setIdeaOpen((v) => !v)} className="text-xs text-muted-foreground hover:text-foreground transition-colors text-left flex items-start gap-1 group max-w-full">
            <span className={cn("block transition-all", ideaOpen ? "" : "line-clamp-1")}>{project.idea}</span>
            {ideaOpen ? <ChevronUp className="h-3 w-3 mt-0.5 shrink-0 opacity-50 group-hover:opacity-100" /> : <ChevronDown className="h-3 w-3 mt-0.5 shrink-0 opacity-50 group-hover:opacity-100" />}
          </button>
        </div>
      </div>
      {iteration && (
        <div className="px-6 pb-3 space-y-2">
          <div className="flex items-center gap-3 flex-wrap">
            <Badge variant="primary" className="font-mono">
              <Hash className="h-3 w-3" />
              ИТЕРАЦИЯ {iteration.number}
            </Badge>
            <span className="text-sm font-medium text-foreground/90 flex-1 min-w-0 truncate">
              {iteration.theme || iteration.summary?.split("\n")[0]?.replace(/^[✓✗]\s*/, "") || (
                <span className="text-muted-foreground italic">тема ещё не определена</span>
              )}
            </span>
            <div className="flex items-center gap-3 text-[11px] text-muted-foreground font-mono">
              <span className="flex items-center gap-1"><Clock className="h-3 w-3" />{duration ?? "—"}</span>
              {stats.total > 0 && (
                <>
                  <span className="text-success">{stats.done} done</span>
                  {stats.running > 0 && <span className="text-primary">{stats.running} active</span>}
                  {stats.failed > 0 && <span className="text-destructive">{stats.failed} failed</span>}
                  <span className="text-muted-foreground/60">из {stats.total}</span>
                </>
              )}
            </div>
          </div>
          {iteration.rationale && (
            <div className="text-[12px] text-muted-foreground/90 leading-relaxed border-l-2 border-border pl-3">{iteration.rationale}</div>
          )}
          {iteration.stories && iteration.stories.length > 0 && (
            <div>
              <button onClick={() => setStoriesOpen((v) => !v)} className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-semibold text-muted-foreground hover:text-foreground transition-colors">
                <Target className="h-3 w-3" />
                Цели итерации ({iteration.stories.length})
                {storiesOpen ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
              </button>
              {storiesOpen && (
                <ol className="mt-1.5 space-y-1 text-[12px] pl-1">
                  {iteration.stories.map((s, i) => (
                    <li key={i} className="flex gap-2">
                      <span className="text-muted-foreground/50 font-mono">{i + 1}.</span>
                      <span className="flex-1">
                        <span className="text-foreground/90 font-medium">{s.title}</span>
                        {s.so_that && <span className="text-muted-foreground"> — {s.so_that}</span>}
                      </span>
                    </li>
                  ))}
                </ol>
              )}
            </div>
          )}
          {iteration.stack_notes && (
            <details className="text-[11px] text-muted-foreground">
              <summary className="cursor-pointer hover:text-foreground transition-colors">Архитектурные заметки</summary>
              <div className="mt-1.5 pl-2 border-l border-border whitespace-pre-wrap leading-relaxed">{iteration.stack_notes}</div>
            </details>
          )}
        </div>
      )}
    </div>
  );
}
