import { Loader2 } from "lucide-react";
import type { ConductorState } from "../types";
import { Badge } from "./ui/badge";

const META: Record<
  ConductorState,
  { label: string; variant: "default" | "primary" | "success" | "warning" | "info" | "destructive"; pulse?: boolean }
> = {
  IDLE: { label: "Idle", variant: "default" },
  RUNNING: { label: "Running", variant: "primary", pulse: true },
  WRAPPING_UP: { label: "Wrapping up", variant: "warning", pulse: true },
  PREPARING_PREVIEW: { label: "Готовим демо", variant: "info", pulse: true },
  PRESENTING: { label: "Presenting", variant: "info" },
  RESUMING: { label: "Resuming", variant: "primary", pulse: true },
  PAUSED: { label: "Paused", variant: "warning" },
  ERROR: { label: "Error", variant: "destructive" },
};

export function StateBadge({ state }: { state: ConductorState }) {
  const m = META[state];
  return (
    <Badge variant={m.variant}>
      {m.pulse && <Loader2 className="h-2.5 w-2.5 animate-spin" />}
      {m.label}
    </Badge>
  );
}
