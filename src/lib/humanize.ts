import type { AgentRole, EventRow } from "../types";

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
};

function safeJson<T = any>(s: string): T | null {
  try {
    return JSON.parse(s) as T;
  } catch {
    return null;
  }
}

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

export function humanize(ev: EventRow): HumanEvent | null {
  const payload = safeJson<any>(ev.payload) ?? {};
  const base = { id: ev.id, ts: ev.ts, role: ev.agent_role };

  switch (ev.type) {
    case "iteration_start":
      return { ...base, kind: "lifecycle", action: `Старт итерации #${payload.number}`, target: payload.mode === "wrapup" ? "режим стабилизации" : "" };
    case "iteration_end":
      return { ...base, kind: "lifecycle", action: "Итерация завершена", target: payload.demoable ? "готово к демо" : payload.demoable === false ? "есть проблемы" : "" };
    case "state_change":
      if (["IDLE", "ERROR", "PRESENTING", "WRAPPING_UP"].includes(payload.state)) {
        return { ...base, role: null, kind: "directive", action: `Состояние: ${payload.state}` };
      }
      return null;
    case "directive":
      if (payload.kind === "wrap_up_requested")
        return { ...base, role: null, kind: "directive", action: "Пользователь запросил остановку" };
      if (payload.kind === "resume")
        return { ...base, role: null, kind: "directive", action: "Пользователь продолжил цикл" };
      return { ...base, role: null, kind: "directive", action: String(payload.kind ?? "") };
    case "agent_start":
      return { ...base, kind: "info", action: "Включился" };
    case "agent_end":
      return { ...base, kind: "info", action: "Закончил", target: payload.durationMs ? `за ${(payload.durationMs / 1000).toFixed(1)}с` : "" };
    case "agent_message": {
      if (ev.agent_role === "product_owner" || ev.agent_role === "architect" || ev.agent_role === "reviewer" || ev.agent_role === "blocker_reviewer") {
        return null;
      }
      const text = String(payload.text ?? "").trim();
      if (!text) return null;
      const stripped = text.replace(/^```\w*\s*\n?/, "").replace(/```\s*$/, "").trim();
      if (/^[{[]/.test(stripped)) return null;
      return { ...base, kind: "thinking", action: text.length > 180 ? text.slice(0, 180) + "…" : text, detail: text };
    }
    case "agent_tool_use": {
      const tool = payload.tool as string;
      const input = payload.input ?? {};
      if (tool === "Bash") {
        const s = summarizeBash(String(input.command ?? ""));
        return { ...base, kind: "shell", action: s.action, target: s.target };
      }
      if (tool === "Write") return { ...base, kind: "write", action: "Создаёт файл", target: shortPath(String(input.file_path ?? "")) };
      if (tool === "Edit") return { ...base, kind: "write", action: "Редактирует файл", target: shortPath(String(input.file_path ?? "")) };
      if (tool === "Read") return { ...base, kind: "read", action: "Читает файл", target: shortPath(String(input.file_path ?? "")) };
      if (tool === "Glob") return { ...base, kind: "search", action: "Ищет файлы", target: String(input.pattern ?? "") };
      if (tool === "Grep") return { ...base, kind: "search", action: "Ищет в коде", target: String(input.pattern ?? "") };
      return { ...base, kind: "info", action: `Вызывает ${tool}` };
    }
    case "agent_tool_result": {
      if (payload.is_error) {
        const content = String(payload.content ?? "");
        return { ...base, kind: "error", action: "Ошибка инструмента", target: content.split("\n", 1)[0].slice(0, 140), detail: content };
      }
      return null;
    }
    case "agent_error":
      return { ...base, kind: "error", action: "Ошибка агента", target: String(payload.message ?? payload.error ?? "").slice(0, 200) };
    case "question_asked":
      return { ...base, kind: "question", action: "Вопрос пользователю", target: String(payload.question ?? "").slice(0, 200) };
    case "question_answered":
      return { ...base, kind: "answer", action: payload.resolution === "user" ? "Пользователь ответил" : "Reviewer ответил сам", target: String(payload.answer ?? "").slice(0, 200) };
    case "system": {
      if (payload.stage === "po_done") return { ...base, role: "product_owner", kind: "info", action: "Сформировал backlog", target: payload.theme };
      if (payload.stage === "arch_done") return { ...base, role: "architect", kind: "info", action: `Декомпозировал на ${payload.tasks} задач` };
      if (payload.wave_size) return { ...base, role: null, kind: "info", action: `Запускает волну из ${payload.wave_size}` };
      if (payload.error) return { ...base, kind: "error", action: "Ошибка системы", target: String(payload.error).slice(0, 200) };
      return null;
    }
    default:
      return null;
  }
}
