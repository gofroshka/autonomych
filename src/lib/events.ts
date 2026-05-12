// Event metadata and helpers used by the live UI feed.
//
// "Structural" events are the ones that change something the user can see on
// the dashboard (state, iteration, agent boundary, question lifecycle) — and
// therefore should trigger an immediate snapshot refresh. Everything else
// (assistant text, tool use/result, low-level diagnostics) is throttled into
// a debounced refresh.

import type { EventPayload, EventType } from "../types";

export const STRUCTURAL_EVENTS = new Set<EventType>([
  "state_change",
  "iteration_start",
  "iteration_end",
  "iteration_error",
  "agent_start",
  "agent_end",
  "agent_error",
  "question_asked",
  "question_answered",
]);

export function isStructural(payload: EventPayload): boolean {
  return STRUCTURAL_EVENTS.has(payload.type);
}
