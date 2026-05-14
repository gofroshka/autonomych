import { useEffect, useState } from "react";
import type { AgentBackend, PermissionMode, ProjectRow } from "../types";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input, Label } from "./ui/input";
import { BACKEND_DEFAULTS, modelsFor, type ModelSuggestion } from "../lib/agent-models";

export function ProjectSettingsModal({
  project,
  onClose,
  onSave,
}: {
  project: ProjectRow;
  onClose: () => void;
  onSave: (s: {
    modelPm: string;
    modelSpecialist: string;
    permissionMode: PermissionMode;
    agentBackend: AgentBackend;
  }) => Promise<void>;
}) {
  const initialBackend: AgentBackend = project.agent_backend ?? "claude_code";
  const [backend, setBackend] = useState<AgentBackend>(initialBackend);
  const [modelPm, setModelPm] = useState(project.model_pm);
  const [modelSpec, setModelSpec] = useState(project.model_specialist);
  const [permMode, setPermMode] = useState<PermissionMode>(project.permission_mode);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // When the user switches CLI, replace the model fields with the backend's
  // defaults — the previous model name almost never makes sense for the
  // other CLI. User can still override afterwards via free-text input.
  useEffect(() => {
    if (backend === initialBackend) return;
    setModelPm(BACKEND_DEFAULTS[backend].pm);
    setModelSpec(BACKEND_DEFAULTS[backend].spec);
  }, [backend, initialBackend]);

  const isRunning =
    project.state === "RUNNING" ||
    project.state === "WRAPPING_UP" ||
    project.state === "RESUMING" ||
    project.state === "PREPARING_PREVIEW";

  const dirty =
    backend !== initialBackend ||
    modelPm !== project.model_pm ||
    modelSpec !== project.model_specialist ||
    permMode !== project.permission_mode;

  const canSave =
    dirty && !submitting && !isRunning && modelPm.trim() && modelSpec.trim();

  const submit = async () => {
    if (!canSave) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSave({
        modelPm: modelPm.trim(),
        modelSpecialist: modelSpec.trim(),
        permissionMode: permMode,
        agentBackend: backend,
      });
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const models = modelsFor(backend);

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Настройки проекта</DialogTitle>
          <DialogDescription>
            {isRunning
              ? "Цикл сейчас работает — настройки можно менять после остановки."
              : "Изменения применятся к следующему запуску агентов."}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label>Агентский CLI</Label>
            <select
              value={backend}
              onChange={(e) => setBackend(e.target.value as AgentBackend)}
              disabled={isRunning}
              className="w-full h-9 rounded-md border border-border bg-background px-3 text-sm disabled:opacity-50"
            >
              <option value="claude_code">Claude Code — Anthropic Claude (claude CLI)</option>
              <option value="codex">Codex — OpenAI Codex (codex CLI)</option>
            </select>
            <p className="text-[11px] text-muted-foreground">
              CLI должен быть установлен и авторизован на твоей машине.
            </p>
          </div>
          <ModelSelect
            label="Модель для PM-агентов"
            hint="PO, Architect, Reviewer, Overseer"
            value={modelPm}
            onChange={setModelPm}
            models={models}
            disabled={isRunning}
          />
          <ModelSelect
            label="Модель для специалистов"
            hint="Backend / Frontend / DevOps / Presenter / Documenter / Merge Resolver"
            value={modelSpec}
            onChange={setModelSpec}
            models={models}
            disabled={isRunning}
          />
          <div className="space-y-1.5">
            <Label>Permission mode</Label>
            <select
              value={permMode}
              onChange={(e) => setPermMode(e.target.value as PermissionMode)}
              disabled={isRunning}
              className="w-full h-9 rounded-md border border-border bg-background px-3 text-sm disabled:opacity-50"
            >
              <option value="bypassPermissions">bypassPermissions (полная автономия)</option>
              <option value="acceptEdits">acceptEdits (Bash блокируется)</option>
              <option value="default">default (спрашивать каждый раз)</option>
            </select>
          </div>
          {error && (
            <div className="text-xs text-destructive border border-destructive/30 bg-destructive/10 rounded-md px-3 py-2">
              {error}
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose} disabled={submitting}>Отмена</Button>
          <Button disabled={!canSave} onClick={submit}>
            {submitting ? "Сохраняю…" : "Сохранить"}
          </Button>
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
  disabled,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (v: string) => void;
  models: ReadonlyArray<ModelSuggestion>;
  disabled?: boolean;
}) {
  const listId = `settings-models-${label.replace(/\s+/g, "-")}`;
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <Input
        list={listId}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        autoComplete="off"
        disabled={disabled}
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
