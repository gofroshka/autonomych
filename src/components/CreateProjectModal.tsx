import { useEffect, useState } from "react";
import { ChevronDown, FolderOpen } from "lucide-react";
import { api } from "../lib/api";
import type { AgentBackend, CreateProjectInput, PermissionMode } from "../types";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input, Label, Textarea } from "./ui/input";
import { cn } from "../lib/cn";
import { BACKEND_DEFAULTS, modelsFor, type ModelSuggestion } from "../lib/agent-models";

interface Props {
  onClose: () => void;
  onCreate: (input: CreateProjectInput) => void;
}

export function CreateProjectModal({ onClose, onCreate }: Props) {
  const [name, setName] = useState("");
  const [idea, setIdea] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [backend, setBackend] = useState<AgentBackend>("claude_code");
  const [modelPm, setModelPm] = useState(BACKEND_DEFAULTS.claude_code.pm);
  const [modelSpec, setModelSpec] = useState(BACKEND_DEFAULTS.claude_code.spec);
  const [permMode, setPermMode] = useState<PermissionMode>("bypassPermissions");

  // Когда юзер переключает CLI, дефолтные модели должны автообновиться,
  // но только если он не успел их вручную поменять под текущий backend.
  useEffect(() => {
    setModelPm(BACKEND_DEFAULTS[backend].pm);
    setModelSpec(BACKEND_DEFAULTS[backend].spec);
  }, [backend]);

  const models = modelsFor(backend);

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
          <DialogDescription>Дай имя и опиши что хочешь построить — Автономыч соберёт команду агентов и начнёт цикл разработки.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label>Имя проекта</Label>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="eco-portal" autoFocus />
          </div>
          <div className="space-y-1.5">
            <Label>Видение продукта</Label>
            <Textarea value={idea} onChange={(e) => setIdea(e.target.value)} rows={5}
              placeholder="Корпоративный портал для компаний, занимающихся экологией..." />
            <p className="text-[11px] text-muted-foreground/80 leading-relaxed">
              Уйдёт в <code className="text-[10px]">docs/product/vision.md</code> — это источник истины для PO и Architect.
              Documenter будет обновлять файл по мере того, как проект эволюционирует, так что описание само догоняет реальность.
            </p>
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
              <div className="space-y-1.5">
                <Label>Агентский CLI</Label>
                <select
                  value={backend}
                  onChange={(e) => setBackend(e.target.value as AgentBackend)}
                  className="w-full h-9 rounded-md border border-border bg-background px-3 text-sm"
                >
                  <option value="claude_code">Claude Code — Anthropic Claude (claude CLI)</option>
                  <option value="codex">Codex — OpenAI Codex (codex CLI)</option>
                </select>
                <p className="text-[11px] text-muted-foreground">
                  CLI должен быть установлен и авторизован на твоей машине.
                  Изменить после создания проекта пока нельзя.
                </p>
              </div>
              <ModelSelect
                label="Модель для PM-агентов"
                hint="PO, Architect, Reviewer, Overseer"
                value={modelPm}
                onChange={setModelPm}
                models={models}
              />
              <ModelSelect
                label="Модель для специалистов"
                hint="Backend / Frontend / DevOps / Presenter / Documenter / Merge Resolver"
                value={modelSpec}
                onChange={setModelSpec}
                models={models}
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
              agent_backend: backend,
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
  models,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (v: string) => void;
  models: ReadonlyArray<ModelSuggestion>;
}) {
  // free-text input with datalist — works for any model name the CLI accepts,
  // plus shows the curated suggestions for the current backend.
  const listId = `models-${label.replace(/\s+/g, "-")}`;
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <Input
        list={listId}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        autoComplete="off"
      />
      <datalist id={listId}>
        {models.map((m) => (
          <option key={m.id} value={m.id}>
            {m.label} — {m.hint}
          </option>
        ))}
      </datalist>
      {hint && <div className="text-[11px] text-muted-foreground">{hint}</div>}
    </div>
  );
}
