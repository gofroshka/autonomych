import { useState } from "react";
import { AlertTriangle, CheckCircle2, ExternalLink, FolderOpen, Loader2, RotateCcw, Sparkles, Square, Wrench, XCircle } from "lucide-react";
import type { ConductorState, DashboardSnapshot, ProjectRow } from "../types";
import { api } from "../lib/api";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Label, Textarea } from "./ui/input";
import { cn } from "../lib/cn";

export function PresentingOverlay({
  project, snapshot, onResume,
}: {
  project: ProjectRow;
  snapshot: DashboardSnapshot | null;
  onResume: (msg: string, mode: "soft" | "override") => void;
}) {
  const [steering, setSteering] = useState("");
  const [mode, setMode] = useState<"soft" | "override">("soft");
  const [retrying, setRetrying] = useState(false);

  const state = (snapshot?.project?.state ?? "PRESENTING") as ConductorState;
  const preview = snapshot?.preview;
  const summary = snapshot?.iteration?.summary ?? "";
  const isPreparing = state === "PREPARING_PREVIEW";
  const previewReady = !!preview?.url && !!preview?.running;
  const previewFailed = !isPreparing && !previewReady && (preview?.prep_error || (preview?.errors?.length ?? 0) > 0);

  const retry = async () => {
    setRetrying(true);
    try { await api.retryPreview(project.id); } finally { setTimeout(() => setRetrying(false), 1500); }
  };
  const cancelPrep = async () => {
    // Hard cancel — kills the Presenter agent's claude subprocess and returns
    // the conductor to IDLE. User can press Start again to resume normal work.
    await api.stopConductor(project.id);
  };
  const openInBrowser = () => { if (preview?.url) api.openExternal(preview.url); };
  const openFolder = () => api.openExternal(project.root_path);

  return (
    <div className="absolute inset-0 z-20 bg-background overflow-y-auto scrollbar-thin">
      <div className="max-w-3xl mx-auto px-8 py-10 space-y-6">
        <div className="flex items-center gap-3 flex-wrap">
          {isPreparing ? (
            <Badge variant="warning"><Loader2 className="h-3 w-3 animate-spin" />Готовим демо</Badge>
          ) : (
            <Badge variant="info"><Sparkles className="h-3 w-3" />Презентация</Badge>
          )}
          <h1 className="text-xl font-semibold">
            Итерация #{snapshot?.iteration?.number}
            {isPreparing ? " — готовим к показу" : " — посмотри, что получилось"}
          </h1>
        </div>
        <section className="space-y-2">
          <Label>Что появилось</Label>
          <div className="rounded-lg border border-border bg-card p-4 text-sm whitespace-pre-wrap leading-relaxed">
            {summary || <span className="text-muted-foreground">(нет summary)</span>}
          </div>
        </section>
        <section className="space-y-3">
          <Label>Демо</Label>
          {isPreparing && (
            <div className="rounded-lg border border-border bg-card p-4 space-y-3">
              <div className="flex items-start justify-between gap-3 flex-wrap">
                <div className="flex items-center gap-2.5">
                  <Loader2 className="h-4 w-4 animate-spin text-primary" />
                  <span className="text-sm font-medium">Готовим демо…</span>
                </div>
                <Button variant="destructive" size="sm" onClick={cancelPrep} className="gap-1.5 h-7">
                  <XCircle className="h-3 w-3" />
                  Отменить подготовку
                </Button>
              </div>
              <p className="text-xs text-muted-foreground leading-relaxed">
                Presenter-агент ставит зависимости, мигрирует и запускает dev-сервер.
                Отмена прервёт его и вернёт цикл в IDLE.
              </p>
              {preview?.logs_tail && (
                <details className="text-[11px]">
                  <summary className="cursor-pointer text-muted-foreground">логи</summary>
                  <pre className="mt-2 bg-black border border-border rounded p-2 max-h-[160px] overflow-y-auto scrollbar-thin font-mono text-muted-foreground whitespace-pre-wrap">
                    {preview.logs_tail}
                  </pre>
                </details>
              )}
            </div>
          )}
          {!isPreparing && previewReady && preview && (
            <div className="rounded-lg border border-success/40 bg-success/5 p-4 space-y-3">
              <div className="flex items-start justify-between gap-3 flex-wrap">
                <div className="flex items-center gap-2.5"><CheckCircle2 className="h-4 w-4 text-success" /><span className="text-sm font-medium">Демо запущено</span></div>
                <Button variant="ghost" size="sm" onClick={retry} disabled={retrying} className="gap-1.5 h-7">
                  <RotateCcw className={cn("h-3 w-3", retrying && "animate-spin")} />
                  Перезапустить
                </Button>
              </div>
              <Button onClick={openInBrowser} className="gap-1.5 w-full sm:w-auto">
                <ExternalLink className="h-3.5 w-3.5" />
                Открыть {preview.url}
              </Button>
              {preview.notes && (
                <div className="text-xs text-foreground/80 leading-relaxed border-l-2 border-success/40 pl-3 whitespace-pre-wrap">{preview.notes}</div>
              )}
            </div>
          )}
          {!isPreparing && !previewReady && (
            <div className="rounded-lg border border-destructive/40 bg-destructive/5 p-4 space-y-3">
              <div className="flex items-start justify-between gap-3 flex-wrap">
                <div className="flex items-center gap-2.5">
                  <AlertTriangle className="h-4 w-4 text-destructive" />
                  <span className="text-sm font-medium">
                    {previewFailed ? "Не получилось запустить демо" : "Демо ещё не готовили"}
                  </span>
                </div>
                <Button onClick={retry} disabled={retrying} className="gap-1.5">
                  <Wrench className={cn("h-3.5 w-3.5", retrying && "animate-spin")} />
                  {retrying ? "Запускаю…" : "Подготовить заново"}
                </Button>
              </div>
              {preview?.prep_error && <div className="text-xs text-destructive leading-relaxed">{preview.prep_error}</div>}
            </div>
          )}
          <div className="flex items-center gap-2 flex-wrap pt-1">
            <Button variant="outline" size="sm" onClick={openFolder} className="gap-1.5">
              <FolderOpen className="h-3.5 w-3.5" />
              Папка проекта
            </Button>
            {!isPreparing && previewReady && (
              <Button variant="outline" size="sm" onClick={async () => { await api.stopPreview(project.id); }} className="gap-1.5">
                <Square className="h-3.5 w-3.5 fill-current" />
                Остановить сервер
              </Button>
            )}
          </div>
        </section>
        <section className="space-y-2 pt-2 border-t border-border">
          <Label>Курс-коррекция (опционально)</Label>
          <Textarea rows={4} value={steering} onChange={(e) => setSteering(e.target.value)}
            placeholder="Например: «Цвета слишком яркие». Или ничего не пиши — продолжим как было." />
          <div className="flex items-center gap-3 flex-wrap pt-1">
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input type="radio" className="h-3.5 w-3.5" checked={mode === "soft"} onChange={() => setMode("soft")} />
              <span className={cn(mode === "soft" ? "text-foreground" : "text-muted-foreground")}>soft (направление для PO)</span>
            </label>
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input type="radio" className="h-3.5 w-3.5" checked={mode === "override"} onChange={() => setMode("override")} />
              <span className={cn(mode === "override" ? "text-foreground" : "text-muted-foreground")}>override (выполнить буквально)</span>
            </label>
            <div className="flex-1" />
            <Button onClick={() => onResume(steering, mode)} disabled={isPreparing}>Продолжаем</Button>
          </div>
        </section>
      </div>
    </div>
  );
}
