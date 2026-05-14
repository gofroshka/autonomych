// Translate raw EventRow events into UI-friendly entries for the activity
// log. The payload is now a typed discriminated union (see types.ts) so
// every variant is handled with exhaustive pattern matching.

import type { AgentRole, EventPayload, EventRow } from "../types";

export interface HumanEvent {
  id: string;
  ts: number;
  role: AgentRole | null;
  action: string;
  target?: string;
  detail?: string;
  kind:
    | "info"
    | "thinking"
    | "read"
    | "write"
    | "shell"
    | "search"
    | "result"
    | "error"
    | "lifecycle"
    | "directive"
    | "question"
    | "answer";
}

export const ROLE_LABEL_RU: Record<AgentRole, string> = {
  product_owner: "Product Owner",
  architect: "Architect",
  specialist_backend: "Backend",
  specialist_frontend: "Frontend",
  specialist_devops: "DevOps",
  reviewer: "Reviewer",
  blocker_reviewer: "Blocker Reviewer",
  overseer: "Overseer",
  presenter: "Presenter",
  merge_resolver: "Merge Resolver",
  documenter: "Documenter",
};

function shortPath(p: string): string {
  if (!p) return "";
  const wt = p.match(/\.autonomych\/worktrees\/[^/]+\/(.*)$/);
  if (wt) return wt[1];
  const parts = p.split("/").filter(Boolean);
  if (parts.length > 4) return parts.slice(-3).join("/");
  return p;
}

function summarizeBash(command: string): { action: string; target: string } {
  const cmd = command.trim();
  const first = cmd.split(/\s+/, 1)[0];
  if (/^npm\s+install|^npm\s+i\b|^pnpm\s+install|^yarn\s+install/.test(cmd))
    return { action: "Устанавливает зависимости", target: cmd.slice(0, 80) };
  if (/^npx\s/.test(cmd)) {
    const pkg = cmd.replace(/^npx\s+(-y\s+)?/, "").split(/\s+/, 1)[0];
    return { action: `Запускает ${pkg}`, target: cmd.slice(0, 100) };
  }
  if (/^npm\s+run|^pnpm\s+run|^yarn\s+run/.test(cmd)) {
    const script = cmd.replace(/^.*?run\s+/, "").split(/\s+/, 1)[0];
    return { action: `Запускает скрипт ${script}`, target: "" };
  }
  if (/^mkdir\s/.test(cmd)) {
    const p = cmd.replace(/^mkdir\s+(-p\s+)?/, "").split(/\s+/, 1)[0];
    return { action: "Создаёт папку", target: shortPath(p) };
  }
  if (/^git\s/.test(cmd)) return { action: "Работает с git", target: cmd.slice(0, 80) };
  if (/prisma\s/.test(cmd)) return { action: "Работает с Prisma", target: cmd.slice(0, 100) };
  if (/^docker\b/.test(cmd)) return { action: "Работает с Docker", target: cmd.slice(0, 100) };
  if (/^ls\b|^pwd\b|^cat\s/.test(cmd))
    return { action: "Изучает структуру", target: cmd.slice(0, 80) };
  return { action: `Выполняет: ${first}`, target: cmd.slice(0, 100) };
}

/** Convert a typed agent_tool_use into the activity log entry. */
function fromToolUse(
  base: { id: string; ts: number; role: AgentRole | null },
  tool: string,
  input: Record<string, unknown>
): HumanEvent {
  switch (tool) {
    case "Bash": {
      const s = summarizeBash(String(input.command ?? ""));
      return { ...base, kind: "shell", action: s.action, target: s.target };
    }
    case "Write":
      return { ...base, kind: "write", action: "Создаёт файл", target: shortPath(String(input.file_path ?? "")) };
    case "Edit":
      return { ...base, kind: "write", action: "Редактирует файл", target: shortPath(String(input.file_path ?? "")) };
    case "Read":
      return { ...base, kind: "read", action: "Читает файл", target: shortPath(String(input.file_path ?? "")) };
    case "Glob":
      return { ...base, kind: "search", action: "Ищет файлы", target: String(input.pattern ?? "") };
    case "Grep":
      return { ...base, kind: "search", action: "Ищет в коде", target: String(input.pattern ?? "") };
    default:
      return { ...base, kind: "info", action: `Вызывает ${tool}` };
  }
}

export function humanize(ev: EventRow): HumanEvent | null {
  const base = { id: ev.id, ts: ev.ts, role: ev.agent_role };
  const p: EventPayload = ev.payload;

  switch (p.type) {
    // ---- Iteration lifecycle ----
    case "iteration_start":
      return {
        ...base,
        kind: "lifecycle",
        action: `Старт итерации #${p.number}`,
        target: p.mode === "wrapup" ? "режим стабилизации" : "",
      };
    case "iteration_end":
      return {
        ...base,
        kind: "lifecycle",
        action: "Итерация завершена",
        target: p.demoable ? "готово к демо" : p.demoable === false ? "есть проблемы" : "",
      };
    case "iteration_error":
      return { ...base, kind: "error", action: "Итерация упала", target: p.error.slice(0, 200) };

    // ---- State + directives ----
    case "state_change":
      if (["IDLE", "ERROR", "PRESENTING", "WRAPPING_UP"].includes(p.state)) {
        return { ...base, role: null, kind: "directive", action: `Состояние: ${p.state}` };
      }
      return null;
    case "wrap_up_requested":
      return { ...base, role: null, kind: "directive", action: "Пользователь запросил остановку" };
    case "presentation_only":
      return { ...base, role: null, kind: "directive", action: "Сразу к демо без новой итерации" };
    case "resume_for_preview":
      return { ...base, role: null, kind: "directive", action: `Резюм для демо (#${p.iteration})` };
    case "resumed":
      return { ...base, role: null, kind: "directive", action: "Пользователь продолжил цикл" };

    // ---- Agent boundaries ----
    case "agent_start":
      return { ...base, role: p.role, kind: "info", action: "Включился" };
    case "agent_end":
      return {
        ...base,
        role: p.role,
        kind: "info",
        action: "Закончил",
        target: p.duration_ms ? `за ${(p.duration_ms / 1000).toFixed(1)}с` : "",
      };
    case "agent_error":
      return { ...base, role: p.role, kind: "error", action: "Ошибка агента", target: p.message.slice(0, 200) };

    // ---- Agent activity ----
    case "agent_message": {
      // Suppress noisy planner / reviewer chatter — those agents return JSON
      // we already parsed; their free-form text would just confuse the log.
      if (
        p.role === "product_owner" ||
        p.role === "architect" ||
        p.role === "reviewer" ||
        p.role === "blocker_reviewer"
      ) {
        return null;
      }
      const text = p.text.trim();
      if (!text) return null;
      const stripped = text.replace(/^```\w*\s*\n?/, "").replace(/```\s*$/, "").trim();
      if (/^[{[]/.test(stripped)) return null;
      return {
        ...base,
        role: p.role,
        kind: "thinking",
        action: text.length > 180 ? text.slice(0, 180) + "…" : text,
        detail: text,
      };
    }
    case "agent_tool_use":
      return fromToolUse({ ...base, role: p.role }, p.tool, (p.input as Record<string, unknown>) ?? {});
    case "agent_tool_result":
      if (p.is_error) {
        return {
          ...base,
          role: p.role,
          kind: "error",
          action: "Ошибка инструмента",
          target: p.content.split("\n", 1)[0].slice(0, 140),
          detail: p.content,
        };
      }
      return null;

    // ---- ask_user routing ----
    case "ask_user_invoked":
      return { ...base, kind: "question", action: "Специалист упёрся", target: p.question.slice(0, 200) };
    case "question_asked":
      return { ...base, kind: "question", action: "Вопрос пользователю", target: p.question.slice(0, 200) };
    case "question_answered":
      return {
        ...base,
        kind: "answer",
        action: p.resolution === "user" ? "Пользователь ответил" : "Reviewer ответил сам",
        target: p.answer_preview.slice(0, 200),
      };

    // ---- Iteration stages (diagnostics) ----
    case "po_done":
      return { ...base, role: "product_owner", kind: "info", action: "Сформировал backlog", target: p.theme };
    case "po_skipped_resume":
      return { ...base, role: "product_owner", kind: "info", action: "Тема уже есть, продолжаем", target: p.theme };
    case "arch_done":
      return { ...base, role: "architect", kind: "info", action: `Декомпозировал на ${p.tasks} задач` };
    case "arch_skipped_resume":
      return { ...base, role: "architect", kind: "info", action: `Задачи уже есть (${p.tasks})` };
    case "reviewer_failed":
      return { ...base, role: "reviewer", kind: "error", action: "Ревью не получено", target: p.error.slice(0, 200) };
    case "resume_iteration":
      return {
        ...base,
        kind: "lifecycle",
        action: `Возобновление итерации #${p.number}`,
        target: `${p.tasks_pending} незавершённых задач`,
      };

    // ---- Wave runner ----
    case "wave_started":
      return { ...base, role: null, kind: "info", action: `Запускает волну из ${p.size}` };
    case "tasks_skipped":
      return { ...base, role: null, kind: "info", action: `Пропустил ${p.count}`, target: p.reason };
    case "graph_deadlock":
      return { ...base, role: null, kind: "error", action: "Тупик: сироты в графе зависимостей" };

    // ---- Worktree / merge ----
    case "worktree_failed":
      return { ...base, kind: "error", action: "Не удалось создать worktree", target: p.error.slice(0, 200) };
    case "merge_failed":
      return {
        ...base,
        kind: "error",
        action: p.conflict ? "Конфликт мерджа" : "Не удалось смерджить",
        target: p.message.split("\n", 1)[0].slice(0, 200),
      };
    case "merge_conflict":
      return {
        ...base,
        kind: "info",
        action: `Конфликт при rebase: ${p.files.length} файл(ов) — зову Merge Resolver`,
        target: p.files.slice(0, 3).join(", ") + (p.files.length > 3 ? "…" : ""),
      };
    case "merge_resolved":
      return {
        ...base,
        kind: p.ok ? "info" : "error",
        action: p.ok ? "Merge Resolver разрешил конфликт" : "Merge Resolver сдался",
        target: p.summary.slice(0, 200),
      };
    case "docs_updated":
      return {
        ...base,
        role: "documenter",
        kind: "info",
        action: "Documenter обновил документацию",
        target: p.summary.slice(0, 200),
      };

    // ---- Preview lifecycle ----
    case "preview_prep_done":
      return { ...base, role: null, kind: "info", action: "Demo готово" };
    case "preview_prep_failed":
      return { ...base, role: null, kind: "error", action: "Demo не запустилось", target: p.error.slice(0, 200) };
    case "preview_shutdown_done":
      return { ...base, role: null, kind: "info", action: "Demo остановлено" };
    case "preview_shutdown_skipped":
      return null;

    // ---- Loop errors ----
    case "backoff":
      return {
        ...base,
        role: null,
        kind: "info",
        action: `Пауза ${(p.duration_ms / 1000).toFixed(1)}с после ${p.consecutive} падений`,
      };
    case "too_many_failures":
      return {
        ...base,
        role: null,
        kind: "error",
        action: `${p.consecutive} падений подряд — стоп`,
      };
    case "loop_error":
      return { ...base, role: null, kind: "error", action: "Сбой цикла", target: p.error.slice(0, 200) };

    // ---- Provider rate-limit cooldown ----
    case "cooldown_started": {
      const ts = new Date(p.retry_at_ms);
      const hh = ts.getHours().toString().padStart(2, "0");
      const mm = ts.getMinutes().toString().padStart(2, "0");
      return {
        ...base,
        role: null,
        kind: "directive",
        action: `Лимит провайдера — продолжим в ${hh}:${mm}`,
        target: p.reason.slice(0, 200),
      };
    }
    case "cooldown_ended":
      return {
        ...base,
        role: null,
        kind: "info",
        action: p.skipped_by_user ? "Кулдаун пропущен — продолжаем" : "Кулдаун закончился — продолжаем",
      };
    case "cooldown_cancelled":
      return { ...base, role: null, kind: "directive", action: "Кулдаун отменён пользователем" };

    default: {
      // Exhaustiveness check — never narrows in practice; if a new variant
      // is added on the Rust side without updating this switch, TS will fail
      // compilation here.
      const _exhaustive: never = p;
      void _exhaustive;
      return null;
    }
  }
}
