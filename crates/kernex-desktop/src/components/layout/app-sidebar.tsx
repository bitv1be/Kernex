import { ChevronRight, Folder, FolderOpen, LogIn, MessageSquare, PanelLeftClose, PanelLeftOpen, Plus, Search, Settings2, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { KernexMark } from "@/components/shared/kernex-mark";
import { ResizeHandle } from "@/components/shared/resize-handle";
import { StatusIndicator } from "@/components/shared/status-indicator";
import type { AuthStatus, SessionRecord, Settings } from "@/lib/types";
import { useAppStore } from "@/lib/store";

function basename(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

function sessionTitle(session: SessionRecord) {
  return session.messages.find((message) => message.role === "user")?.content.trim() || "New session";
}

export function AppSidebar({ sessions, settings, auth, codexConnected, activeSession, workspace, onNewSession, onOpenProject, onSession, onDelete }: {
  sessions: SessionRecord[];
  settings?: Settings;
  auth: AuthStatus[];
  codexConnected: boolean;
  activeSession?: string;
  workspace: string;
  onNewSession: () => void;
  onOpenProject: (path?: string) => void;
  onSession: (session: SessionRecord) => void;
  onDelete: (id: string) => void;
}) {
  const [search, setSearch] = useState("");
  const [narrow, setNarrow] = useState(() => matchMedia("(max-width: 900px)").matches);
  const mode = useAppStore((state) => state.sidebarMode);
  const width = useAppStore((state) => state.sidebarWidth);
  const setWidth = useAppStore((state) => state.setSidebarWidth);
  const setMode = useAppStore((state) => state.setSidebarMode);
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const filtered = useMemo(() => sessions.filter((session) => sessionTitle(session).toLowerCase().includes(search.toLowerCase())), [search, sessions]);
  const providerAuth = auth.find((status) => status.profile.provider === settings?.provider.name && (status.active || status.profile.name === settings?.provider.auth_profile));
  const connected = settings?.provider.name === "local" || (settings?.provider.name === "codex" ? codexConnected : Boolean(providerAuth?.credential_available && !providerAuth.expired));

  useEffect(() => {
    const media = matchMedia("(max-width: 900px)");
    const update = () => setNarrow(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  if (mode === "hidden") return null;
  const compact = mode === "compact" || narrow;
  return <aside aria-label="Kernex navigation" className="relative flex shrink-0 flex-col overflow-hidden border-r bg-sidebar text-sidebar-foreground" style={{ width: compact ? 52 : width }}>
    {!compact && <ResizeHandle side="right" onResize={(delta) => setWidth(width + delta)} />}
    <div className="flex h-12 items-center gap-2 px-2">
      <KernexMark />
      {!compact && <div className="min-w-0"><div className="text-xs font-semibold tracking-tight">Kernex</div><div className="truncate text-[10px] text-muted-foreground">Developer agent</div></div>}
      <Button variant="ghost" size="icon" className="ml-auto h-7 w-7" onClick={() => setMode(compact ? "expanded" : "compact")} aria-label={compact ? "Expand sidebar" : "Compact sidebar"}>{compact ? <PanelLeftOpen className="h-3.5 w-3.5" /> : <PanelLeftClose className="h-3.5 w-3.5" />}</Button>
    </div>
    <div className="px-2 pb-2">
      <Tooltip><TooltipTrigger asChild><Button className="w-full justify-start" size={compact ? "icon" : "sm"} onClick={onNewSession} aria-label="New session"><Plus className="h-4 w-4" />{!compact && <span>New session</span>}</Button></TooltipTrigger>{compact && <TooltipContent side="right">New session · Ctrl N</TooltipContent>}</Tooltip>
    </div>
    {!compact && <div className="px-2 pb-2"><div className="relative"><Search className="pointer-events-none absolute left-2.5 top-2.5 h-3.5 w-3.5 text-muted-foreground" /><Input data-session-search value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search sessions" aria-label="Search sessions" className="h-8 border-transparent bg-sidebar-accent/55 pl-8 text-xs shadow-none focus-visible:border-input" /></div></div>}
    <Separator />
    <ScrollArea className="min-h-0 flex-1">
      <nav className="space-y-5 p-2">
        <SidebarSection compact={compact} label="Workspace">
          <SidebarButton compact={compact} active={Boolean(workspace)} icon={workspace ? FolderOpen : Folder} label={workspace ? basename(workspace) : "Open project"} title={workspace || "Open project"} onClick={() => onOpenProject()} />
          {!compact && settings?.recent_projects.filter((path) => path !== workspace).slice(0, 3).map((path) => <button key={path} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[11px] text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground" title={path} onClick={() => onOpenProject(path)}><ChevronRight className="h-3 w-3 shrink-0" /><span className="truncate">{basename(path)}</span></button>)}
        </SidebarSection>
        <SidebarSection compact={compact} label="Recent sessions">
          {filtered.slice(0, compact ? 8 : 30).map((session) => <div key={session.id} className="group relative">
            <SidebarButton compact={compact} active={activeSession === session.id} icon={MessageSquare} label={sessionTitle(session)} title={sessionTitle(session)} onClick={() => onSession(session)} meta={`${session.provider} · ${session.status}`} />
            {!compact && <Button variant="ghost" size="icon" className="absolute right-1 top-1 h-6 w-6 opacity-0 group-hover:opacity-100 focus:opacity-100" onClick={(event) => { event.stopPropagation(); onDelete(session.id); }} aria-label={`Delete ${sessionTitle(session)}`}><Trash2 className="h-3 w-3" /></Button>}
          </div>)}
          {!filtered.length && !compact && <p className="px-2 py-4 text-center text-[11px] text-muted-foreground">{search ? "No matching sessions" : workspace ? "No sessions yet" : "Open a project to begin"}</p>}
        </SidebarSection>
      </nav>
    </ScrollArea>
    <Separator />
    <div className="space-y-1 p-2">
      <SidebarButton compact={compact} icon={Settings2} label="Settings" title="Settings" onClick={() => setSettingsOpen(true)} />
      <div className={compact ? "flex justify-center py-2" : "rounded border border-sidebar-border bg-sidebar-accent/30 p-2"}>
        {compact ? <Tooltip><TooltipTrigger><StatusIndicator status={connected ? "success" : "offline"} label="" /></TooltipTrigger><TooltipContent side="right">{connected ? "Provider ready" : "Provider needs authentication"}</TooltipContent></Tooltip> : <>
          <div className="flex items-center justify-between gap-2"><span className="truncate text-[11px] font-medium">{settings?.provider.name ?? "Loading provider"}</span><StatusIndicator status={connected ? "success" : "offline"} label={connected ? "Ready" : "Offline"} /></div>
          <div className="mt-1 truncate text-[10px] text-muted-foreground">{settings?.provider.model || "No model selected"}</div>
          {!connected && <button className="mt-2 flex items-center gap-1 text-[10px] text-foreground hover:underline" onClick={() => setSettingsOpen(true, "authentication")}><LogIn className="h-3 w-3" />Connect account</button>}
        </>}
      </div>
    </div>
  </aside>;
}

function SidebarSection({ compact, label, children }: { compact: boolean; label: string; children: React.ReactNode }) {
  return <section aria-label={label}>{!compact && <h2 className="mb-1 px-2 text-[9px] font-medium uppercase tracking-[0.12em] text-muted-foreground">{label}</h2>}<div className="space-y-0.5">{children}</div></section>;
}

function SidebarButton({ compact, active = false, icon: Icon, label, title, meta, onClick }: { compact: boolean; active?: boolean; icon: typeof Folder; label: string; title: string; meta?: string; onClick: () => void }) {
  const button = <button className={`flex w-full items-center rounded text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-sidebar-ring ${compact ? "h-9 justify-center" : "gap-2 px-2 py-1.5"} ${active ? "bg-sidebar-accent text-sidebar-accent-foreground" : "text-muted-foreground hover:bg-sidebar-accent/70 hover:text-sidebar-accent-foreground"}`} onClick={onClick} aria-current={active ? "page" : undefined}>
    <Icon className="h-3.5 w-3.5 shrink-0" />
    {!compact && <span className="min-w-0 flex-1"><span className="block truncate text-[11px] font-medium" title={title}>{label}</span>{meta && <span className="mt-0.5 block truncate text-[9px] text-muted-foreground">{meta}</span>}</span>}
  </button>;
  return compact ? <Tooltip><TooltipTrigger asChild>{button}</TooltipTrigger><TooltipContent side="right">{title}</TooltipContent></Tooltip> : button;
}
