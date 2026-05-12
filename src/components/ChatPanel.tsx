import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Send } from "lucide-react";
import type { ChatMessageRow, ProjectRow } from "../types";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Textarea } from "./ui/input";
import { cn } from "../lib/cn";

export function ChatPanel({ project }: { project: ProjectRow }) {
  const [messages, setMessages] = useState<ChatMessageRow[]>([]);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(async () => {
    setMessages(await api.getChatHistory(project.id));
  }, [project.id]);

  useEffect(() => { refresh(); }, [refresh]);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, sending]);

  const send = async () => {
    const trimmed = text.trim();
    if (!trimmed || sending) return;
    setText(""); setSending(true);
    setMessages((prev) => [...prev, { id: `pending-${Date.now()}`, project_id: project.id, role: "user", text: trimmed, ts: Date.now(), error: null }]);
    try {
      await api.sendChatMessage(project.id, trimmed);
      await refresh();
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div ref={scrollRef} className="flex-1 overflow-y-auto scrollbar-thin px-4 py-4 space-y-3">
        {messages.length === 0 && (
          <div className="text-center py-12 px-2 space-y-3">
            <p className="text-sm text-foreground/80">Спроси Overseer'а что происходит в проекте.</p>
            <p className="text-xs text-muted-foreground">«как идёт текущая итерация?», «почему выбрали Prisma?»</p>
            <p className="text-[11px] text-muted-foreground/70 max-w-[280px] mx-auto pt-2 border-t border-border/50 mt-4">
              Только читает код и объясняет. Для правок — «Завершить и показать» + steering.
            </p>
          </div>
        )}
        {messages.map((m) => <ChatBubble key={m.id} message={m} />)}
        {sending && <ThinkingBubble />}
      </div>
      <div className="border-t border-border bg-card/40 p-3 space-y-2">
        <Textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
          rows={2}
          placeholder="Спроси что-нибудь… Enter — отправить, Shift+Enter — перенос"
          disabled={sending}
          className="resize-none"
        />
        <div className="flex justify-end">
          <Button onClick={send} disabled={!text.trim() || sending} size="sm" className="gap-1.5">
            <Send className="h-3 w-3" />
            {sending ? "Думаю…" : "Отправить"}
          </Button>
        </div>
      </div>
    </div>
  );
}

function ChatBubble({ message }: { message: ChatMessageRow }) {
  const isUser = message.role === "user";
  return (
    <div className={cn("flex", isUser ? "justify-end" : "justify-start")}>
      <div className={cn(
        "max-w-[88%] rounded-2xl px-3.5 py-2.5 text-[13px] leading-relaxed whitespace-pre-wrap break-words",
        isUser ? "bg-primary text-primary-foreground rounded-br-sm" : "bg-secondary text-secondary-foreground rounded-bl-sm"
      )}>
        {message.text}
        {message.error && <div className="mt-2 text-[10px] text-destructive italic">{message.error}</div>}
      </div>
    </div>
  );
}

function ThinkingBubble() {
  return (
    <div className="flex justify-start">
      <div className="bg-secondary rounded-2xl rounded-bl-sm px-4 py-3 flex items-center gap-1.5">
        <span className="h-1.5 w-1.5 bg-muted-foreground rounded-full animate-dot-thinking" />
        <span className="h-1.5 w-1.5 bg-muted-foreground rounded-full animate-dot-thinking [animation-delay:0.15s]" />
        <span className="h-1.5 w-1.5 bg-muted-foreground rounded-full animate-dot-thinking [animation-delay:0.3s]" />
      </div>
    </div>
  );
}
