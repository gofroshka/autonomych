import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Send } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
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
    setText("");
    setSending(true);
    const pendingId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? `pending-${crypto.randomUUID()}`
        : `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setMessages((prev) => [
      ...prev,
      {
        id: pendingId,
        project_id: project.id,
        role: "user",
        text: trimmed,
        ts: Date.now(),
        error: null,
      },
    ]);
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
        "max-w-[88%] rounded-2xl px-3.5 py-2.5 text-[13px] leading-relaxed break-words",
        isUser
          ? "bg-primary text-primary-foreground rounded-br-sm whitespace-pre-wrap"
          : "bg-secondary text-secondary-foreground rounded-bl-sm"
      )}>
        {isUser ? message.text : <AssistantMarkdown text={message.text} />}
        {message.error && <div className="mt-2 text-[10px] text-destructive italic">{message.error}</div>}
      </div>
    </div>
  );
}

/**
 * Render Overseer's reply as real markdown — headings, lists, inline code,
 * fenced code, links, tables (GFM). Links open in the user's system browser
 * via Tauri's opener, never inside the embedded webview.
 *
 * Styling lives inline via tailwind arbitrary-child selectors so we don't
 * leak chat-specific typography into a global stylesheet.
 */
function AssistantMarkdown({ text }: { text: string }) {
  return (
    <div
      className={cn(
        // Tight typography for the bubble: small line spacing, no h1/h2
        // giants — chat is conversational, not document-y.
        "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
        "[&_h1]:text-[14px] [&_h1]:font-semibold [&_h1]:mt-2.5 [&_h1]:mb-1.5",
        "[&_h2]:text-[13px] [&_h2]:font-semibold [&_h2]:mt-2.5 [&_h2]:mb-1.5",
        "[&_h3]:text-[13px] [&_h3]:font-semibold [&_h3]:mt-2 [&_h3]:mb-1 [&_h3]:opacity-80",
        "[&_p]:my-1.5",
        "[&_ul]:my-1.5 [&_ul]:pl-4 [&_ul]:list-disc [&_ul]:space-y-0.5",
        "[&_ol]:my-1.5 [&_ol]:pl-4 [&_ol]:list-decimal [&_ol]:space-y-0.5",
        "[&_li]:leading-snug [&_li>p]:my-0",
        "[&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:bg-background/60 [&_code]:text-[12px] [&_code]:font-mono",
        "[&_pre]:my-2 [&_pre]:p-2.5 [&_pre]:rounded [&_pre]:bg-background/70 [&_pre]:overflow-x-auto [&_pre]:text-[12px]",
        "[&_pre_code]:bg-transparent [&_pre_code]:p-0",
        "[&_strong]:font-semibold",
        "[&_em]:italic",
        "[&_blockquote]:border-l-2 [&_blockquote]:border-foreground/20 [&_blockquote]:pl-2.5 [&_blockquote]:opacity-80 [&_blockquote]:my-1.5",
        "[&_hr]:my-3 [&_hr]:border-foreground/15",
        "[&_table]:my-2 [&_table]:border-collapse [&_table]:text-[12px]",
        "[&_th]:border [&_th]:border-foreground/15 [&_th]:px-2 [&_th]:py-1 [&_th]:font-semibold [&_th]:text-left",
        "[&_td]:border [&_td]:border-foreground/15 [&_td]:px-2 [&_td]:py-1"
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
              className="underline decoration-foreground/40 underline-offset-2 hover:decoration-foreground cursor-pointer"
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
