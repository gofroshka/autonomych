import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle, ArrowDown, Brain, CheckCircle2, FileCode, FilePlus,
  HelpCircle, MessageSquareReply, Play, RefreshCcw, Search, Sparkles, Square, Terminal,
  type LucideIcon,
} from "lucide-react";
import type { EventRow } from "../types";
import { humanize, ROLE_LABEL_RU, type HumanEvent } from "../lib/humanize";
import { Button } from "./ui/button";
import { cn } from "../lib/cn";

const NEAR_BOTTOM_PX = 80;

const KIND_META: Record<HumanEvent["kind"], { icon: LucideIcon; color: string }> = {
  info: { icon: Sparkles, color: "text-muted-foreground" },
  thinking: { icon: Brain, color: "text-info" },
  read: { icon: FileCode, color: "text-muted-foreground" },
  write: { icon: FilePlus, color: "text-success" },
  shell: { icon: Terminal, color: "text-warning" },
  search: { icon: Search, color: "text-muted-foreground" },
  result: { icon: CheckCircle2, color: "text-success" },
  error: { icon: AlertCircle, color: "text-destructive" },
  lifecycle: { icon: Play, color: "text-primary" },
  directive: { icon: RefreshCcw, color: "text-warning" },
  question: { icon: HelpCircle, color: "text-warning" },
  answer: { icon: MessageSquareReply, color: "text-success" },
};

export function ActivityLog({ events }: { events: EventRow[] }) {
  const items: HumanEvent[] = useMemo(
    () =>
      events
        .slice()
        .sort((a, b) => a.ts - b.ts)
        .map(humanize)
        .filter((x): x is HumanEvent => x !== null),
    [events]
  );
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const [hasNew, setHasNew] = useState(false);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    const near = distance < NEAR_BOTTOM_PX;
    stickRef.current = near;
    if (near) setHasNew(false);
  };

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (stickRef.current) {
      el.scrollTop = el.scrollHeight;
      setHasNew(false);
    } else setHasNew(true);
  }, [items.length]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);

  const jumpToBottom = () => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    stickRef.current = true;
    setHasNew(false);
  };

  return (
    <div className="flex-1 min-h-0 relative flex flex-col">
      <div ref={scrollRef} onScroll={handleScroll} className="flex-1 overflow-y-auto scrollbar-thin px-3 py-3 space-y-1">
        {items.length === 0 && (
          <div className="text-xs text-muted-foreground/70 text-center py-12 px-4">
            <Square className="h-5 w-5 mx-auto mb-3 opacity-30" />
            Тишина. Запусти цикл — здесь появятся живые шаги агентов.
          </div>
        )}
        {items.map((it) => <ActivityRow key={it.id} item={it} />)}
      </div>
      {hasNew && (
        <Button variant="default" size="sm" onClick={jumpToBottom} className="absolute bottom-3 left-1/2 -translate-x-1/2 shadow-lg rounded-full h-7">
          <ArrowDown className="h-3 w-3" />
          Новые события
        </Button>
      )}
    </div>
  );
}

function ActivityRow({ item }: { item: HumanEvent }) {
  const meta = KIND_META[item.kind];
  const Icon = meta.icon;
  const time = new Date(item.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return (
    <div
      className={cn(
        "group flex items-start gap-2.5 rounded-md px-2 py-1.5 -mx-1 transition-colors hover:bg-accent/40",
        item.kind === "lifecycle" && "bg-primary/5",
        item.kind === "error" && "bg-destructive/5"
      )}
      title={item.detail ?? undefined}
    >
      <Icon className={cn("h-3.5 w-3.5 mt-1 shrink-0", meta.color)} />
      <div className="flex-1 min-w-0">
        <div className="flex items-baseline gap-2 flex-wrap">
          {item.role && (
            <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              {ROLE_LABEL_RU[item.role]}
            </span>
          )}
          <span className="text-[13px] leading-snug">{item.action}</span>
        </div>
        {item.target && (
          <div className="text-[11px] text-muted-foreground font-mono truncate mt-0.5">{item.target}</div>
        )}
      </div>
      <span className="text-[10px] text-muted-foreground/60 shrink-0 mt-1 font-mono">{time}</span>
    </div>
  );
}
