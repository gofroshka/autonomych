import { useState } from "react";
import {
  AlertTriangle,
  FolderOpen,
  Loader2,
  MessageSquare,
  RotateCcw,
  Send,
  Sparkles,
  Wrench,
  XCircle,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { BacklogCategory, BacklogPriority, DashboardSnapshot, ProjectRow } from "../types";
import { CategorySelector } from "./BacklogPanel";
import { api } from "../lib/api";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Label, Textarea } from "./ui/input";
import { cn } from "../lib/cn";

/** UI feedback after pressing "retry" — leaves the spinner visible briefly
 *  so the click registers visually even when the IPC call resolves instantly. */
const RETRY_FEEDBACK_MS = 1500;

/** One turn in the user↔Presenter chat. Ephemeral component-local state. */
interface PresenterTurn {
  id: string;
  from: "user" | "presenter";
  text: string;
  /** Set on presenter turns when the agent attached a draft steering. */
  draftSteering?: string;
}

interface Props {
  project: ProjectRow;
  snapshot: DashboardSnapshot | null;
  /** Wake the conductor from Presenting → next iteration starts. */
  onResume: () => void;
  /** Fires after the user adds something to the backlog from this overlay,
   *  so the parent can refresh the snapshot and update the right-panel
   *  badge count. */
  onBacklogChanged: () => void;
}

export function PresentingOverlay({ project, snapshot, onResume, onBacklogChanged }: Props) {
  // Single compose form — user types feedback, picks a category (bug /
  // critical / feature / wish / ...), hits "+ В беклог". Can add multiple
  // before pressing "Продолжаем". PO of the next iteration honours category
  // ordering on its own (bugs before features); no override / direct prompt
  // injection involved.
  const [backlogText, setBacklogText] = useState("");
  const [backlogCategory, setBacklogCategory] = useState<BacklogCategory>("bug");
  const [backlogPriority, setBacklogPriority] = useState<BacklogPriority>("normal");
  const [addingToBacklog, setAddingToBacklog] = useState(false);
  const [justAddedCount, setJustAddedCount] = useState(0);
  const [retrying, setRetrying] = useState(false);
  const [chatTurns, setChatTurns] = useState<PresenterTurn[]>([]);
  const [chatInput, setChatInput] = useState("");
  const [chatSending, setChatSending] = useState(false);

  const state = snapshot?.project?.state ?? "PRESENTING";
  const preview = snapshot?.preview;
  const summary = snapshot?.iteration?.summary ?? "";
  const isPreparing = state === "PREPARING_PREVIEW";
  const hasInstructions = !!preview?.instructions;
  const hasError = !isPreparing && !!preview?.prep_error;

  const sendChat = async () => {
    const trimmed = chatInput.trim();
    if (!trimmed || chatSending) return;
    setChatInput("");
    setChatSending(true);
    const turnId = `t-${Date.now()}`;
    setChatTurns((prev) => [...prev, { id: turnId, from: "user", text: trimmed }]);
    try {
      const r = await api.presenterChat(project.id, trimmed);
      setChatTurns((prev) => [
        ...prev,
        {
          id: `t-${Date.now()}-r`,
          from: "presenter",
          text: r.reply,
          draftSteering: r.draft_steering ?? undefined,
        },
      ]);
    } catch (e) {
      setChatTurns((prev) => [
        ...prev,
        {
          id: `t-${Date.now()}-e`,
          from: "presenter",
          text: `_Не получилось спросить Presenter'а: ${String(e)}_`,
        },
      ]);
    } finally {
      setChatSending(false);
    }
  };

  // Presenter chat-replies can attach a `draft_steering` string — usually a
  // succinct description of a code-side bug. Stuff it into the backlog
  // compose box so the user can review and click "+ В беклог" (or move it
  // to override if it's truly critical).
  const applyDraft = (text: string) => {
    setBacklogText((cur) => (cur.trim() ? `${cur.trim()}\n\n${text}` : text));
  };

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

        {!isPreparing && (
          <section className="space-y-3 pt-2 border-t border-border">
            <div className="space-y-1">
              <Label>
                <MessageSquare className="inline h-3.5 w-3.5 mr-1.5 -mt-0.5" />
                Сообщить Presenter'у о проблеме
              </Label>
              <p className="text-[11px] text-muted-foreground leading-relaxed">
                Если демо работает не так как ожидаешь — напиши. Presenter сам разберётся:
                починит запуск если это его косяк (порт, env, перезапуск),
                либо предложит черновик правки для следующей итерации если это баг в коде.
              </p>
            </div>

            {chatTurns.length > 0 && (
              <div className="space-y-2.5">
                {chatTurns.map((turn) => (
                  <PresenterChatTurn
                    key={turn.id}
                    turn={turn}
                    onApplyDraft={() => turn.draftSteering && applyDraft(turn.draftSteering)}
                  />
                ))}
              </div>
            )}

            <div className="flex gap-2">
              <Textarea
                rows={2}
                value={chatInput}
                onChange={(e) => setChatInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    sendChat();
                  }
                }}
                placeholder="Например: «при открытии товара фронт идёт на 5000 порт, а сервер на 3000»"
                disabled={chatSending}
                className="resize-none flex-1"
              />
              <Button
                onClick={sendChat}
                disabled={!chatInput.trim() || chatSending}
                size="sm"
                className="gap-1.5 self-end"
              >
                <Send className="h-3 w-3" />
                {chatSending ? "Разбираюсь…" : "Спросить"}
              </Button>
            </div>
          </section>
        )}

        <section className="space-y-3 pt-2 border-t border-border">
          <div className="flex items-baseline justify-between gap-2">
            <Label>Правки и идеи → в беклог</Label>
            <span className="text-[11px] text-muted-foreground">
              Можно добавлять сколько хочешь. PO сначала закрывает критич/баги, потом фичи.
            </span>
          </div>
          <Textarea
            rows={3}
            value={backlogText}
            onChange={(e) => setBacklogText(e.target.value)}
            placeholder="Например: «Главная страница 500-тит при логине», «добавить экспорт в CSV», «цвета слишком яркие»."
          />
          <CategorySelector value={backlogCategory} onChange={setBacklogCategory} />
          <div className="flex items-center gap-3 flex-wrap">
            <span className="text-[11px] text-muted-foreground">Приоритет:</span>
            {(["high", "normal", "low"] as const).map((p) => (
              <label key={p} className="flex items-center gap-1 text-xs cursor-pointer">
                <input
                  type="radio"
                  className="h-3 w-3"
                  checked={backlogPriority === p}
                  onChange={() => setBacklogPriority(p)}
                />
                <span
                  className={cn(
                    backlogPriority === p ? "text-foreground" : "text-muted-foreground"
                  )}
                >
                  {p}
                </span>
              </label>
            ))}
            <div className="flex-1" />
            <Button
              variant="outline"
              size="sm"
              onClick={async () => {
                const text = backlogText.trim();
                if (!text || addingToBacklog) return;
                setAddingToBacklog(true);
                try {
                  await api.addBacklogItem(project.id, text, {
                    category: backlogCategory,
                    priority: backlogPriority,
                  });
                  setBacklogText("");
                  setJustAddedCount((n) => n + 1);
                  onBacklogChanged();
                } finally {
                  setAddingToBacklog(false);
                }
              }}
              disabled={!backlogText.trim() || addingToBacklog}
              className="gap-1.5"
            >
              {addingToBacklog ? "Добавляю…" : "+ В беклог"}
            </Button>
          </div>
          {justAddedCount > 0 && (
            <p className="text-[11px] text-success">
              ✓ Добавлено в беклог: {justAddedCount}. Можешь добавить ещё или сразу продолжить.
            </p>
          )}
          <div className="flex items-center gap-2 pt-2 border-t border-border/50">
            <div className="flex-1 text-[11px] text-muted-foreground">
              {backlogText.trim() && (
                <span className="block">
                  ⚠️ В поле есть текст — он не сохранится, если не нажать «+ В беклог».
                </span>
              )}
            </div>
            <Button onClick={onResume} disabled={isPreparing}>
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

/**
 * One turn in the user↔Presenter chat. User messages render as plain text
 * (whitespace-preserving), Presenter replies render through the same
 * markdown pipeline as the launch instructions block. If the agent
 * attached a draft steering, a small action button below the bubble
 * copies it into the course-correction textarea above.
 */
function PresenterChatTurn({
  turn,
  onApplyDraft,
}: {
  turn: PresenterTurn;
  onApplyDraft: () => void;
}) {
  const isUser = turn.from === "user";
  return (
    <div className={cn("flex flex-col gap-1.5", isUser ? "items-end" : "items-start")}>
      <div
        className={cn(
          "max-w-[88%] rounded-lg px-3 py-2 text-[13px] leading-relaxed break-words",
          isUser
            ? "bg-primary text-primary-foreground whitespace-pre-wrap"
            : "bg-card border border-border"
        )}
      >
        {isUser ? turn.text : <InstructionsBlock text={turn.text} />}
      </div>
      {turn.draftSteering && (
        <div className="max-w-[88%] flex items-start gap-2 px-3 py-2 rounded-lg border border-info/40 bg-info/5">
          <div className="flex-1 min-w-0 space-y-1.5">
            <div className="text-[11px] text-info font-medium">
              Presenter предлагает курс-коррекцию
            </div>
            <div className="text-[12px] text-foreground/80 leading-snug whitespace-pre-wrap">
              {turn.draftSteering}
            </div>
          </div>
          <Button size="sm" variant="outline" onClick={onApplyDraft} className="shrink-0 h-7 text-[11px]">
            Применить
          </Button>
        </div>
      )}
    </div>
  );
}
