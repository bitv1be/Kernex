import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, FolderOpen, GitBranch, Sparkles } from "lucide-react";
import { useEffect, useMemo } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { api } from "@/lib/api";
import { useAppStore } from "@/lib/store";
import type { AgentEvent, PendingApproval, SessionRecord, Settings } from "@/lib/types";
import { ChatPanel } from "@/components/chat-panel";
import { InspectorPanel } from "@/components/inspector-panel";
import { PermissionDialog } from "@/components/permission-dialog";
import { ProjectSidebar } from "@/components/project-sidebar";
import { SettingsSheet } from "@/components/settings-sheet";

interface AgentFinished { sessionId: string; error?: string; cancelled: boolean }

export default function App() {
  const queryClient = useQueryClient();
  const store = useAppStore();
  const appendEvent = useAppStore((state) => state.appendEvent);
  const setApproval = useAppStore((state) => state.setApproval);
  const setError = useAppStore((state) => state.setError);
  const setRunning = useAppStore((state) => state.setRunning);
  const setSession = useAppStore((state) => state.setSession);
  const setSettings = useAppStore((state) => state.setSettings);
  const workspace = useAppStore((state) => state.workspace);
  const themeSetting = useAppStore((state) => state.settings?.theme);
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: api.settings });
  const providersQuery = useQuery({ queryKey: ["providers"], queryFn: api.providers });
  const authQuery = useQuery({ queryKey: ["auth"], queryFn: api.authStatus });
  const sessionsQuery = useQuery({ queryKey: ["sessions", workspace], queryFn: () => api.sessions(workspace, 100), enabled: Boolean(workspace) });
  const diagnostics = useMemo(() => store.events.map((event) => `${new Date().toISOString()} ${JSON.stringify(event)}`), [store.events]);

  useEffect(() => { if (settingsQuery.data) setSettings(settingsQuery.data); }, [settingsQuery.data, setSettings]);
  useEffect(() => {
    const theme = themeSetting ?? "dark";
    const light = theme === "light" || (theme === "system" && matchMedia("(prefers-color-scheme: light)").matches);
    document.documentElement.classList.toggle("light", light);
    document.documentElement.classList.toggle("dark", !light);
  }, [themeSetting]);
  useEffect(() => {
    const unlisteners = [
      listen<AgentEvent>("agent-event", ({ payload }) => appendEvent(payload)),
      listen<PendingApproval>("permission-request", ({ payload }) => setApproval(payload)),
      listen<AgentFinished>("agent-finished", async ({ payload }) => {
        setRunning(false);
        if (payload.error && !payload.cancelled) setError(payload.error);
        try { setSession(await api.session(payload.sessionId)); } catch (error) { setError(String(error)); }
        await queryClient.invalidateQueries({ queryKey: ["sessions", workspace] });
      }),
    ];
    return () => { for (const unlisten of unlisteners) void unlisten.then((dispose) => dispose()); };
  }, [appendEvent, queryClient, setApproval, setError, setRunning, setSession, workspace]);

  const openProject = async (path?: string) => {
    try {
      const selected = path ?? await open({ directory: true, multiple: false, title: "Open a project in Kernex" });
      if (!selected || Array.isArray(selected)) return;
      const overview = await api.overview(selected);
      store.setWorkspace(overview.path, overview);
      store.setSession(undefined);
      store.setDiff(await api.gitDiff(overview.path).catch(() => ""));
      if (store.settings) {
        const recent = [overview.path, ...store.settings.recent_projects.filter((item) => item !== overview.path)].slice(0, 20);
        const next = { ...store.settings, recent_projects: recent };
        await api.saveSettings(next);
        store.setSettings(next);
      }
    } catch (error) { store.setError(String(error)); }
  };

  const runAgent = async (task: string) => {
    if (!store.workspace || !store.settings) { store.setError("Open a project and configure a model first."); return; }
    if (!store.settings.provider.model.trim()) { store.setError("Select a model in Settings before starting an agent."); return; }
    store.resetStream();
    store.setRunning(true);
    try {
      const id = await api.startAgent({
        workspace: store.workspace,
        task,
        provider: store.settings.provider.name,
        model: store.settings.provider.model,
        baseUrl: store.settings.provider.base_url,
        authProfile: store.settings.provider.auth_profile,
        permissionMode: store.settings.permission_mode,
        maxSteps: 24,
        sessionId: store.session?.id,
      });
      const prior = store.session;
      const local: SessionRecord = prior ? { ...prior, id, messages: [...prior.messages, { role: "user", content: task, tool_calls: [] }], status: "active" } : {
        id, workspace_path: store.workspace, provider: store.settings.provider.name, model: store.settings.provider.model,
        messages: [{ role: "user", content: task, tool_calls: [] }], tool_calls: [], tool_results: [], permission_decisions: [],
        created_at: new Date().toISOString(), updated_at: new Date().toISOString(), token_usage: {}, generated_diffs: [], status: "active",
      };
      store.setSession(local);
    } catch (error) { store.setRunning(false); store.setError(String(error)); }
  };

  const saveSettings = async (settings: Settings) => {
    await api.saveSettings(settings);
    store.setSettings(settings);
  };
  const refreshOverview = async () => {
    if (!store.workspace) return;
    store.setWorkspace(store.workspace, await api.overview(store.workspace));
  };
  const loadSession = async (session: SessionRecord) => { store.setSession(await api.session(session.id)); };
  const deleteSession = async (id: string) => { await api.deleteSession(id); if (store.session?.id === id) store.setSession(undefined); await sessionsQuery.refetch(); };
  const selectFile = async (path: string) => { store.setFile(path, "Loading…"); try { store.setFile(path, await api.readFile(store.workspace, path)); } catch (error) { store.setFile(path, `Unable to read file: ${String(error)}`); } };

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col">
        <header className="flex h-12 shrink-0 items-center justify-between border-b bg-card/70 px-3 backdrop-blur">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex items-center gap-2 font-semibold"><div className="flex h-7 w-7 items-center justify-center rounded-md bg-emerald-500 text-emerald-950"><Sparkles className="h-4 w-4" /></div>Kernex</div>
            <div className="h-5 w-px bg-border" />
            <DropdownMenu>
              <DropdownMenuTrigger asChild><Button variant="ghost" size="sm" className="max-w-[460px]"><FolderOpen className="h-4 w-4" /><span className="truncate">{store.workspace || "Open project"}</span><ChevronDown className="h-3.5 w-3.5" /></Button></DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-96"><DropdownMenuItem onClick={() => void openProject()}><FolderOpen className="mr-2 h-4 w-4" />Open project…</DropdownMenuItem>{store.settings?.recent_projects.length ? <DropdownMenuSeparator /> : null}{store.settings?.recent_projects.map((path) => <DropdownMenuItem key={path} onClick={() => void openProject(path)} className="truncate">{path}</DropdownMenuItem>)}</DropdownMenuContent>
            </DropdownMenu>
            {store.overview?.isGitRepository && <span className="hidden items-center gap-1 text-xs text-muted-foreground md:flex"><GitBranch className="h-3.5 w-3.5" />repository</span>}
          </div>
          <div className="flex items-center gap-1">
            <span className="mr-2 hidden text-xs text-muted-foreground lg:inline">{store.settings?.provider.name}/{store.settings?.provider.model || "no model"}</span>
            <Tooltip><TooltipTrigger asChild><span>{store.settings ? <SettingsSheet key={JSON.stringify(store.settings)} settings={store.settings} providerInfo={providersQuery.data ?? []} auth={authQuery.data ?? []} overview={store.overview} diagnostics={diagnostics} onSave={saveSettings} refreshAuth={async () => { await authQuery.refetch(); }} refreshOverview={refreshOverview} /> : <Button variant="ghost" size="sm" disabled>Loading settings…</Button>}</span></TooltipTrigger><TooltipContent>Settings</TooltipContent></Tooltip>
          </div>
        </header>
        {store.error && <Alert className="m-2 w-auto shrink-0 border-red-500/40 bg-red-500/5"><AlertTitle>Something went wrong</AlertTitle><AlertDescription className="flex justify-between gap-3">{store.error}<Button variant="ghost" size="sm" onClick={() => store.setError(undefined)}>Dismiss</Button></AlertDescription></Alert>}
        <main className="flex min-h-0 flex-1">
          <ProjectSidebar overview={store.overview} sessions={sessionsQuery.data ?? []} activeSession={store.session?.id} onFile={(path) => void selectFile(path)} onSession={(session) => void loadSession(session)} onDelete={(id) => void deleteSession(id)} />
          <ChatPanel onRun={runAgent} onCancel={async () => { await api.cancelAgent(); }} />
          <InspectorPanel />
        </main>
        <footer className="flex h-6 shrink-0 items-center justify-between border-t bg-card px-3 text-[10px] text-muted-foreground"><span>{store.running ? "Agent running" : "Ready"}</span><span>{store.session ? `${store.session.token_usage.input_tokens ?? 0} in · ${store.session.token_usage.output_tokens ?? 0} out` : "Shared local sessions enabled"}</span></footer>
        <PermissionDialog />
      </div>
    </TooltipProvider>
  );
}
