import { Clock3, File, Folder, GitBranch, MessageSquare, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { SessionRecord, WorkspaceOverview } from "@/lib/types";

export function ProjectSidebar({ overview, sessions, activeSession, onFile, onSession, onDelete }: {
  overview?: WorkspaceOverview;
  sessions: SessionRecord[];
  activeSession?: string;
  onFile: (path: string) => void;
  onSession: (session: SessionRecord) => void;
  onDelete: (id: string) => void;
}) {
  return (
    <aside className="flex w-64 shrink-0 flex-col border-r bg-card/40">
      <Tabs defaultValue="files" className="flex min-h-0 flex-1 flex-col">
        <div className="border-b p-2"><TabsList className="grid w-full grid-cols-2"><TabsTrigger value="files"><Folder className="mr-1 h-3.5 w-3.5" />Files</TabsTrigger><TabsTrigger value="sessions"><Clock3 className="mr-1 h-3.5 w-3.5" />Sessions</TabsTrigger></TabsList></div>
        <TabsContent value="files" className="min-h-0 flex-1 p-0">
          <ScrollArea className="h-full">
            <div className="p-2">
              {overview?.isGitRepository && <div className="mb-2 flex items-center gap-2 rounded px-2 py-1 text-xs text-muted-foreground"><GitBranch className="h-3.5 w-3.5" />Git repository</div>}
              {overview?.files.map((file) => <button key={file.path} className="flex w-full items-center gap-2 truncate rounded px-2 py-1.5 text-left text-xs text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => onFile(file.path)} title={file.path}><File className="h-3.5 w-3.5 shrink-0" /><span className="truncate">{file.path}</span></button>)}
              {!overview && <p className="p-3 text-xs text-muted-foreground">Open a project to inspect its files.</p>}
            </div>
          </ScrollArea>
        </TabsContent>
        <TabsContent value="sessions" className="min-h-0 flex-1 p-0">
          <ScrollArea className="h-full"><div className="space-y-1 p-2">
            {sessions.map((session) => <div key={session.id} className={`group flex items-start gap-1 rounded p-1 ${activeSession === session.id ? "bg-accent" : "hover:bg-accent/60"}`}><button className="min-w-0 flex-1 px-1 py-1 text-left" onClick={() => onSession(session)}><div className="flex items-center gap-2 text-xs font-medium"><MessageSquare className="h-3.5 w-3.5" /><span className="truncate">{session.messages.find((message) => message.role === "user")?.content || "New session"}</span></div><div className="mt-1 truncate text-[10px] text-muted-foreground">{session.provider}/{session.model} · {session.status}</div></button><Button size="icon" variant="ghost" className="h-7 w-7 opacity-0 group-hover:opacity-100" onClick={() => onDelete(session.id)}><Trash2 className="h-3 w-3" /></Button></div>)}
            {sessions.length === 0 && <p className="p-3 text-xs text-muted-foreground">No sessions for this project.</p>}
          </div></ScrollArea>
        </TabsContent>
      </Tabs>
    </aside>
  );
}
