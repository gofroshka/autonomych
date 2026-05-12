import { useEffect, useMemo, useState } from "react";
import {
  ReactFlow, Background, BackgroundVariant, Controls, MiniMap, Position, Handle,
  useNodesState, useEdgesState, type Node, type Edge, type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Boxes, Cpu, Monitor, Server, type LucideIcon } from "lucide-react";
import type { TaskRow } from "../types";
import { cn } from "../lib/cn";
import { formatDuration } from "../lib/format";

/** How often to re-render the live elapsed timer for in-progress tasks. */
const TICK_MS = 1000;

const ROLE_META: Record<string, { label: string; icon: LucideIcon; color: string }> = {
  specialist_backend: { label: "Backend", icon: Server, color: "text-info" },
  specialist_frontend: { label: "Frontend", icon: Monitor, color: "text-success" },
  specialist_devops: { label: "DevOps", icon: Boxes, color: "text-warning" },
};

const STATUS_STYLE: Record<TaskRow["status"], string> = {
  pending: "border-border bg-card",
  in_progress: "border-primary/60 bg-primary/10 shadow-[0_0_0_3px_hsl(var(--primary)/0.15)]",
  done: "border-success/40 bg-success/10",
  failed: "border-destructive/50 bg-destructive/10",
  skipped: "border-border bg-card/30 opacity-60",
};

const STATUS_LABEL: Record<TaskRow["status"], string> = {
  pending: "Ждёт", in_progress: "В работе", done: "Готово", failed: "Упало", skipped: "Пропущено",
};

const COLUMN_WIDTH = 280;
const ROW_HEIGHT = 140;
const NODE_WIDTH = 230;

const ROLE_ORDER: Record<string, number> = {
  specialist_devops: 0, specialist_backend: 1, specialist_frontend: 2,
};

interface TaskNodeData extends Record<string, unknown> {
  task: TaskRow;
  /** Cached "now" timestamp used by in-progress nodes for the live timer.
   *  Threaded via node data so we can update it without rebuilding the layout. */
  now: number;
}
type TaskNode = Node<TaskNodeData, "task">;

/**
 * Returns elapsed milliseconds for a task, or null if not yet applicable.
 *
 * - `in_progress`: from `started_at` (fallback to `created_at`) until `now`.
 * - terminal states (`done`/`failed`/`skipped`): from `started_at` until `ended_at`.
 * - everything else (`pending`): null — nothing to time yet.
 */
function elapsedFor(task: TaskRow, now: number): number | null {
  const start = task.started_at ?? task.created_at;
  if (task.status === "in_progress") {
    return Math.max(0, now - start);
  }
  if (
    task.ended_at &&
    (task.status === "done" || task.status === "failed" || task.status === "skipped")
  ) {
    return Math.max(0, task.ended_at - start);
  }
  return null;
}

function computeLevels(tasks: TaskRow[]): Map<string, number> {
  const byArchId = new Map<string, TaskRow>();
  for (const t of tasks) if (t.architect_id) byArchId.set(t.architect_id, t);
  const levels = new Map<string, number>();
  const visit = (t: TaskRow, stack: Set<string>): number => {
    const cached = levels.get(t.id);
    if (cached !== undefined) return cached;
    // Cycle guard: any task that re-enters its own subgraph is treated as
    // level 0 so we don't recurse forever.
    if (stack.has(t.id)) return 0;
    stack.add(t.id);
    let max = 0;
    for (const dArch of t.depends_on ?? []) {
      const dep = byArchId.get(dArch);
      if (dep) max = Math.max(max, visit(dep, stack) + 1);
    }
    stack.delete(t.id);
    levels.set(t.id, max);
    return max;
  };
  for (const t of tasks) visit(t, new Set());
  return levels;
}

function buildLayout(
  tasks: TaskRow[],
  now: number
): { nodes: TaskNode[]; edges: Edge[] } {
  if (tasks.length === 0) return { nodes: [], edges: [] };
  const byArchId = new Map<string, TaskRow>();
  for (const t of tasks) if (t.architect_id) byArchId.set(t.architect_id, t);
  const levels = computeLevels(tasks);
  const perLevel = new Map<number, TaskRow[]>();
  for (const t of tasks) {
    const l = levels.get(t.id) ?? 0;
    const arr = perLevel.get(l) ?? [];
    arr.push(t);
    perLevel.set(l, arr);
  }
  for (const [, arr] of perLevel) {
    arr.sort(
      (a, b) =>
        (ROLE_ORDER[a.role] ?? 9) - (ROLE_ORDER[b.role] ?? 9) || a.created_at - b.created_at
    );
  }
  const nodes: TaskNode[] = tasks.map((t) => {
    const lvl = levels.get(t.id) ?? 0;
    const column = perLevel.get(lvl) ?? [];
    const idx = column.findIndex((x) => x.id === t.id);
    return {
      id: t.id,
      type: "task",
      position: { x: lvl * COLUMN_WIDTH, y: idx * ROW_HEIGHT },
      data: { task: t, now },
      sourcePosition: Position.Right,
      targetPosition: Position.Left,
    };
  });
  const edges: Edge[] = tasks.flatMap((t) =>
    (t.depends_on ?? [])
      .map((dArch) => {
        const dep = byArchId.get(dArch);
        if (!dep) return null;
        return {
          id: `${dep.id}->${t.id}`,
          source: dep.id,
          target: t.id,
          type: "smoothstep",
          animated: t.status === "in_progress",
          style: {
            stroke: t.status === "failed"
              ? "hsl(var(--destructive) / 0.6)"
              : "hsl(var(--muted-foreground) / 0.55)",
            strokeWidth: 1.5,
          },
        } as Edge;
      })
      .filter((x): x is Edge => x !== null)
  );
  return { nodes, edges };
}

function TaskFlowNode({ data }: NodeProps<TaskNode>) {
  const task = data.task;
  const cls = STATUS_STYLE[task.status] ?? STATUS_STYLE.pending;
  const ms = elapsedFor(task, data.now);
  const meta = ROLE_META[task.role] ?? { label: task.role, icon: Cpu, color: "text-muted-foreground" };
  const RoleIcon = meta.icon;
  const isRunning = task.status === "in_progress";
  return (
    <div
      className={cn(
        "rounded-lg border p-3 transition-all flex flex-col gap-2 min-h-[100px] cursor-grab active:cursor-grabbing",
        cls
      )}
      style={{ width: NODE_WIDTH }}
    >
      <Handle type="target" position={Position.Left} className="!bg-muted-foreground/60 !w-2 !h-2 !border-0" />
      <Handle type="source" position={Position.Right} className="!bg-muted-foreground/60 !w-2 !h-2 !border-0" />
      <div className="flex items-center gap-1.5">
        <RoleIcon className={cn("h-3 w-3", meta.color)} />
        <span className="text-[9px] font-semibold uppercase tracking-wider text-muted-foreground">{meta.label}</span>
        <div className="flex-1" />
        <StatusDot status={task.status} />
      </div>
      <div
        className={cn("text-[13px] font-medium leading-snug flex-1 text-foreground", task.status === "skipped" && "line-through")}
        style={{ display: "-webkit-box", WebkitLineClamp: 3, WebkitBoxOrient: "vertical", overflow: "hidden" }}
      >
        {task.title || "(без заголовка)"}
      </div>
      <div className="flex items-center justify-between text-[10px] text-muted-foreground/80 font-mono">
        <span>{STATUS_LABEL[task.status] ?? task.status}</span>
        {ms !== null && (
          <span
            className={cn(
              "tabular-nums",
              isRunning && "text-primary"
            )}
            title={isRunning ? "Идёт прямо сейчас" : undefined}
          >
            {formatDuration(ms)}
          </span>
        )}
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: TaskRow["status"] }) {
  const base = "h-1.5 w-1.5 rounded-full shrink-0";
  if (status === "in_progress") return <span className={cn(base, "bg-primary animate-soft-pulse")} />;
  if (status === "done") return <span className={cn(base, "bg-success")} />;
  if (status === "failed") return <span className={cn(base, "bg-destructive")} />;
  if (status === "skipped") return <span className={cn(base, "bg-muted-foreground/40")} />;
  return <span className={cn(base, "bg-muted-foreground/60")} />;
}

const nodeTypes = { task: TaskFlowNode };

export function TaskGraph({ tasks }: { tasks: TaskRow[] }) {
  // Live wall clock for the in-progress elapsed-time display. Ticks only when
  // at least one task is actually running, so an idle graph is render-quiet.
  const [now, setNow] = useState(() => Date.now());
  const hasRunning = useMemo(
    () => tasks.some((t) => t.status === "in_progress"),
    [tasks]
  );

  // Re-layout only when the graph goes from empty to non-empty (or vice
  // versa). buildLayout is otherwise driven by status / structural changes
  // pushed in below.
  const initial = useMemo(
    () => buildLayout(tasks, now),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tasks.length === 0]
  );
  const [nodes, setNodes, onNodesChange] = useNodesState<TaskNode>(initial.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>(initial.edges);

  // Patch node data + edges when the task list changes (status, new nodes,
  // dependencies). Positions are preserved from the user's drag state.
  useEffect(() => {
    const fresh = buildLayout(tasks, now);
    setNodes((current) => {
      const byId = new Map(current.map((n) => [n.id, n]));
      return fresh.nodes.map((n) => {
        const existing = byId.get(n.id);
        return existing ? { ...n, position: existing.position } : n;
      });
    });
    setEdges(fresh.edges);
    // `now` is intentionally not a dep here — it has its own effect below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tasks, setNodes, setEdges]);

  // Cheap timer-only update: refresh just the `now` field in each node's
  // data. No relayout, no re-deriving edges.
  useEffect(() => {
    setNodes((current) =>
      current.map((n) => ({ ...n, data: { ...n.data, now } }))
    );
  }, [now, setNodes]);

  // Drive the tick only while something is running.
  useEffect(() => {
    if (!hasRunning) return;
    const id = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(id);
  }, [hasRunning]);

  if (tasks.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center text-center h-full text-muted-foreground">
        <Cpu className="h-7 w-7 mb-3 opacity-30" />
        <p className="text-sm">Архитектор ещё не разложил задачи.</p>
        <p className="text-xs mt-1 opacity-70">Они появятся как только Architect закончит планирование.</p>
      </div>
    );
  }

  return (
    <div className="h-full w-full relative">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
        minZoom={0.3}
        maxZoom={2}
        proOptions={{ hideAttribution: true }}
        nodesDraggable
        nodesConnectable={false}
        panOnDrag
        elementsSelectable
        defaultEdgeOptions={{ type: "smoothstep", style: { strokeWidth: 1.5 } }}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="hsl(var(--muted-foreground) / 0.18)" />
        <MiniMap
          pannable zoomable
          maskColor="hsl(var(--background) / 0.7)"
          nodeBorderRadius={6}
          nodeColor={(n) => {
            const t = (n.data as TaskNodeData | undefined)?.task;
            if (!t) return "hsl(var(--muted-foreground))";
            if (t.status === "in_progress") return "hsl(var(--primary))";
            if (t.status === "done") return "hsl(var(--success))";
            if (t.status === "failed") return "hsl(var(--destructive))";
            return "hsl(var(--muted-foreground) / 0.4)";
          }}
          className="!bg-card !border-border"
        />
        <Controls showInteractive={false} className="[&>button]:!bg-card [&>button]:!border-border [&>button]:!text-foreground [&>button:hover]:!bg-accent" />
      </ReactFlow>
    </div>
  );
}
