import { useState } from "react";
import {
  AlertTriangle,
  FolderOpen,
  Loader2,
  RotateCcw,
  Sparkles,
  Wrench,
  XCircle,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { DashboardSnapshot, ProjectRow, SteeringMode } from "../types";
import { api } from "../lib/api";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Label, Textarea } from "./ui/input";
import { cn } from "../lib/cn";

/** UI feedback after pressing "retry" — leaves the spinner visible briefly
 *  so the click registers visually even when the IPC call resolves instantly. */
const RETRY_FEEDBACK_MS = 1500;

interface Props {
  project: ProjectRow;
  snapshot: DashboardSnapshot | null;
  onResume: (msg: string, mode: SteeringMode) => void;
}

export function PresentingOverlay({ project, snapshot, onResume }: Props) {
  const [steering, setSteering] = useState("");
  const [mode, setMode] = useState<SteeringMode>("soft");
  const [retrying, setRetrying] = useState(false);

  const state = snapshot?.project?.state ?? "PRESENTING";
  const preview = snapshot?.preview;
  const summary = snapshot?.iteration?.summary ?? "";
  const isPreparing = state === "PREPARING_PREVIEW";
  const hasInstructions = !!preview?.instructions;
  const hasError = !isPreparing && !!preview?.prep_error;

  const retry = async () => {
    setRetrying(true);
    try {
      await api.retryPreview(project.id);
    } finally {
      setTimeout(() => setRetrying(false), RETRY_FEEDBACK_MS);
    }
  };

  const cancelPrep = async () => {
    // Hard cancel — kills the Presenter agent's claude subprocess and returns
    // the conductor to IDLE. User can press Start again to resume normal work.
    await api.stopConductor(project.id);
  };

  const openFolder = () => api.openExternal(project.root_path);

  return (
    <div className="absolute inset-0 z-20 bg-background overflow-y-auto scrollbar-thin">
      <div className="max-w-3xl mx-auto px-8 py-10 space-y-6">
        <div className="flex items-center gap-3 flex-wrap">
          {isPreparing ? (
            <Badge variant="warning">
              <Loader2 className="h-3 w-3 animate-spin" />
              Готовим демо
            </Badge>
          ) : (
            <Badge variant="info">
              <Sparkles className="h-3 w-3" />
              Презентация
            </Badge>
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
                  <span className="text-sm font-medium">Presenter готовит демо…</span>
                </div>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={cancelPrep}
                  className="gap-1.5 h-7"
                >
                  <XCircle className="h-3 w-3" />
                  Отменить
                </Button>
              </div>
              <p className="text-xs text-muted-foreground leading-relaxed">
                Агент ставит зависимости, поднимает сервисы и запускает приложение.
                Отмена прервёт его и вернёт цикл в IDLE.
              </p>
            </div>
          )}

          {!isPreparing && hasInstructions && (
            <div className="rounded-lg border border-success/40 bg-success/5 p-4 space-y-3">
              <div className="flex items-start justify-between gap-3 flex-wrap">
                <div className="flex items-center gap-2.5">
                  <Sparkles className="h-4 w-4 text-success" />
                  <span className="text-sm font-medium">Демо готово</span>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={retry}
                  disabled={retrying}
                  className="gap-1.5 h-7"
                >
                  <RotateCcw className={cn("h-3 w-3", retrying && "animate-spin")} />
                  Перезапустить
                </Button>
              </div>

              <InstructionsBlock text={preview!.instructions!} />
            </div>
          )}

          {!isPreparing && !hasInstructions && (
            <div className="rounded-lg border border-destructive/40 bg-destructive/5 p-4 space-y-3">
              <div className="flex items-start justify-between gap-3 flex-wrap">
                <div className="flex items-center gap-2.5">
                  <AlertTriangle className="h-4 w-4 text-destructive" />
                  <span className="text-sm font-medium">
                    {hasError ? "Не получилось запустить демо" : "Демо ещё не готовили"}
                  </span>
                </div>
                <Button onClick={retry} disabled={retrying} className="gap-1.5">
                  <Wrench className={cn("h-3.5 w-3.5", retrying && "animate-spin")} />
                  {retrying ? "Запускаю…" : "Подготовить"}
                </Button>
              </div>
              {preview?.prep_error && (
                <div className="text-xs text-destructive leading-relaxed">
                  {preview.prep_error}
                </div>
              )}
            </div>
          )}

          <div className="flex items-center gap-2 flex-wrap pt-1">
            <Button variant="outline" size="sm" onClick={openFolder} className="gap-1.5">
              <FolderOpen className="h-3.5 w-3.5" />
              Папка проекта
            </Button>
          </div>
        </section>

        <section className="space-y-2 pt-2 border-t border-border">
          <Label>Курс-коррекция (опционально)</Label>
          <Textarea
            rows={4}
            value={steering}
            onChange={(e) => setSteering(e.target.value)}
            placeholder="Например: «Цвета слишком яркие». Или ничего не пиши — продолжим как было."
          />
          <div className="flex items-center gap-3 flex-wrap pt-1">
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="radio"
                className="h-3.5 w-3.5"
                checked={mode === "soft"}
                onChange={() => setMode("soft")}
              />
              <span className={cn(mode === "soft" ? "text-foreground" : "text-muted-foreground")}>
                soft (направление для PO)
              </span>
            </label>
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="radio"
                className="h-3.5 w-3.5"
                checked={mode === "override"}
                onChange={() => setMode("override")}
              />
              <span
                className={cn(mode === "override" ? "text-foreground" : "text-muted-foreground")}
              >
                override (выполнить буквально)
              </span>
            </label>
            <div className="flex-1" />
            <Button onClick={() => onResume(steering, mode)} disabled={isPreparing}>
              Продолжаем
            </Button>
          </div>
        </section>
      </div>
    </div>
  );
}

/**
 * Render the Presenter agent's free-form text.
 *
 * It's markdown — possibly with bare URLs (`**http://...**` or `<url>` or
 * `[label](url)`). We use react-markdown + GFM so all three forms render
 * as real clickable links. URL clicks go through Tauri's opener so they
 * land in the user's actual browser, not the embedded webview.
 */
function InstructionsBlock({ text }: { text: string }) {
  return (
    <div
      className={cn(
        "text-[13px] leading-relaxed text-foreground/90",
        // Inline markdown styling — small but readable.
        "[&_h1]:text-base [&_h1]:font-semibold [&_h1]:mt-3 [&_h1]:mb-2",
        "[&_h2]:text-[15px] [&_h2]:font-semibold [&_h2]:mt-3 [&_h2]:mb-2",
        "[&_h3]:text-[13px] [&_h3]:font-semibold [&_h3]:mt-2.5 [&_h3]:mb-1.5 [&_h3]:text-foreground/80",
        "[&_p]:my-1.5",
        "[&_ul]:my-1.5 [&_ul]:pl-5 [&_ul]:list-disc [&_ul]:space-y-0.5",
        "[&_ol]:my-1.5 [&_ol]:pl-5 [&_ol]:list-decimal [&_ol]:space-y-0.5",
        "[&_li]:leading-snug",
        "[&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:bg-secondary [&_code]:text-[12px]",
        "[&_pre]:p-3 [&_pre]:rounded [&_pre]:bg-secondary [&_pre]:overflow-x-auto [&_pre]:my-2",
        "[&_strong]:font-semibold [&_strong]:text-foreground"
      )}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children, ...rest }) => (
            <a
              {...rest}
              href={href}
              onClick={(e) => {
                e.preventDefault();
                if (href) api.openExternal(href);
              }}
              className="text-primary underline decoration-primary/40 underline-offset-2 hover:decoration-primary cursor-pointer"
            >
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
