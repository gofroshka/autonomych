import { useEffect, useState } from "react";
import { CheckCircle2, ChevronDown, ChevronRight, Hash, Loader2, XCircle } from "lucide-react";
import type { HistoryEntry, IterationRow, ProjectRow, TaskRow } from "../types";
import { api } from "../lib/api";
import { Badge } from "./ui/badge";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "./ui/dialog";
import { cn } from "../lib/cn";
import { formatAgo, formatDateTime, formatDuration } from "../lib/format";

export function HistoryDialog({ project, onClose }: { project: ProjectRow; onClose: () => void }) {
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  useEffect(() => {
    let cancelled = false;
    api.getIterationHistory(project.id).then((data) => { if (!cancelled) setEntries(data); });
    return () => { cancelled = true; };
  }, [project.id]);
  const toggle = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-3xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>История «{project.name}»</DialogTitle>
          <DialogDescription>Все итерации проекта от свежей к старой.</DialogDescription>
        </DialogHeader>
        <div className="flex-1 overflow-y-auto scrollbar-thin -mx-2 px-2 min-h-0">
          {entries === null && (
            <div className="flex items-center justify-center py-12 text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          )}
          {entries && entries.length === 0 && (
            <div className="text-center py-10 text-sm text-muted-foreground">Итераций ещё не было.</div>
          )}
          {entries && entries.length > 0 && (
            <div className="space-y-2">
              {entries.map((e) => (
                <IterationCard key={e.iteration.id} entry={e} expanded={expanded.has(e.iteration.id)} onToggle={() => toggle(e.iteration.id)} />
              ))}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function IterationCard({ entry, expanded, onToggle }: { entry: HistoryEntry; expanded: boolean; onToggle: () => void }) {
  const { iteration: it, tasks } = entry;
  const duration = it.ended_at && it.started_at ? formatDuration(it.ended_at - it.started_at) : "—";
  const statusMeta = STATUS_META[it.status] ?? STATUS_META.failed;
  const done = tasks.filter((t) => t.status === "done").length;
  const failed = tasks.filter((t) => t.status === "failed").length;
  return (
    <div className="rounded-lg border border-border bg-card/40">
      <button onClick={onToggle} className="w-full px-4 py-3 flex items-start gap-3 text-left hover:bg-accent/30 transition-colors rounded-lg">
        {expanded ? <ChevronDown className="h-3.5 w-3.5 mt-1 text-muted-foreground shrink-0" /> : <ChevronRight className="h-3.5 w-3.5 mt-1 text-muted-foreground shrink-0" />}
        <div className="flex-1 min-w-0 space-y-1">
          <div className="flex items-center gap-2 flex-wrap">
            <Badge variant="default" className="font-mono">
              <Hash className="h-3 w-3" />
              {it.number}
            </Badge>
            <Badge variant={statusMeta.variant}>{statusMeta.label}</Badge>
            {it.mode === "wrapup" && <Badge variant="warning">stabilization</Badge>}
            <span className="text-sm font-medium truncate flex-1">
              {it.theme || <span className="text-muted-foreground italic">без темы</span>}
            </span>
          </div>
          <div className="flex items-center gap-3 text-[11px] text-muted-foreground font-mono">
            <span>{formatDateTime(it.started_at)}</span>
            <span>·</span>
            <span>{duration}</span>
            {tasks.length > 0 && (
              <>
                <span>·</span>
                <span className="text-success">{done} done</span>
                {failed > 0 && <span className="text-destructive">{failed} failed</span>}
                <span>из {tasks.length}</span>
              </>
            )}
            <span>·</span>
            <span>{formatAgo(it.started_at)}</span>
          </div>
        </div>
      </button>
      {expanded && (
        <div className="px-4 pb-4 pt-1 space-y-3 border-t border-border/60">
          {it.rationale && <Section title="Обоснование"><div className="text-xs text-muted-foreground leading-relaxed">{it.rationale}</div></Section>}
          {it.stories?.length > 0 && (
            <Section title={`Цели (${it.stories.length})`}>
              <ol className="space-y-1.5 text-xs">
                {it.stories.map((s, i) => (
                  <li key={i} className="flex gap-2">
                    <span className="text-muted-foreground/50 font-mono">{i + 1}.</span>
                    <span className="flex-1">
                      <span className="font-medium">{s.title}</span>
                      {s.so_that && <span className="text-muted-foreground"> — {s.so_that}</span>}
                    </span>
                  </li>
                ))}
              </ol>
            </Section>
          )}
          {it.stack_notes && <Section title="Архитектурные заметки"><div className="text-xs text-muted-foreground leading-relaxed whitespace-pre-wrap">{it.stack_notes}</div></Section>}
          {tasks.length > 0 && (
            <Section title={`Задачи (${tasks.length})`}>
              <div className="space-y-1">
                {tasks.map((t) => <TaskRowItem key={t.id} task={t} />)}
              </div>
            </Section>
          )}
          {it.summary && <Section title="Итог ревьюера"><div className="text-xs text-foreground/80 leading-relaxed whitespace-pre-wrap">{it.summary}</div></Section>}
        </div>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="text-[10px] uppercase tracking-wider font-semibold text-muted-foreground mb-1.5">{title}</div>
      {children}
    </div>
  );
}

function TaskRowItem({ task }: { task: TaskRow }) {
  const ms = task.ended_at && task.created_at ? task.ended_at - task.created_at : null;
  return (
    <div className={cn(
      "flex items-center gap-2 text-xs py-1 px-2 rounded transition-colors",
      task.status === "done" && "text-foreground/90",
      task.status === "failed" && "text-destructive/90",
      task.status === "skipped" && "text-muted-foreground/60 line-through"
    )}>
      <TaskStatusIcon status={task.status} />
      <span className="text-[10px] uppercase tracking-wider text-muted-foreground w-14 shrink-0">
        {ROLE_SHORT[task.role] ?? task.role}
      </span>
      <span className="flex-1 truncate">{task.title}</span>
      {ms !== null && task.status === "done" && (
        <span className="font-mono text-[10px] text-muted-foreground/70 shrink-0">{formatDuration(ms)}</span>
      )}
    </div>
  );
}

function TaskStatusIcon({ status }: { status: TaskRow["status"] }) {
  if (status === "done") return <CheckCircle2 className="h-3 w-3 text-success" />;
  if (status === "failed") return <XCircle className="h-3 w-3 text-destructive" />;
  if (status === "in_progress") return <Loader2 className="h-3 w-3 text-primary animate-spin" />;
  return <span className="h-3 w-3 rounded-full bg-muted-foreground/30" />;
}

const ROLE_SHORT: Record<string, string> = {
  specialist_backend: "BACK", specialist_frontend: "FRONT", specialist_devops: "OPS",
  product_owner: "PO", architect: "ARCH", reviewer: "REV",
};

const STATUS_META: Record<IterationRow["status"], { label: string; variant: "default" | "success" | "warning" | "destructive" | "info" }> = {
  running: { label: "идёт", variant: "info" },
  wrapping_up: { label: "стабилизация", variant: "warning" },
  completed: { label: "завершена", variant: "success" },
  presented: { label: "показана", variant: "success" },
  failed: { label: "упала", variant: "destructive" },
};
