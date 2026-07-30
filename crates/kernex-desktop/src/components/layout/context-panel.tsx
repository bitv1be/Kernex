import CodeMirror from "@uiw/react-codemirror";
import { Activity, Braces, File, FileDiff, Files, GitBranch, Network, Search, Server, TerminalSquare, X } from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/shared/empty-state";
import { ResizeHandle } from "@/components/shared/resize-handle";
import { DiffLines, FileChange } from "@/components/events/file-change";
import { ToolEvent } from "@/components/events/tool-event";
import { useAppStore, type ContextTab } from "@/lib/store";
import { TerminalPanel } from "@/components/terminal-panel";

const tabs: { id: ContextTab; label: string; icon: typeof Files }[] = [
  { id: "files", label: "Files", icon: Files },
  { id: "changes", label: "Changes", icon: FileDiff },
  { id: "terminal", label: "Terminal", icon: TerminalSquare },
  { id: "activity", label: "Activity", icon: Activity },
  { id: "context", label: "Context", icon: Braces },
];

export function ContextPanel({ onFile }: { onFile: (path: string) => void }) {
  const [search, setSearch] = useState("");
  const open = useAppStore((state) => state.contextOpen);
  const width = useAppStore((state) => state.contextWidth);
  const tab = useAppStore((state) => state.contextTab);
  const setOpen = useAppStore((state) => state.setContextOpen);
  const setWidth = useAppStore((state) => state.setContextWidth);
  const setTab = useAppStore((state) => state.setContextTab);
  const workspace = useAppStore((state) => state.workspace);
  const overview = useAppStore((state) => state.overview);
  const file = useAppStore((state) => state.selectedFile);
  const content = useAppStore((state) => state.fileContent);
  const diff = useAppStore((state) => state.diff);
  const events = useAppStore((state) => state.events);
  const session = useAppStore((state) => state.session);
  const theme = useAppStore((state) => state.settings?.theme);
  const filteredFiles = useMemo(() => overview?.files.filter((item) => item.path.toLowerCase().includes(search.toLowerCase())) ?? [], [overview, search]);
  const activity = events.filter((event) => event.type === "tool_started" || event.type === "tool_finished" || event.type === "tool_failed");
  if (!open) return null;

  return <aside aria-label="Workspace context" className="relative hidden shrink-0 flex-col border-l bg-card xl:flex" style={{ width }}>
    <ResizeHandle side="left" onResize={(delta) => setWidth(width + delta)} />
    <div className="flex h-10 shrink-0 items-center border-b px-1">
      <div className="flex min-w-0 flex-1 items-center">{tabs.map(({ id, label, icon: Icon }) => <button key={id} className={`flex h-8 min-w-0 flex-1 items-center justify-center gap-1 border-b text-[9px] transition-colors ${tab === id ? "border-foreground text-foreground" : "border-transparent text-muted-foreground hover:text-foreground"}`} onClick={() => setTab(id)} title={label}><Icon className="h-3.5 w-3.5" /><span className="hidden min-[1500px]:inline">{label}</span></button>)}</div>
      <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setOpen(false)} aria-label="Close context panel"><X className="h-3.5 w-3.5" /></Button>
    </div>
    {tab === "files" && <div className="flex min-h-0 flex-1 flex-col">
      <div className="relative border-b p-2"><Search className="absolute left-4 top-4 h-3.5 w-3.5 text-muted-foreground" /><Input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Filter project files" aria-label="Filter project files" className="h-8 pl-8 text-xs" /></div>
      {file && <div className="flex h-8 shrink-0 items-center gap-2 border-b px-3 text-[10px]"><File className="h-3.5 w-3.5" /><span className="min-w-0 flex-1 truncate font-mono" title={file}>{file}</span><Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => useAppStore.getState().setFile()} aria-label="Close file"><X className="h-3 w-3" /></Button></div>}
      {file ? <div className="min-h-0 flex-1 overflow-auto"><CodeMirror value={content} readOnly theme={theme === "light" ? "light" : "dark"} basicSetup={{ lineNumbers: true, foldGutter: true, highlightActiveLine: false }} className="min-h-full text-xs" /></div> : <div className="min-h-0 flex-1 overflow-auto p-2">{filteredFiles.map((item) => <button key={item.path} className="content-auto flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[10px] text-muted-foreground hover:bg-muted hover:text-foreground" onClick={() => onFile(item.path)} title={item.path}><File className="h-3.5 w-3.5 shrink-0" /><span className="min-w-0 flex-1 truncate font-mono">{item.path}</span><span className="text-[8px]">{formatSize(item.size)}</span></button>)}{!overview && <EmptyState icon={Files} title="No project open" description="Open a project to browse its indexed files." />}{overview && !filteredFiles.length && <EmptyState icon={Search} title="No matching files" description="Try a shorter file name or path." />}</div>}
    </div>}
    {tab === "changes" && <div className="min-h-0 flex-1 overflow-auto p-3">{diff ? <FileChange diff={diff} onOpenFile={(path) => { setTab("files"); onFile(path); }} /> : session?.generated_diffs.length ? <div className="space-y-3">{session.generated_diffs.map((item, index) => <FileChange key={index} diff={item} onOpenFile={(path) => { setTab("files"); onFile(path); }} />)}</div> : <EmptyState icon={FileDiff} title="No file changes" description="Working-tree and agent-generated diffs will appear here." />}</div>}
    {tab === "terminal" && <div className="min-h-0 flex-1"><TerminalPanel workspace={workspace} /></div>}
    {tab === "activity" && <div className="min-h-0 flex-1 space-y-2 overflow-auto p-3">{activity.map((event, index) => event.type === "tool_started" ? <ToolEvent key={`${event.call.id}-${index}`} call={event.call} state="running" /> : <ToolEvent key={`${event.call_id}-${index}`} result={{ call_id: event.call_id, name: event.name, result: event.type === "tool_finished" ? event.result : "", error: event.type === "tool_failed" ? event.error : undefined, timestamp: new Date().toISOString() }} />)}{!activity.length && <EmptyState icon={Activity} title="No recent activity" description="Tool calls and execution details appear while the agent is working." />}</div>}
    {tab === "context" && <div className="min-h-0 flex-1 space-y-5 overflow-auto p-4">
      <ContextSection title="Workspace" icon={GitBranch}>{overview ? <><ContextRow label="Path" value={overview.path} mono /><ContextRow label="Indexed files" value={String(overview.files.length)} /><ContextRow label="Git" value={overview.isGitRepository ? "Repository" : "Not detected"} /></> : <p className="text-xs text-muted-foreground">No workspace selected.</p>}</ContextSection>
      <ContextSection title="Instructions" icon={Braces}>{overview?.instructions.length ? overview.instructions.map((path) => <div key={path} className="truncate font-mono text-[10px]" title={path}>{path}</div>) : <p className="text-xs text-muted-foreground">No project instructions discovered.</p>}</ContextSection>
      <ContextSection title="Integrations" icon={Network}><div className="flex flex-wrap gap-1.5">{overview?.mcpServers.map((name) => <Badge key={name} variant="outline"><Server className="h-3 w-3" />{name}</Badge>)}{overview?.languageServers.map((name) => <Badge key={name} variant="outline">LSP · {name}</Badge>)}{!overview?.mcpServers.length && !overview?.languageServers.length && <p className="text-xs text-muted-foreground">No MCP or language servers configured.</p>}</div></ContextSection>
      <ContextSection title="Session context" icon={Braces}><ContextRow label="Input tokens" value={(session?.token_usage.input_tokens ?? 0).toLocaleString()} /><ContextRow label="Output tokens" value={(session?.token_usage.output_tokens ?? 0).toLocaleString()} /><ContextRow label="Permissions" value={String(session?.permission_decisions.length ?? 0)} /></ContextSection>
      {diff && <details><summary className="cursor-pointer text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Raw working diff</summary><div className="mt-2 max-h-80 overflow-auto rounded border bg-code py-2"><DiffLines diff={diff} /></div></details>}
    </div>}
  </aside>;
}

function ContextSection({ title, icon: Icon, children }: { title: string; icon: typeof Files; children: React.ReactNode }) {
  return <section><h2 className="mb-2 flex items-center gap-2 text-[9px] font-medium uppercase tracking-wider text-muted-foreground"><Icon className="h-3.5 w-3.5" />{title}</h2><div className="space-y-2 rounded-md border bg-background/45 p-3">{children}</div></section>;
}

function ContextRow({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return <div className="flex items-start justify-between gap-3 text-[10px]"><span className="text-muted-foreground">{label}</span><span className={`min-w-0 truncate text-right ${mono ? "font-mono" : ""}`} title={value}>{value}</span></div>;
}

function formatSize(size: number) {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${Math.round(size / 1024)} KB`;
  return `${(size / 1024 / 1024).toFixed(1)} MB`;
}
