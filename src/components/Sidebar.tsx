import { useEffect, useRef, useState } from "react";
import { FolderOpen, MoreVertical, Pencil, Plus, Trash2 } from "lucide-react";
import type { ProjectRow } from "../types";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Separator } from "./ui/separator";
import { cn } from "../lib/cn";

export function Sidebar({
  projects,
  activeId,
  onSelect,
  onNew,
  onDelete,
  onRename,
}: {
  projects: ProjectRow[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (project: ProjectRow) => void;
  onRename: (project: ProjectRow) => void;
}) {
  return (
    <aside className="flex flex-col w-[280px] shrink-0 border-r border-border bg-card/30">
      <div className="flex items-center justify-between px-4 h-12 shrink-0">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
          Проекты
        </span>
        <Button variant="ghost" size="sm" className="h-7 px-2 gap-1.5" onClick={onNew}>
          <Plus className="h-3.5 w-3.5" />
          Новый
        </Button>
      </div>
      <Separator />
      <div className="flex-1 overflow-y-auto scrollbar-thin px-2 py-2 space-y-0.5">
        {projects.length === 0 && (
          <div className="text-center py-12 text-xs text-muted-foreground/70">
            Создай первый проект
          </div>
        )}
        {projects.map((p) => (
          <ProjectItem
            key={p.id}
            project={p}
            isActive={p.id === activeId}
            onSelect={() => onSelect(p.id)}
            onDelete={() => onDelete(p)}
            onRename={() => onRename(p)}
            onOpenFolder={() => api.openExternal(p.root_path)}
          />
        ))}
      </div>
    </aside>
  );
}

function ProjectItem({
  project,
  isActive,
  onSelect,
  onDelete,
  onRename,
  onOpenFolder,
}: {
  project: ProjectRow;
  isActive: boolean;
  onSelect: () => void;
  onDelete: () => void;
  onRename: () => void;
  onOpenFolder: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menuOpen) return;
    const onClick = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [menuOpen]);

  const running =
    project.state === "RUNNING" ||
    project.state === "WRAPPING_UP" ||
    project.state === "RESUMING" ||
    project.state === "PREPARING_PREVIEW";

  return (
    <div
      ref={ref}
      onClick={onSelect}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuOpen(true);
      }}
      className={cn(
        "group relative rounded-md px-3 py-2.5 cursor-pointer border border-transparent transition-colors",
        isActive ? "bg-accent border-border" : "hover:bg-accent/60"
      )}
    >
      <div className="flex items-start gap-2">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            {running && (
              <span className="h-1.5 w-1.5 rounded-full bg-primary animate-soft-pulse shrink-0" />
            )}
            <span className="text-sm font-medium truncate">{project.name}</span>
          </div>
          <div className="text-[11px] text-muted-foreground line-clamp-2 mt-0.5 leading-snug">
            {project.idea}
          </div>
        </div>
        <button
          className={cn(
            "shrink-0 p-1 rounded transition-opacity",
            isActive ? "opacity-70" : "opacity-0 group-hover:opacity-70",
            "hover:opacity-100 hover:bg-secondary"
          )}
          onClick={(e) => {
            e.stopPropagation();
            setMenuOpen((v) => !v);
          }}
        >
          <MoreVertical className="h-3.5 w-3.5" />
        </button>
      </div>
      {menuOpen && (
        <div
          onClick={(e) => e.stopPropagation()}
          className="absolute top-9 right-1 z-30 w-52 rounded-md border border-border bg-popover py-1 shadow-xl"
        >
          <MenuItem icon={<FolderOpen className="h-3.5 w-3.5" />} onClick={() => { setMenuOpen(false); onOpenFolder(); }}>
            Открыть папку
          </MenuItem>
          <MenuItem
            icon={<Pencil className="h-3.5 w-3.5" />}
            disabled={running}
            onClick={() => { setMenuOpen(false); onRename(); }}
          >
            {running ? "Изменить идею (после стопа)" : "Изменить идею"}
          </MenuItem>
          <Separator className="my-1" />
          <MenuItem
            danger
            icon={<Trash2 className="h-3.5 w-3.5" />}
            onClick={() => { setMenuOpen(false); onDelete(); }}
          >
            Удалить
          </MenuItem>
        </div>
      )}
    </div>
  );
}

function MenuItem({
  children,
  icon,
  onClick,
  danger,
  disabled,
}: {
  children: React.ReactNode;
  icon: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "w-full text-left px-3 py-1.5 text-xs flex items-center gap-2 transition-colors disabled:opacity-40 disabled:cursor-not-allowed",
        danger ? "text-destructive hover:bg-destructive/10" : "hover:bg-accent"
      )}
    >
      {icon}
      {children}
    </button>
  );
}
