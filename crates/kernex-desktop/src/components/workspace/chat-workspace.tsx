import { Clock3, FolderOpen, MessageSquarePlus, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/shared/empty-state";
import { KernexMark } from "@/components/shared/kernex-mark";
import { MessageComposer, type ComposerSubmission } from "@/components/composer/message-composer";
import type { SessionRecord, Settings } from "@/lib/types";
import { useAppStore } from "@/lib/store";
import { ConversationTimeline } from "./conversation-timeline";

export function ChatWorkspace({ sessions, onRun, onCancel, onUpdateSettings, onOpenProject, onSession, onOpenFile }: { sessions: SessionRecord[]; onRun: (submission: ComposerSubmission) => Promise<void>; onCancel: () => Promise<void>; onUpdateSettings: (settings: Settings) => Promise<void>; onOpenProject: (path?: string) => void; onSession: (session: SessionRecord) => void; onOpenFile: (path: string) => void }) {
  const workspace = useAppStore((state) => state.workspace);
  const session = useAppStore((state) => state.session);
  const settings = useAppStore((state) => state.settings);
  const events = useAppStore((state) => state.events);
  const streamed = useAppStore((state) => state.streamedAnswer);
  const running = useAppStore((state) => state.running);
  const recentProjects = settings?.recent_projects ?? [];
  const hasConversation = Boolean(session?.messages.length || streamed || events.length);
  return <section className="flex min-w-0 flex-1 flex-col bg-background" aria-label="Agent workspace">
    {workspace && <div className="flex h-10 shrink-0 items-center justify-between border-b px-3">
      <div className="min-w-0"><h1 className="truncate text-xs font-medium">{session?.messages.find((message) => message.role === "user")?.content.split("\n").at(-1)?.slice(0, 100) || "New session"}</h1><p className="text-[9px] text-muted-foreground">{running ? "Agent is working" : session ? `${session.provider} / ${session.model} · ${session.status}` : "Ready for a new task"}</p></div>
      <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => window.dispatchEvent(new Event("kernex:search-messages"))} disabled={!hasConversation} aria-label="Search conversation"><Search className="h-3.5 w-3.5" /></Button>
    </div>}
    {!workspace ? <WelcomeScreen recentProjects={recentProjects} sessions={sessions} onOpenProject={onOpenProject} onSession={onSession} /> : hasConversation ? <ConversationTimeline session={session} events={events} streamed={streamed} provider={settings?.provider.name} model={settings?.provider.model} onOpenFile={onOpenFile} /> : <div className="min-h-0 flex-1 overflow-auto"><EmptyState icon={MessageSquarePlus} title="Start a focused coding session" description="Describe what you want to inspect, change, test, or review. Kernex will keep tool use visible and ask before protected actions." action={{ label: "Focus composer", onClick: () => window.dispatchEvent(new Event("kernex:focus-composer")) }} /></div>}
    <MessageComposer onRun={onRun} onCancel={onCancel} onUpdateSettings={onUpdateSettings} />
  </section>;
}

function WelcomeScreen({ recentProjects, sessions, onOpenProject, onSession }: { recentProjects: string[]; sessions: SessionRecord[]; onOpenProject: (path?: string) => void; onSession: (session: SessionRecord) => void }) {
  return <div className="min-h-0 flex-1 overflow-y-auto">
    <div className="mx-auto flex min-h-full max-w-3xl flex-col justify-center px-8 py-12">
      <div className="mb-8"><KernexMark className="mb-4 h-9 w-9" /><h1 className="text-xl font-semibold tracking-tight">Kernex</h1><p className="mt-2 max-w-lg text-sm leading-6 text-muted-foreground">A calm, permission-aware workspace for inspecting code, running tools, and shipping deliberate changes.</p></div>
      <Button className="w-fit" onClick={() => onOpenProject()}><FolderOpen className="h-4 w-4" />Open a project</Button>
      <div className="mt-10 grid gap-8 md:grid-cols-2">
        <section><h2 className="mb-2 flex items-center gap-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"><FolderOpen className="h-3.5 w-3.5" />Recent projects</h2><div className="space-y-1">{recentProjects.slice(0, 5).map((path) => <button key={path} className="flex w-full items-center gap-2 rounded px-2 py-2 text-left text-xs hover:bg-muted" onClick={() => onOpenProject(path)}><span className="min-w-0 flex-1"><span className="block truncate font-medium">{path.split(/[\\/]/).filter(Boolean).at(-1)}</span><span className="block truncate text-[9px] text-muted-foreground" title={path}>{path}</span></span></button>)}{!recentProjects.length && <p className="px-2 py-3 text-xs text-muted-foreground">No recent projects.</p>}</div></section>
        <section><h2 className="mb-2 flex items-center gap-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"><Clock3 className="h-3.5 w-3.5" />Recent sessions</h2><div className="space-y-1">{sessions.slice(0, 5).map((session) => <button key={session.id} className="w-full rounded px-2 py-2 text-left text-xs hover:bg-muted" onClick={() => onSession(session)}><span className="block truncate font-medium">{session.messages.find((message) => message.role === "user")?.content || "New session"}</span><span className="mt-0.5 block text-[9px] text-muted-foreground">{session.provider} / {session.model}</span></button>)}{!sessions.length && <p className="px-2 py-3 text-xs text-muted-foreground">No recent sessions.</p>}</div></section>
      </div>
      <div className="mt-10 flex flex-wrap gap-2 text-[10px] text-muted-foreground"><span>Ctrl O open project</span><span>·</span><span>Ctrl K commands</span><span>·</span><span>Ctrl , settings</span></div>
    </div>
  </div>;
}
