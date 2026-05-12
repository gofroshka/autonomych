import { useState } from "react";
import { ChevronDown, FolderOpen } from "lucide-react";
import { api } from "../lib/api";
import type { CreateProjectInput, PermissionMode } from "../types";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input, Label, Textarea } from "./ui/input";
import { cn } from "../lib/cn";

interface Props {
  onClose: () => void;
  onCreate: (input: CreateProjectInput) => void;
}

/** CLI-resolved aliases — always point to the latest model in each family.
 *  No version pinning, no maintenance burden on us when Anthropic ships
 *  a new revision. */
const MODELS: ReadonlyArray<{ id: string; label: string; hint: string }> = [
  { id: "opus", label: "Opus", hint: "самая умная и дорогая" },
  { id: "sonnet", label: "Sonnet", hint: "баланс цена/качество" },
  { id: "haiku", label: "Haiku", hint: "быстрая и дешёвая" },
];

export function CreateProjectModal({ onClose, onCreate }: Props) {
  const [name, setName] = useState("");
  const [idea, setIdea] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [modelPm, setModelPm] = useState("opus");
  const [modelSpec, setModelSpec] = useState("sonnet");
  const [permMode, setPermMode] = useState<PermissionMode>("bypassPermissions");

  const pickDir = async () => {
    const p = await api.pickDirectory();
    if (p) setRootPath(p);
  };
  const canCreate = name.trim() && idea.trim() && rootPath.trim();

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Новый проект</DialogTitle>
          <DialogDescription>Дай имя и опиши идею. Автономыч поднимет команду агентов и начнёт цикл разработки.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label>Имя проекта</Label>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="eco-portal" autoFocus />
          </div>
          <div className="space-y-1.5">
            <Label>Идея</Label>
            <Textarea value={idea} onChange={(e) => setIdea(e.target.value)} rows={5}
              placeholder="Корпоративный портал для компаний, занимающихся экологией..." />
          </div>
          <div className="space-y-1.5">
            <Label>Папка проекта</Label>
            <div className="flex gap-2">
              <Input value={rootPath} onChange={(e) => setRootPath(e.target.value)} placeholder="/Users/you/projects/eco-portal" />
              <Button variant="outline" size="icon" onClick={pickDir}><FolderOpen className="h-4 w-4" /></Button>
            </div>
          </div>
          <button type="button" onClick={() => setAdvanced((v) => !v)}
            className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 transition-colors">
            <ChevronDown className={cn("h-3 w-3 transition-transform", advanced && "rotate-180")} />
            Продвинутые настройки
          </button>
          {advanced && (
            <div className="space-y-4 border-l-2 border-border pl-4 ml-1">
              <ModelSelect
                label="Модель для PM-агентов"
                hint="PO, Architect, Reviewer, Overseer"
                value={modelPm}
                onChange={setModelPm}
              />
              <ModelSelect
                label="Модель для специалистов"
                hint="Backend / Frontend / DevOps / Presenter / Documenter / Merge Resolver"
                value={modelSpec}
                onChange={setModelSpec}
              />
              <div className="space-y-1.5">
                <Label>Permission mode</Label>
                <select value={permMode} onChange={(e) => setPermMode(e.target.value as typeof permMode)}
                  className="w-full h-9 rounded-md border border-border bg-background px-3 text-sm">
                  <option value="bypassPermissions">bypassPermissions (полная автономия)</option>
                  <option value="acceptEdits">acceptEdits (Bash блокируется)</option>
                  <option value="default">default (спрашивать каждый раз)</option>
                </select>
              </div>
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>Отмена</Button>
          <Button
            disabled={!canCreate}
            onClick={() => onCreate({
              name: name.trim(), idea: idea.trim(), root_path: rootPath.trim(),
              model_pm: modelPm, model_specialist: modelSpec, permission_mode: permMode,
            })}
          >Создать</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ModelSelect({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-full h-9 rounded-md border border-border bg-background px-3 text-sm"
      >
        {MODELS.map((m) => (
          <option key={m.id} value={m.id}>
            {m.label} — {m.hint}
          </option>
        ))}
      </select>
    </div>
  );
}
