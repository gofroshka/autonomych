import { useState } from "react";
import type { ProjectRow } from "../types";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input, Label, Textarea } from "./ui/input";

export function RenameProjectModal({
  project, onClose, onSave,
}: { project: ProjectRow; onClose: () => void; onSave: (name: string, idea: string) => Promise<void> }) {
  const [name, setName] = useState(project.name);
  const [idea, setIdea] = useState(project.idea);
  const [submitting, setSubmitting] = useState(false);
  const dirty = name.trim() !== project.name || idea.trim() !== project.idea;
  const isRunning = project.state === "RUNNING" || project.state === "WRAPPING_UP" || project.state === "RESUMING" || project.state === "PREPARING_PREVIEW";
  const canSave = dirty && name.trim().length > 0 && !submitting && !isRunning;
  const submit = async () => {
    if (!canSave) return;
    setSubmitting(true);
    try { await onSave(name.trim(), idea.trim()); onClose(); } finally { setSubmitting(false); }
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Редактировать проект</DialogTitle>
          <DialogDescription>
            {isRunning
              ? "Цикл сейчас работает — редактирование заблокировано."
              : "Это только UI-описание для сайдбара. Чтобы реально поменять направление проекта — отредактируй docs/product/vision.md (или дай айтем в беклог с категорией critical)."}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label>Имя</Label>
            <Input value={name} onChange={(e) => setName(e.target.value)} disabled={isRunning} autoFocus />
          </div>
          <div className="space-y-1.5">
            <Label>Описание (для сайдбара)</Label>
            <Textarea value={idea} onChange={(e) => setIdea(e.target.value)} rows={4} disabled={isRunning} />
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose} disabled={submitting}>Отмена</Button>
          <Button disabled={!canSave} onClick={submit}>{submitting ? "Сохраняю…" : "Сохранить"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
