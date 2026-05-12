import { useEffect, useMemo } from "react";
import {
  ReactFlow, Background, BackgroundVariant, Controls, MiniMap, Position, Handle,
  useNodesState, useEdgesState, type Node, type Edge, type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Boxes, Cpu, Monitor, Server, type LucideIcon } from "lucide-react";
import type { TaskRow } from "../types";
import { cn } from "../lib/cn";
import { formatDuration } from "../lib/format";

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

interface TaskNodeData extends Record<string, unknown> { task: TaskRow }
type TaskNode = Node<TaskNodeData, "task">;

function computeLevels(tasks: TaskRow[]): Map<string, number> {
  const byArchId = new Map<string, TaskRow>();
  for (const t of tasks) if (t.architect_id) byArchId.set(t.architect_id, t);
  const levels = new Map<string, number>();
  const visit = (t: TaskRow, stack: Set<string>): number => {
    if (levels.has(t.id)) return levels.get(t.id)!;
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

function buildLayout(tasks: TaskRow[]): { nodes: TaskNode[]; edges: Edge[] } {
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
      data: { task: t },
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
  const ms = task.ended_at && task.created_at ? task.ended_at - task.created_at : null;
  const meta = ROLE_META[task.role] ?? { label: task.role, icon: Cpu, color: "text-muted-foreground" };
  const RoleIcon = meta.icon;
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
        {ms !== null && task.status === "done" && <span>{formatDuration(ms)}</span>}
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
  const initial = useMemo(() => buildLayout(tasks), [tasks.length === 0]);
  const [nodes, setNodes, onNodesChange] = useNodesState<TaskNode>(initial.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>(initial.edges);

  useEffect(() => {
    const fresh = buildLayout(tasks);
    setNodes((current) => {
      const byId = new Map(current.map((n) => [n.id, n]));
      return fresh.nodes.map((n) => {
        const existing = byId.get(n.id);
        return existing ? { ...n, position: existing.position } : n;
      });
    });
    setEdges(fresh.edges);
  }, [tasks, setNodes, setEdges]);

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
