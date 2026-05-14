// Curated suggestions for each backend's model picker. Free-text input is
// always allowed — this just powers the <datalist> hints in the modals.
//
// Codex CLI's "current" models are the gpt-5.x series shown by `codex` when
// you bring up its model picker. Older names like `gpt-5`, `gpt-5-codex`,
// `o3` are accepted by `codex -m ...` flag but legacy ChatGPT-auth tarifs
// don't grant access to them — keep the suggestions in step with what
// `codex model` actually offers, otherwise users hit
// "model is not supported when using Codex with a ChatGPT account" right
// after `turn.started`.

import type { AgentBackend } from "../types";

export interface ModelSuggestion {
  id: string;
  label: string;
  hint: string;
}

export const CLAUDE_MODELS: ReadonlyArray<ModelSuggestion> = [
  { id: "opus", label: "Opus", hint: "самая умная и дорогая" },
  { id: "sonnet", label: "Sonnet", hint: "баланс цена/качество" },
  { id: "haiku", label: "Haiku", hint: "быстрая и дешёвая" },
];

export const CODEX_MODELS: ReadonlyArray<ModelSuggestion> = [
  { id: "gpt-5.5", label: "gpt-5.5", hint: "frontier — сложный код и research" },
  { id: "gpt-5.4", label: "gpt-5.4", hint: "сильная everyday-модель" },
  { id: "gpt-5.4-mini", label: "gpt-5.4-mini", hint: "дешёвая и быстрая" },
  { id: "gpt-5.3-codex", label: "gpt-5.3-codex", hint: "coding-tuned, для специалистов" },
  { id: "gpt-5.2", label: "gpt-5.2", hint: "для длинных задач" },
];

export const BACKEND_DEFAULTS: Record<AgentBackend, { pm: string; spec: string }> = {
  claude_code: { pm: "opus", spec: "sonnet" },
  codex: { pm: "gpt-5.4", spec: "gpt-5.3-codex" },
};

export function modelsFor(backend: AgentBackend): ReadonlyArray<ModelSuggestion> {
  return backend === "claude_code" ? CLAUDE_MODELS : CODEX_MODELS;
}
