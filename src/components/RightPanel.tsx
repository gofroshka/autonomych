import { Activity, MessageSquare } from "lucide-react";
import type { EventRow, ProjectRow } from "../types";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./ui/tabs";
import { ActivityLog } from "./ActivityLog";
import { ChatPanel } from "./ChatPanel";

export function RightPanel({ events, project }: { events: EventRow[]; project: ProjectRow | null }) {
  return (
    <aside className="flex flex-col w-[440px] shrink-0 border-l border-border bg-card/30">
      <Tabs defaultValue="activity" className="flex flex-col flex-1 min-h-0">
        <TabsList className="px-2">
          <TabsTrigger value="activity" className="gap-1.5">
            <Activity className="h-3 w-3" />
            Активность
          </TabsTrigger>
          <TabsTrigger value="chat" disabled={!project} className="gap-1.5">
            <MessageSquare className="h-3 w-3" />
            Чат с Overseer
          </TabsTrigger>
        </TabsList>
        {/* forceMount keeps panels mounted across tab switches so transient
            state (e.g. ChatPanel's "thinking" animation) survives. CSS
            data-[state=inactive]:hidden in ./ui/tabs.tsx handles visibility. */}
        <TabsContent value="activity" className="flex flex-col" forceMount>
          <ActivityLog events={events} />
        </TabsContent>
        <TabsContent value="chat" className="flex flex-col" forceMount>
          {project && <ChatPanel project={project} />}
        </TabsContent>
      </Tabs>
    </aside>
  );
}
