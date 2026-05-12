import { useEffect, useState } from "react";
import { HelpCircle } from "lucide-react";
import type { QuestionRow } from "../types";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Label, Textarea } from "./ui/input";
import { ROLE_LABEL_RU } from "../lib/humanize";

export function QuestionModal({
  question, onAnswer,
}: { question: QuestionRow; onAnswer: (answer: string) => Promise<void> }) {
  const [text, setText] = useState("");
  const [submitting, setSubmitting] = useState(false);
  useEffect(() => { setText(""); }, [question.id]);
  const submit = async () => {
    const trimmed = text.trim();
    if (!trimmed || submitting) return;
    setSubmitting(true);
    try { await onAnswer(trimmed); } finally { setSubmitting(false); }
  };
  return (
    <Dialog open onOpenChange={() => {}}>
      <DialogContent hideClose className="max-w-xl">
        <DialogHeader>
          <div className="flex items-center gap-2 mb-1">
            <div className="rounded-full bg-warning/15 text-warning p-1.5">
              <HelpCircle className="h-4 w-4" />
            </div>
            <span className="text-[10px] uppercase tracking-wider font-semibold text-warning">Агент ждёт ответа</span>
          </div>
          <DialogTitle>{question.question}</DialogTitle>
          <DialogDescription>
            От: {question.agent_role ? ROLE_LABEL_RU[question.agent_role] ?? question.agent_role : "?"} · Blocker Reviewer решил, что без тебя не обойтись.
          </DialogDescription>
        </DialogHeader>
        {question.context && (
          <div className="rounded-md border border-border bg-background p-3 text-xs leading-relaxed whitespace-pre-wrap max-h-[260px] overflow-y-auto scrollbar-thin">
            <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-2">Контекст</div>
            {question.context}
          </div>
        )}
        <div className="space-y-1.5">
          <Label>Твой ответ</Label>
          <Textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit(); }}
            rows={5}
            placeholder="Напиши ответ. Cmd+Enter — отправить."
            autoFocus
          />
        </div>
        <DialogFooter>
          <Button disabled={!text.trim() || submitting} onClick={submit}>
            {submitting ? "Отправляю…" : "Отправить ответ"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
