import { useState } from "react";
import { AlertTriangle } from "lucide-react";
import type { ProjectRow } from "../types";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input, Label } from "./ui/input";

export function DeleteProjectModal({
  project, onClose, onConfirm,
}: {
  project: ProjectRow;
  onClose: () => void;
  onConfirm: (deleteFiles: boolean) => Promise<void>;
}) {
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [confirmText, setConfirmText] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const canConfirm = confirmText === project.name && !submitting;
  const submit = async () => {
    if (!canConfirm) return;
    setSubmitting(true);
    try { await onConfirm(deleteFiles); } finally { setSubmitting(false); }
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Удалить «{project.name}»?</DialogTitle>
          <DialogDescription>Метаданные удалятся без возможности восстановить.</DialogDescription>
        </DialogHeader>
        <label className="flex items-start gap-3 rounded-lg border border-border bg-background p-3 cursor-pointer hover:bg-accent/30 transition-colors">
          <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium">Также удалить папку с кодом</div>
            <div className="text-[11px] text-muted-foreground font-mono truncate mt-0.5">{project.root_path}</div>
            {deleteFiles && (
              <div className="mt-2 flex items-center gap-1.5 text-[11px] text-destructive font-semibold">
                <AlertTriangle className="h-3 w-3" />
                Содержимое папки удалится безвозвратно.
              </div>
            )}
          </div>
        </label>
        <div className="space-y-1.5">
          <Label>Введи имя проекта: <span className="text-foreground font-mono">{project.name}</span></Label>
          <Input value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={project.name} autoFocus />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose} disabled={submitting}>Отмена</Button>
          <Button variant="destructive" disabled={!canConfirm} onClick={submit}>
            {submitting ? "Удаляю…" : deleteFiles ? "Удалить проект и код" : "Удалить проект"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
