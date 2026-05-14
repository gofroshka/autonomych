import { Activity, ListTodo, MessageSquare } from "lucide-react";
import type { BacklogItem, EventRow, ProjectRow } from "../types";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./ui/tabs";
import { ActivityLog } from "./ActivityLog";
import { ChatPanel } from "./ChatPanel";
import { BacklogPanel } from "./BacklogPanel";

export function RightPanel({
  events,
  project,
  backlog,
  onBacklogChanged,
}: {
  events: EventRow[];
  project: ProjectRow | null;
  backlog: BacklogItem[];
  onBacklogChanged: () => void;
}) {
  const activeCount = backlog.filter(
    (b) => b.status === "pending" || b.status === "in_iteration"
  ).length;
  return (
    <aside className="flex flex-col w-[440px] shrink-0 border-l border-border bg-card/30">
      <Tabs defaultValue="activity" className="flex flex-col flex-1 min-h-0">
        <TabsList className="px-2">
          <TabsTrigger value="activity" className="gap-1.5">
            <Activity className="h-3 w-3" />
            Активность
          </TabsTrigger>
          <TabsTrigger value="backlog" disabled={!project} className="gap-1.5">
            <ListTodo className="h-3 w-3" />
            Беклог
            {activeCount > 0 && (
              <span className="ml-0.5 text-[10px] font-mono text-muted-foreground">
                {activeCount}
              </span>
            )}
          </TabsTrigger>
          <TabsTrigger value="chat" disabled={!project} className="gap-1.5">
            <MessageSquare className="h-3 w-3" />
            Чат
          </TabsTrigger>
        </TabsList>
        {/* forceMount keeps panels mounted across tab switches so transient
            state (e.g. ChatPanel's "thinking" animation) survives. CSS
            data-[state=inactive]:hidden in ./ui/tabs.tsx handles visibility. */}
        <TabsContent value="activity" className="flex flex-col" forceMount>
          <ActivityLog events={events} />
        </TabsContent>
        <TabsContent value="backlog" className="flex flex-col" forceMount>
          <BacklogPanel
            project={project}
            activeBacklog={backlog}
            onChanged={onBacklogChanged}
          />
        </TabsContent>
        <TabsContent value="chat" className="flex flex-col" forceMount>
          {project && <ChatPanel project={project} />}
        </TabsContent>
      </Tabs>
    </aside>
  );
}
