// Backlog panel — manages the project's parking-lot of "what to do next".
//
// What's in this list:
//   - User-typed items (this UI + the steering compose form in PresentingOverlay)
//   - Failed/skipped tasks from past iterations (auto-added by the conductor)
//   - Reviewer-flagged risks (auto-added by the Reviewer's JSON output)
//   - Presenter mid-demo bug reports (auto-added when the user confirms)
//   - PO-parked ideas (`add_to_backlog` in PO output)
//
// PO of the next iteration receives this list (filtered to active items)
// and picks 1-3 to address. The "Активные" tab shows what's open right now;
// "Архив" shows done/dismissed for context.

import { useEffect, useMemo, useState } from "react";
import { Plus, X, ChevronDown, ChevronRight, Trash2 } from "lucide-react";
import type {
  BacklogCategory,
  BacklogItem,
  BacklogPriority,
  BacklogSource,
  ProjectRow,
} from "../types";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Input, Label, Textarea } from "./ui/input";
import { cn } from "../lib/cn";
import { CATEGORY_META, CATEGORY_ORDER } from "../lib/backlog";

const SOURCE_LABEL: Record<BacklogSource, { label: string; color: string }> = {
  user_steering: { label: "User", color: "bg-primary/10 text-primary border-primary/30" },
  reviewer_risk: { label: "Reviewer", color: "bg-warning/10 text-warning border-warning/30" },
  failed_task: { label: "Failed task", color: "bg-destructive/10 text-destructive border-destructive/30" },
  skipped_task: { label: "Skipped task", color: "bg-muted text-muted-foreground border-border" },
  presenter_bug: { label: "Demo bug", color: "bg-info/10 text-info border-info/30" },
  po_carryover: { label: "PO", color: "bg-success/10 text-success border-success/30" },
};

const PRIORITY_LABEL: Record<BacklogPriority, { label: string; rank: number; color: string }> = {
  high: { label: "high", rank: 0, color: "text-destructive" },
  normal: { label: "normal", rank: 1, color: "text-muted-foreground" },
  low: { label: "low", rank: 2, color: "text-muted-foreground/60" },
};

export function BacklogPanel({
  project,
  activeBacklog,
  onChanged,
}: {
  project: ProjectRow | null;
  /** Active items from DashboardSnapshot — used as the default view so we
   *  don't fetch the full archive on every render. The user can flip to
   *  archive to pull it via `list_backlog`. */
  activeBacklog: BacklogItem[];
  /** Notify parent that data may have changed (so it re-fetches snapshot). */
  onChanged?: () => void;
}) {
  const [tab, setTab] = useState<"active" | "archive">("active");
  const [archive, setArchive] = useState<BacklogItem[]>([]);
  const [archiveLoaded, setArchiveLoaded] = useState(false);
  const [addingOpen, setAddingOpen] = useState(false);

  // Reload archive when project changes or the user opens the archive tab.
  useEffect(() => {
    if (tab !== "archive" || !project) return;
    api.listBacklog(project.id).then((list) => {
      setArchive(list.filter((b) => b.status === "done" || b.status === "dismissed"));
      setArchiveLoaded(true);
    });
  }, [tab, project, activeBacklog]);

  if (!project) {
    return (
      <div className="flex-1 flex items-center justify-center text-xs text-muted-foreground/70 px-6 text-center">
        Открой проект чтобы увидеть беклог
      </div>
    );
  }

  const items = tab === "active" ? activeBacklog : archive;

  const dismiss = async (id: string) => {
    await api.dismissBacklogItem(id);
    onChanged?.();
  };

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="px-3 pt-3 pb-2 flex items-center gap-2 shrink-0">
        <div className="inline-flex rounded-md border border-border overflow-hidden">
          <button
            onClick={() => setTab("active")}
            className={cn(
              "px-2.5 py-1 text-[11px] font-medium transition-colors",
              tab === "active" ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/50"
            )}
          >
            Активные ({activeBacklog.length})
          </button>
          <button
            onClick={() => setTab("archive")}
            className={cn(
              "px-2.5 py-1 text-[11px] font-medium transition-colors border-l border-border",
              tab === "archive" ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/50"
            )}
          >
            Архив {archiveLoaded && `(${archive.length})`}
          </button>
        </div>
        <div className="flex-1" />
        <Button
          variant="outline"
          size="sm"
          className="h-7 gap-1.5"
          onClick={() => setAddingOpen((v) => !v)}
        >
          <Plus className="h-3 w-3" />
          Добавить
        </Button>
      </div>
      {addingOpen && (
        <AddItemForm
          project={project}
          onClose={() => setAddingOpen(false)}
          onAdded={() => {
            setAddingOpen(false);
            onChanged?.();
          }}
        />
      )}
      <div className="flex-1 overflow-y-auto scrollbar-thin px-3 pb-3 pt-1">
        {items.length === 0 ? (
          <div className="text-center py-12 text-xs text-muted-foreground/70">
            {tab === "active"
              ? "Беклог пуст. PO будет искать идеи в коде и истории."
              : "В архиве пока ничего нет."}
          </div>
        ) : (
          <div className="space-y-1.5">
            {items.map((item) => (
              <BacklogItemCard key={item.id} item={item} onDismiss={() => dismiss(item.id)} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function BacklogItemCard({
  item,
  onDismiss,
}: {
  item: BacklogItem;
  onDismiss: () => void;
}) {
  const [open, setOpen] = useState(false);
  const source = SOURCE_LABEL[item.source];
  const cat = CATEGORY_META[item.category];
  const prio = PRIORITY_LABEL[item.priority];
  const inIteration = item.status === "in_iteration";
  const archived = item.status === "done" || item.status === "dismissed";
  const canDismiss = !archived;

  const created = useMemo(() => {
    const d = new Date(item.created_at);
    return `${d.getMonth() + 1}/${d.getDate()}`;
  }, [item.created_at]);

  return (
    <div
      className={cn(
        "border border-border rounded-md bg-card/40 hover:bg-card/70 transition-colors",
        inIteration && "border-primary/50 bg-primary/5",
        item.status === "done" && "opacity-50",
        item.status === "dismissed" && "opacity-40 line-through"
      )}
    >
      <div className="flex items-start gap-1.5 px-2.5 py-2">
        <button
          onClick={() => setOpen((v) => !v)}
          className="mt-0.5 shrink-0 text-muted-foreground hover:text-foreground transition-colors"
        >
          {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        </button>
        <div className="flex-1 min-w-0">
          <div className="flex items-start gap-2">
            <div className="text-[13px] font-medium flex-1 min-w-0 break-words leading-snug">
              {item.title}
            </div>
          </div>
          <div className="flex items-center gap-1.5 mt-1 flex-wrap text-[10px] font-mono">
            <span
              className={cn(
                "inline-block px-1.5 py-px rounded border font-semibold",
                cat.color
              )}
            >
              {cat.emoji} {cat.label}
            </span>
            <span
              className={cn(
                "inline-block px-1.5 py-px rounded border",
                source.color
              )}
            >
              {source.label}
            </span>
            <span className={cn("font-semibold", prio.color)}>{prio.label}</span>
            {inIteration && (
              <span className="inline-block px-1.5 py-px rounded border border-primary/50 bg-primary/10 text-primary">
                в итерации
              </span>
            )}
            {item.status === "done" && (
              <span className="inline-block px-1.5 py-px rounded border border-success/50 bg-success/10 text-success">
                done
              </span>
            )}
            <span className="text-muted-foreground/60">{created}</span>
          </div>
        </div>
        {canDismiss && (
          <button
            onClick={onDismiss}
            className="shrink-0 p-1 rounded text-muted-foreground/50 hover:text-destructive hover:bg-destructive/10 transition-colors"
            title="Убрать из беклога"
          >
            <Trash2 className="h-3 w-3" />
          </button>
        )}
      </div>
      {open && item.details && (
        <div className="px-2.5 pb-2.5 pl-7 text-[11px] text-muted-foreground whitespace-pre-wrap leading-relaxed">
          {item.details}
        </div>
      )}
    </div>
  );
}

function AddItemForm({
  project,
  onClose,
  onAdded,
}: {
  project: ProjectRow;
  onClose: () => void;
  onAdded: () => void;
}) {
  const [title, setTitle] = useState("");
  const [details, setDetails] = useState("");
  const [category, setCategory] = useState<BacklogCategory>("feature");
  const [priority, setPriority] = useState<BacklogPriority>("normal");
  const [submitting, setSubmitting] = useState(false);
  const submit = async () => {
    const t = title.trim();
    if (!t || submitting) return;
    setSubmitting(true);
    try {
      await api.addBacklogItem(project.id, t, {
        details: details.trim() || undefined,
        category,
        priority,
      });
      onAdded();
    } finally {
      setSubmitting(false);
    }
  };
  return (
    <div className="mx-3 mb-2 p-3 border border-border rounded-md bg-card/50 space-y-2 shrink-0">
      <div className="flex items-start gap-2">
        <div className="flex-1 space-y-2">
          <Input
            placeholder="Что нужно сделать..."
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            autoFocus
          />
          <Textarea
            placeholder="Детали (опционально)"
            value={details}
            onChange={(e) => setDetails(e.target.value)}
            rows={2}
          />
          <CategorySelector value={category} onChange={setCategory} />
          <div className="flex items-center gap-3 text-[11px]">
            <Label className="text-xs">Приоритет:</Label>
            {(["high", "normal", "low"] as const).map((p) => (
              <label key={p} className="flex items-center gap-1 cursor-pointer">
                <input
                  type="radio"
                  className="h-3 w-3"
                  checked={priority === p}
                  onChange={() => setPriority(p)}
                />
                <span className={cn(priority === p ? "text-foreground" : "text-muted-foreground")}>
                  {p}
                </span>
              </label>
            ))}
          </div>
        </div>
        <button
          onClick={onClose}
          className="shrink-0 p-1 rounded text-muted-foreground hover:text-foreground hover:bg-secondary"
        >
          <X className="h-3 w-3" />
        </button>
      </div>
      <div className="flex gap-2 justify-end">
        <Button size="sm" variant="ghost" onClick={onClose}>
          Отмена
        </Button>
        <Button size="sm" disabled={!title.trim() || submitting} onClick={submit}>
          {submitting ? "Добавляю…" : "Добавить"}
        </Button>
      </div>
    </div>
  );
}

/** Reusable horizontal category picker. PO order — critical → wish — so
 *  user reads severity left to right. Hint text under each button comes
 *  from `CATEGORY_META[cat].hint`. */
export function CategorySelector({
  value,
  onChange,
  size = "md",
}: {
  value: BacklogCategory;
  onChange: (v: BacklogCategory) => void;
  size?: "sm" | "md";
}) {
  return (
    <div className="space-y-1">
      <Label className={cn("text-xs", size === "sm" && "text-[11px]")}>Категория:</Label>
      <div className="flex gap-1 flex-wrap">
        {CATEGORY_ORDER.map((c) => {
          const m = CATEGORY_META[c];
          const active = value === c;
          return (
            <button
              key={c}
              type="button"
              onClick={() => onChange(c)}
              title={m.hint}
              className={cn(
                "inline-flex items-center gap-1 rounded-md border text-[11px] font-medium transition-colors",
                size === "sm" ? "px-1.5 py-0.5" : "px-2 py-1",
                active
                  ? m.color + " ring-1 ring-current/30"
                  : "border-border text-muted-foreground hover:text-foreground hover:bg-accent/50"
              )}
            >
              <span>{m.emoji}</span>
              <span>{m.label}</span>
            </button>
          );
        })}
      </div>
      <p className="text-[10px] text-muted-foreground/80 leading-tight">
        {CATEGORY_META[value].hint}
      </p>
    </div>
  );
}
