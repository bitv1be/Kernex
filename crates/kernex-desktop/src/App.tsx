import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2, Settings2, X } from "lucide-react";
import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AppCommandPalette } from "@/components/command-palette/command-palette";
import type { ComposerSubmission } from "@/components/composer/message-composer";
import { AppSidebar } from "@/components/layout/app-sidebar";
import { AppTitlebar } from "@/components/layout/app-titlebar";
import { ContextPanel } from "@/components/layout/context-panel";
import { StatusBar } from "@/components/layout/status-bar";
import { PermissionDialog } from "@/components/permission-dialog";
import { ConfirmationDialog } from "@/components/shared/confirmation-dialog";
import { ChatWorkspace } from "@/components/workspace/chat-workspace";
import { api } from "@/lib/api";
import { useAppStore } from "@/lib/store";
import { prepareTask } from "@/lib/task";
import type { AgentEvent, PendingApproval, SessionRecord, Settings } from "@/lib/types";

interface AgentFinished { sessionId: string; error?: string; cancelled: boolean }

const SettingsSheet = lazy(() => import("@/components/settings-sheet").then((module) => ({ default: module.SettingsSheet })));

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
  const notifyOnComplete = useAppStore((state) => state.notifyOnComplete);
  const [completionNotice, setCompletionNotice] = useState<string>();
  const [pendingDelete, setPendingDelete] = useState<SessionRecord>();
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: api.settings });
  const providersQuery = useQuery({ queryKey: ["providers"], queryFn: api.providers });
  const authQuery = useQuery({ queryKey: ["auth"], queryFn: api.authStatus });
  const codexAccountQuery = useQuery({ queryKey: ["codex-account"], queryFn: api.codexAccount, enabled: settingsQuery.data?.provider.name === "codex" });
  const sessionsQuery = useQuery({ queryKey: ["sessions", workspace], queryFn: () => api.sessions(workspace || undefined, 100) });
  const diagnostics = useMemo(() => store.events.map((event) => JSON.stringify(event)), [store.events]);

  useEffect(() => { if (settingsQuery.data) setSettings(settingsQuery.data); }, [settingsQuery.data, setSettings]);
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: light)");
    const applyTheme = () => {
      const theme = themeSetting ?? "dark";
      const light = theme === "light" || (theme === "system" && media.matches);
      document.documentElement.classList.toggle("light", light);
      document.documentElement.classList.toggle("dark", !light);
      document.querySelector('meta[name="theme-color"]')?.setAttribute("content", light ? "#ffffff" : "#111112");
    };
    applyTheme();
    media.addEventListener("change", applyTheme);
    return () => media.removeEventListener("change", applyTheme);
  }, [themeSetting]);
  useEffect(() => {
    const unlisteners = [
      listen<AgentEvent>("agent-event", ({ payload }) => appendEvent(payload)),
      listen<PendingApproval>("permission-request", ({ payload }) => setApproval(payload)),
      listen<AgentFinished>("agent-finished", async ({ payload }) => {
        setRunning(false);
        try {
          setSession(await api.session(payload.sessionId));
          if (workspace) {
            const [overview, diff] = await Promise.all([api.overview(workspace), api.gitDiff(workspace).catch(() => "")]);
            useAppStore.getState().setWorkspace(workspace, overview);
            useAppStore.getState().setDiff(diff);
          }
          if (payload.error && !payload.cancelled) setError(payload.error);
          if (!payload.error && notifyOnComplete) {
            setCompletionNotice(payload.cancelled ? "Agent run cancelled." : "Task completed successfully.");
            window.setTimeout(() => setCompletionNotice(undefined), 4000);
          }
        } catch (cause) { setError(String(cause)); }
        await queryClient.invalidateQueries({ queryKey: ["sessions"] });
      }),
    ];
    return () => { for (const unlisten of unlisteners) void unlisten.then((dispose) => dispose()); };
  }, [appendEvent, notifyOnComplete, queryClient, setApproval, setError, setRunning, setSession, workspace]);
  useEffect(() => {
    if (settingsQuery.error) setError(`Unable to load settings: ${String(settingsQuery.error)}`);
    else if (providersQuery.error) setError(`Unable to load providers: ${String(providersQuery.error)}`);
    else if (authQuery.error) setError(`Unable to load authentication status: ${String(authQuery.error)}`);
  }, [authQuery.error, providersQuery.error, setError, settingsQuery.error]);

  const saveSettings = useCallback(async (settings: Settings) => { await api.saveSettings(settings); setSettings(settings); }, [setSettings]);
  const openProject = useCallback(async (path?: string) => {
    try {
      const selected = path ?? await open({ directory: true, multiple: false, title: "Open a project in Kernex" });
      if (!selected || Array.isArray(selected)) return;
      const [overview, diff] = await Promise.all([api.overview(selected), api.gitDiff(selected).catch(() => "")]);
      useAppStore.getState().setWorkspace(overview.path, overview);
      useAppStore.getState().setSession(undefined);
      useAppStore.getState().setDiff(diff);
      const currentSettings = useAppStore.getState().settings;
      if (currentSettings) {
        const recent = [overview.path, ...currentSettings.recent_projects.filter((item) => item !== overview.path)].slice(0, 20);
        await saveSettings({ ...currentSettings, recent_projects: recent });
      }
    } catch (cause) { setError(String(cause)); }
  }, [saveSettings, setError]);
  const loadSession = useCallback(async (session: SessionRecord) => {
    try {
      if (session.workspace_path !== useAppStore.getState().workspace) {
        const [overview, diff] = await Promise.all([api.overview(session.workspace_path), api.gitDiff(session.workspace_path).catch(() => "")]);
        useAppStore.getState().setWorkspace(overview.path, overview);
        useAppStore.getState().setDiff(diff);
      }
      setSession(await api.session(session.id));
    } catch (cause) { setError(String(cause)); }
  }, [setError, setSession]);
  const newSession = useCallback(() => { setSession(undefined); requestAnimationFrame(() => window.dispatchEvent(new Event("kernex:focus-composer"))); }, [setSession]);
  const runAgent = useCallback(async (submission: ComposerSubmission) => {
    const current = useAppStore.getState();
    if (!current.workspace || !current.settings) { setError("Open a project and configure a model first."); return; }
    if (!current.settings.provider.model.trim()) { setError("Select a model in Settings before starting an agent."); return; }
    const task = prepareTask(submission);
    current.resetStream(); current.setRunning(true);
    try {
      const id = await api.startAgent({ workspace: current.workspace, task, provider: current.settings.provider.name, model: current.settings.provider.model, baseUrl: current.settings.provider.base_url, authProfile: current.settings.provider.auth_profile, permissionMode: current.settings.permission_mode, maxSteps: submission.mode === "plan" ? 12 : submission.mode === "review" ? 18 : 24, sessionId: current.session?.id });
      const prior = current.session;
      const local: SessionRecord = prior ? { ...prior, id, messages: [...prior.messages, { role: "user", content: task, tool_calls: [] }], status: "active", updated_at: new Date().toISOString() } : { id, workspace_path: current.workspace, provider: current.settings.provider.name, model: current.settings.provider.model, messages: [{ role: "user", content: task, tool_calls: [] }], tool_calls: [], tool_results: [], permission_decisions: [], created_at: new Date().toISOString(), updated_at: new Date().toISOString(), token_usage: {}, generated_diffs: [], status: "active" };
      current.setSession(local);
    } catch (cause) { current.setRunning(false); setError(String(cause)); }
  }, [setError]);
  const cancelAgent = useCallback(async () => { try { await api.cancelAgent(); } catch (cause) { setError(String(cause)); } }, [setError]);
  const refreshOverview = useCallback(async () => { const current = useAppStore.getState(); if (!current.workspace) return; current.setWorkspace(current.workspace, await api.overview(current.workspace)); }, []);
  const selectFile = useCallback(async (path: string) => {
    const current = useAppStore.getState();
    current.setContextTab("files"); current.setFile(path, "Loading…");
    try { current.setFile(path, await api.readFile(current.workspace, path)); }
    catch (cause) { current.setFile(path, `Unable to read file: ${String(cause)}`); }
  }, []);
  const deleteSession = useCallback(async (session: SessionRecord) => { try { await api.deleteSession(session.id); if (useAppStore.getState().session?.id === session.id) setSession(undefined); await queryClient.invalidateQueries({ queryKey: ["sessions"] }); } catch (cause) { setError(String(cause)); } }, [queryClient, setError, setSession]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const mod = event.ctrlKey || event.metaKey;
      if (mod && event.key.toLowerCase() === "k") { event.preventDefault(); store.setCommandPaletteOpen(true); }
      if (mod && event.key.toLowerCase() === "n") { event.preventDefault(); newSession(); }
      if (mod && event.key.toLowerCase() === "o") { event.preventDefault(); void openProject(); }
      if (mod && event.key === ",") { event.preventDefault(); store.setSettingsOpen(true); }
      if (mod && !event.shiftKey && event.key.toLowerCase() === "b") { event.preventDefault(); store.cycleSidebar(); }
      if (mod && event.shiftKey && event.key.toLowerCase() === "b") { event.preventDefault(); store.setContextOpen(!useAppStore.getState().contextOpen); }
      if (mod && event.key.toLowerCase() === "f") { event.preventDefault(); window.dispatchEvent(new Event("kernex:search-messages")); }
      if (mod && event.key.toLowerCase() === "l") { event.preventDefault(); window.dispatchEvent(new Event("kernex:focus-composer")); }
      if (mod && event.key === "." && useAppStore.getState().running) { event.preventDefault(); void cancelAgent(); }
      if (event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        const sessions = sessionsQuery.data ?? [];
        if (!sessions.length) return;
        event.preventDefault();
        const currentIndex = sessions.findIndex((session) => session.id === useAppStore.getState().session?.id);
        const delta = event.key === "ArrowUp" ? -1 : 1;
        const next = sessions[(Math.max(0, currentIndex) + delta + sessions.length) % sessions.length];
        void loadSession(next);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancelAgent, loadSession, newSession, openProject, sessionsQuery.data, store]);

  const sessions = sessionsQuery.data ?? [];
  return <TooltipProvider delayDuration={250}>
    <div className="flex h-full min-w-0 flex-col">
      <AppTitlebar workspace={workspace} />
      {store.error && <Alert className="m-2 flex w-auto shrink-0 items-start gap-3 rounded-md border-destructive/35 bg-destructive/5 p-3"><AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" /><div className="min-w-0 flex-1"><AlertTitle>Something needs attention</AlertTitle><AlertDescription className="break-words">{store.error}</AlertDescription></div><Button variant="ghost" size="sm" onClick={() => store.setSettingsOpen(true)}><Settings2 className="h-3.5 w-3.5" />Settings</Button><Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => store.setError(undefined)} aria-label="Dismiss error"><X className="h-3.5 w-3.5" /></Button></Alert>}
      {completionNotice && <div role="status" className="fixed right-4 top-12 z-40 flex items-center gap-2 rounded-md border bg-popover px-3 py-2 text-xs shadow-lg"><CheckCircle2 className="h-4 w-4 text-success" />{completionNotice}</div>}
      <main className="flex min-h-0 min-w-0 flex-1">
        <AppSidebar sessions={sessions} settings={store.settings} auth={authQuery.data ?? []} codexConnected={Boolean(codexAccountQuery.data?.account)} activeSession={store.session?.id} workspace={workspace} onNewSession={newSession} onOpenProject={(path) => void openProject(path)} onSession={(session) => void loadSession(session)} onDelete={(id) => { const session = sessions.find((item) => item.id === id); if (session) setPendingDelete(session); }} />
        <ChatWorkspace sessions={sessions} onRun={runAgent} onCancel={cancelAgent} onUpdateSettings={saveSettings} onOpenProject={(path) => void openProject(path)} onSession={(session) => void loadSession(session)} onOpenFile={(path) => void selectFile(path)} />
        <ContextPanel onFile={(path) => void selectFile(path)} />
      </main>
      <StatusBar />
      <PermissionDialog />
      {store.settings && <Suspense fallback={null}><SettingsSheet key={JSON.stringify(store.settings)} settings={store.settings} providerInfo={providersQuery.data ?? []} auth={authQuery.data ?? []} overview={store.overview} diagnostics={diagnostics} onSave={saveSettings} refreshAuth={async () => { await authQuery.refetch(); await codexAccountQuery.refetch(); }} refreshOverview={refreshOverview} /></Suspense>}
      <AppCommandPalette sessions={sessions} settings={store.settings} overview={store.overview} onNewSession={newSession} onOpenProject={() => void openProject()} onSession={(session) => void loadSession(session)} onFile={(path) => void selectFile(path)} />
      <ConfirmationDialog open={Boolean(pendingDelete)} title="Delete this session?" description="This removes the saved conversation and its audit trail from local Kernex storage. The project files are not changed." confirmLabel="Delete session" destructive onOpenChange={(open) => { if (!open) setPendingDelete(undefined); }} onConfirm={() => { if (pendingDelete) void deleteSession(pendingDelete); }} />
    </div>
  </TooltipProvider>;
}
