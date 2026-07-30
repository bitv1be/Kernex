import { AlertCircle, Bell, Bot, Check, CheckCircle2, ChevronRight, ExternalLink, FileKey2, GitBranch, Info, KeyRound, Keyboard, LoaderCircle, LockKeyhole, LogOut, Network, Package, Palette, Plug, RefreshCw, RotateCcw, Save, Server, Shield, ShieldCheck, Sparkles, Star, TerminalSquare, UserRound, Workflow, Wrench } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { KeyboardKey } from "@/components/shared/keyboard-key";
import { StatusIndicator } from "@/components/shared/status-indicator";
import { api } from "@/lib/api";
import { useAppStore } from "@/lib/store";
import type { AuthStatus, CodexAccountStatus, CodexRateLimits, PermissionMode, ProviderKind, ProviderModel, ProviderSummary, Settings, WorkspaceOverview } from "@/lib/types";

const providers: ProviderKind[] = ["codex", "openai-compatible", "anthropic", "gemini", "local", "custom"];
export type CodexAuthPhase = "loading" | "idle" | "signing-in" | "signing-out" | "refreshing";

const sections = [
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "providers", label: "AI providers", icon: Network },
  { id: "models", label: "Models", icon: Bot },
  { id: "oauth", label: "OAuth authentication", icon: ExternalLink },
  { id: "api-keys", label: "API keys", icon: KeyRound },
  { id: "agent", label: "Agent behavior", icon: Workflow },
  { id: "permissions", label: "Permissions", icon: Shield },
  { id: "terminal", label: "Terminal", icon: TerminalSquare },
  { id: "git", label: "Git", icon: GitBranch },
  { id: "mcp", label: "MCP servers", icon: Server },
  { id: "plugins", label: "Plugins", icon: Plug },
  { id: "shortcuts", label: "Keyboard shortcuts", icon: Keyboard },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "updates", label: "Updates", icon: Package },
  { id: "privacy", label: "Privacy", icon: LockKeyhole },
  { id: "about", label: "Application information", icon: Info },
  { id: "diagnostics", label: "Diagnostics", icon: Wrench },
] as const;

export function SettingsSheet({ settings, providerInfo, auth, overview, diagnostics, onSave, refreshAuth, refreshOverview }: {
  settings: Settings;
  providerInfo: ProviderSummary[];
  auth: AuthStatus[];
  overview?: WorkspaceOverview;
  diagnostics: string[];
  onSave: (settings: Settings) => Promise<void>;
  refreshAuth: () => Promise<void>;
  refreshOverview: () => Promise<void>;
}) {
  const open = useAppStore((state) => state.settingsOpen);
  const section = useAppStore((state) => state.settingsSection);
  const setOpen = useAppStore((state) => state.setSettingsOpen);
  const sidebarMode = useAppStore((state) => state.sidebarMode);
  const setSidebarMode = useAppStore((state) => state.setSidebarMode);
  const contextOpen = useAppStore((state) => state.contextOpen);
  const setContextOpen = useAppStore((state) => state.setContextOpen);
  const favoriteModels = useAppStore((state) => state.favoriteModels);
  const toggleFavoriteModel = useAppStore((state) => state.toggleFavoriteModel);
  const notifyOnComplete = useAppStore((state) => state.notifyOnComplete);
  const setNotifyOnComplete = useAppStore((state) => state.setNotifyOnComplete);
  const [draft, setDraft] = useState(settings);
  const [profile, setProfile] = useState("personal");
  const [secret, setSecret] = useState("");
  const [variable, setVariable] = useState("OPENAI_API_KEY");
  const [clientId, setClientId] = useState("");
  const [googleProject, setGoogleProject] = useState("");
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<string>();
  const [models, setModels] = useState<ProviderModel[]>([]);
  const [modelSearch, setModelSearch] = useState("");
  const [discovering, setDiscovering] = useState(false);
  const [saving, setSaving] = useState(false);
  const [codexAccount, setCodexAccount] = useState<CodexAccountStatus>();
  const [codexLimits, setCodexLimits] = useState<CodexRateLimits>();
  const [codexPhase, setCodexPhase] = useState<CodexAuthPhase>(settings.provider.name === "codex" ? "loading" : "idle");
  const [projectConfig, setProjectConfig] = useState("");
  const activeSection = sections.some((item) => item.id === section) ? section : "appearance";
  const provider = providerInfo.find((item) => item.kind === draft.provider.name);
  const filteredModels = useMemo(() => models.filter((model) => `${model.id} ${model.display_name ?? ""}`.toLowerCase().includes(modelSearch.toLowerCase())).sort((left, right) => Number(favoriteModels.includes(right.id)) - Number(favoriteModels.includes(left.id))), [favoriteModels, modelSearch, models]);

  useEffect(() => {
    let active = true;
    if (overview) void api.projectConfig(overview.path).then((value) => { if (active) setProjectConfig(value); }).catch((cause) => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [overview]);
  useEffect(() => {
    let active = true;
    if (draft.provider.name === "codex") {
      void Promise.all([api.codexAccount(), api.codexRateLimits().catch(() => undefined)]).then(([account, limits]) => { if (active) { setCodexAccount(account); setCodexLimits(limits); } }).catch((cause) => { if (active) setError(String(cause)); }).finally(() => { if (active) setCodexPhase("idle"); });
    }
    return () => { active = false; };
  }, [draft.provider.name]);

  const updateProvider = (patch: Partial<Settings["provider"]>) => setDraft((current) => ({ ...current, provider: { ...current.provider, ...patch } }));
  const selectProvider = (name: ProviderKind) => {
    const info = providerInfo.find((item) => item.kind === name);
    setModels([]);
    setError(undefined);
    if (name === "codex") setCodexPhase("loading");
    updateProvider({ name, base_url: info?.base_url, auth_profile: name === "codex" ? undefined : draft.provider.auth_profile });
  };
  const save = async () => {
    setSaving(true); setError(undefined); setStatus(undefined);
    try { await onSave(draft); setStatus("Settings saved."); }
    catch (cause) { setError(String(cause)); }
    finally { setSaving(false); }
  };
  const discover = async () => {
    setDiscovering(true); setError(undefined);
    try { setModels(await api.discoverModels(draft.provider.name, draft.provider.model, draft.provider.base_url, draft.provider.auth_profile)); }
    catch (cause) { setError(String(cause)); }
    finally { setDiscovering(false); }
  };
  const login = async (method: "key" | "env" | "oauth") => {
    setError(undefined); setStatus(undefined);
    try {
      if (method === "key") await api.loginApiKey(profile, draft.provider.name, secret);
      if (method === "env") await api.loginEnvironment(profile, draft.provider.name, variable);
      if (method === "oauth") await api.loginOAuth(profile, draft.provider.name, clientId, googleProject);
      setSecret(""); setStatus("Authentication profile connected."); await refreshAuth();
    } catch (cause) { setError(String(cause)); }
  };
  const refreshCodex = async () => {
    setError(undefined); setCodexPhase("refreshing");
    try { const [account, limits] = await Promise.all([api.codexAccount(), api.codexRateLimits().catch(() => undefined)]); setCodexAccount(account); setCodexLimits(limits); }
    catch (cause) { setError(String(cause)); }
    finally { setCodexPhase("idle"); }
  };
  const loginCodex = async () => {
    setError(undefined); setCodexPhase("signing-in");
    try { setCodexAccount(await api.codexLogin()); setCodexLimits(await api.codexRateLimits().catch(() => undefined)); }
    catch (cause) { setError(String(cause)); }
    finally { setCodexPhase("idle"); }
  };
  const logoutCodex = async () => {
    setError(undefined); setCodexPhase("signing-out");
    try { await api.codexLogout(); setCodexAccount(await api.codexAccount()); setCodexLimits(undefined); }
    catch (cause) { setError(String(cause)); }
    finally { setCodexPhase("idle"); }
  };
  const saveProjectConfig = async () => {
    if (!overview) return;
    setError(undefined); setStatus(undefined);
    try { await api.saveProjectConfig(overview.path, projectConfig); setStatus("Project extension configuration saved."); await refreshOverview(); }
    catch (cause) { setError(String(cause)); }
  };

  return <Sheet open={open} onOpenChange={(value) => setOpen(value)}>
    <SheetContent className="flex w-[min(1080px,calc(100vw-40px))] max-w-none flex-col p-0">
      <SheetHeader className="mb-0 h-16 shrink-0 justify-center border-b px-6 pr-12"><SheetTitle className="text-base font-semibold">Settings</SheetTitle><SheetDescription className="text-xs text-muted-foreground">Shared configuration for the Kernex CLI and desktop application.</SheetDescription></SheetHeader>
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Settings sections" className="w-56 shrink-0 overflow-y-auto border-r bg-muted/15 p-2">{sections.map(({ id, label, icon: Icon }) => <button key={id} className={`flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-[11px] ${activeSection === id ? "bg-accent font-medium text-foreground" : "text-muted-foreground hover:bg-accent/60 hover:text-foreground"}`} onClick={() => setOpen(true, id)}><Icon className="h-3.5 w-3.5 shrink-0" /><span className="min-w-0 flex-1 truncate">{label}</span>{activeSection === id && <ChevronRight className="h-3 w-3" />}</button>)}</nav>
        <main className="min-w-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-2xl space-y-6 px-8 py-7">
            {error && <Alert className="border-destructive/40 bg-destructive/5"><AlertCircle className="mb-2 h-4 w-4 text-destructive" /><AlertTitle>Action failed</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
            {status && <Alert className="border-success/30 bg-success/5"><CheckCircle2 className="mb-2 h-4 w-4 text-success" /><AlertTitle>Done</AlertTitle><AlertDescription>{status}</AlertDescription></Alert>}
            {activeSection === "appearance" && <SettingsPage title="Appearance" description="Choose a theme and restore the desktop layout.">
              <Field label="Theme" description="Dark is the default for new installations."><Select value={draft.theme} onValueChange={(theme) => setDraft({ ...draft, theme })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="dark">Dark</SelectItem><SelectItem value="light">Light</SelectItem><SelectItem value="system">System</SelectItem></SelectContent></Select></Field>
              <Field label="Sidebar" description="The selected state persists between launches."><Select value={sidebarMode} onValueChange={setSidebarMode}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="expanded">Expanded</SelectItem><SelectItem value="compact">Compact icons</SelectItem><SelectItem value="hidden">Hidden</SelectItem></SelectContent></Select></Field>
              <SwitchRow label="Context panel" description="Show files, diffs, terminal, activity, and active context." checked={contextOpen} onChange={setContextOpen} />
              <Button variant="outline" onClick={() => { setSidebarMode("expanded"); setContextOpen(true); useAppStore.getState().setSidebarWidth(272); useAppStore.getState().setContextWidth(420); }}><RotateCcw className="h-4 w-4" />Reset panel layout</Button>
            </SettingsPage>}
            {activeSection === "providers" && <SettingsPage title="AI providers" description="Configure the provider used by the current and future sessions.">
              <Field label="Provider"><Select value={draft.provider.name} onValueChange={selectProvider}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{providers.map((item) => <SelectItem key={item} value={item}>{item}</SelectItem>)}</SelectContent></Select></Field>
              {draft.provider.name !== "codex" && <Field label="API base URL" description="Leave the provider default unless you use a compatible or local endpoint."><Input value={draft.provider.base_url ?? ""} onChange={(event) => updateProvider({ base_url: event.target.value })} /></Field>}
              <div className="grid gap-2">{providerInfo.map((item) => { const connected = item.kind === "local" || item.kind === "codex" ? item.kind === draft.provider.name : auth.some((entry) => entry.profile.provider === item.kind && entry.credential_available && !entry.expired); return <button key={item.kind} className={`flex items-center gap-3 rounded-md border p-3 text-left ${draft.provider.name === item.kind ? "bg-muted/50" : "hover:bg-muted/25"}`} onClick={() => selectProvider(item.kind)}><Network className="h-4 w-4 text-muted-foreground" /><div className="min-w-0 flex-1"><div className="text-xs font-medium">{item.kind}</div><div className="truncate text-[9px] text-muted-foreground">{item.base_url || "Managed connection"}</div></div><StatusIndicator status={connected ? "success" : "offline"} label={connected ? "Ready" : "Not connected"} /></button>; })}</div>
            </SettingsPage>}
            {activeSection === "models" && <SettingsPage title="Models" description="Discover, search, favorite, and select real models from the configured provider.">
              <div className="flex gap-2"><Input value={modelSearch} onChange={(event) => setModelSearch(event.target.value)} placeholder="Search discovered models" /><Button variant="outline" onClick={() => void discover()} disabled={discovering}>{discovering ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}Discover</Button></div>
              <Field label="Current model"><Input value={draft.provider.model} onChange={(event) => updateProvider({ model: event.target.value })} placeholder="Model ID" /></Field>
              <div className="max-h-[430px] space-y-1 overflow-y-auto">{filteredModels.map((model) => <div key={model.id} className={`flex items-start gap-3 rounded-md border p-3 ${draft.provider.model === model.id ? "bg-muted/50" : ""}`}><button className="min-w-0 flex-1 text-left" onClick={() => updateProvider({ model: model.id })}><div className="flex items-center gap-2"><span className="truncate text-xs font-medium">{model.display_name || model.id}</span>{model.is_default && <Badge>default</Badge>}{draft.provider.model === model.id && <Check className="h-3.5 w-3.5" />}</div><div className="mt-1 flex flex-wrap gap-2 text-[9px] text-muted-foreground"><span>{model.id}</span>{model.input_token_limit && <span>{model.input_token_limit.toLocaleString()} context</span>}{model.output_token_limit && <span>{model.output_token_limit.toLocaleString()} max output</span>}{model.owned_by && <span>{model.owned_by}</span>}</div>{model.description && <p className="mt-2 text-[10px] leading-4 text-muted-foreground">{model.description}</p>}</button><Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => toggleFavoriteModel(model.id)} aria-label={`${favoriteModels.includes(model.id) ? "Unfavorite" : "Favorite"} ${model.id}`}><Star className={`h-3.5 w-3.5 ${favoriteModels.includes(model.id) ? "fill-current" : ""}`} /></Button></div>)}{!models.length && <InfoPanel icon={Bot} title="Discover the provider catalog" description="Kernex requests the live model list using your configured endpoint and authentication profile. You can still enter a model ID directly." />}{models.length > 0 && !filteredModels.length && <InfoPanel icon={Bot} title="No matching models" description="Clear the search or discover the provider catalog again." />}</div>
            </SettingsPage>}
            {activeSection === "oauth" && <SettingsPage title="OAuth authentication" description="Browser sign-in is only shown for providers with a real implemented flow.">
              {draft.provider.name === "codex" ? <CodexAuthPanel account={codexAccount} limits={codexLimits} phase={codexPhase} error={error} onLogin={() => void loginCodex()} onLogout={() => void logoutCodex()} onRefresh={() => void refreshCodex()} /> : provider?.oauth_pkce ? <><Field label="Profile name"><Input value={profile} onChange={(event) => setProfile(event.target.value)} /></Field><Field label="Official OAuth client ID"><Input value={clientId} onChange={(event) => setClientId(event.target.value)} /></Field>{draft.provider.name === "gemini" && <Field label="Google Cloud project ID" description="Required by Google for API quota."><Input value={googleProject} onChange={(event) => setGoogleProject(event.target.value)} /></Field>}<Button onClick={() => void login("oauth")} disabled={!clientId || (draft.provider.name === "gemini" && !googleProject)}><ExternalLink className="h-4 w-4" />Open provider sign-in</Button></> : <InfoPanel icon={ExternalLink} title="OAuth is not available for this provider" description={`${draft.provider.name} has no official third-party OAuth flow implemented by Kernex. Use an API key, an environment reference, or choose a provider with managed OAuth.`} />}
            </SettingsPage>}
            {activeSection === "api-keys" && <SettingsPage title="API keys" description="Secrets are written to the native keyring and never displayed again after saving.">
              {draft.provider.name === "codex" ? <InfoPanel icon={ShieldCheck} title="Codex uses managed ChatGPT authentication" description="Kernex delegates subscription authentication and token refresh to the installed Codex CLI. No API key is requested here." /> : <><Field label="Profile name"><Input value={profile} onChange={(event) => setProfile(event.target.value)} /></Field><Tabs defaultValue="key"><TabsList className="grid w-full grid-cols-2"><TabsTrigger value="key">API key</TabsTrigger><TabsTrigger value="environment">Environment</TabsTrigger></TabsList><TabsContent value="key" className="space-y-3 pt-2"><Input type="password" autoComplete="new-password" value={secret} onChange={(event) => setSecret(event.target.value)} placeholder="Stored securely in the native keyring" /><Button onClick={() => void login("key")} disabled={!secret}><KeyRound className="h-4 w-4" />Store API key</Button></TabsContent><TabsContent value="environment" className="space-y-3 pt-2"><Input value={variable} onChange={(event) => setVariable(event.target.value)} placeholder="OPENAI_API_KEY" /><Button onClick={() => void login("env")} disabled={!variable}><FileKey2 className="h-4 w-4" />Use environment reference</Button></TabsContent></Tabs></>}
              <AuthProfiles auth={auth} provider={draft.provider.name} onRefresh={refreshAuth} />
            </SettingsPage>}
            {activeSection === "agent" && <SettingsPage title="Agent behavior" description="Control how Kernex approaches tasks and protected actions."><Field label="Default permission mode"><PermissionSelect value={draft.permission_mode} onChange={(permission_mode) => setDraft({ ...draft, permission_mode })} /></Field><InfoPanel icon={Workflow} title="Session modes live in the composer" description="Agent, plan-only, and review modes are selected per task so a mode change cannot silently alter an existing session." /></SettingsPage>}
            {activeSection === "permissions" && <SettingsPage title="Permissions" description="Every sensitive operation remains visible and auditable."><Field label="Permission mode"><PermissionSelect value={draft.permission_mode} onChange={(permission_mode) => setDraft({ ...draft, permission_mode })} /></Field><div className="space-y-2"><PermissionExplanation mode="read-only" text="Inspect files and project state without writes, processes, or network access." /><PermissionExplanation mode="ask" text="Ask before protected shell, file, network, Git, installation, and external-application actions." /><PermissionExplanation mode="auto-safe" text="Allow known low-risk actions and ask when impact or scope increases." /><PermissionExplanation mode="full-access" text="Allow broad actions for this run. Dangerous details remain visible in the event timeline." /></div></SettingsPage>}
            {activeSection === "terminal" && <SettingsPage title="Terminal" description="The integrated terminal uses argument-vector execution in the current workspace."><InfoPanel icon={TerminalSquare} title="Permissioned terminal" description="Commands run through the shared Rust permission gate. Standard output, standard error, exit status, and duration remain visible in the terminal and agent timeline." /><Button variant="outline" disabled={!overview} onClick={() => { useAppStore.getState().setContextTab("terminal"); setOpen(false); }}><TerminalSquare className="h-4 w-4" />Open terminal</Button></SettingsPage>}
            {activeSection === "git" && <SettingsPage title="Git" description="Repository state is read from the currently open workspace.">{overview?.isGitRepository ? <><InfoPanel icon={GitBranch} title="Git repository connected" description={overview.path} /><pre className="max-h-72 overflow-auto rounded-md border bg-code p-3 font-mono text-[10px] leading-5">{overview.gitStatus || "Working tree clean."}</pre></> : <InfoPanel icon={GitBranch} title="No Git repository selected" description="Open a Git-backed project to view branch and working-tree state." />}</SettingsPage>}
            {(activeSection === "mcp" || activeSection === "plugins") && <SettingsPage title={activeSection === "mcp" ? "MCP servers" : "Plugins"} description="Project extensions are validated and saved through the shared core.">
              <div className="flex flex-wrap gap-2">{overview?.mcpServers.map((name) => <Badge key={name} variant="outline"><Server className="h-3 w-3" />{name}</Badge>)}{overview?.languageServers.map((name) => <Badge key={name} variant="outline">LSP · {name}</Badge>)}{!overview?.mcpServers.length && !overview?.languageServers.length && <span className="text-xs text-muted-foreground">No extensions currently configured.</span>}</div>
              <Field label=".kernex/config.toml" description="MCP, LSP, and plugin configuration for this workspace."><Textarea value={projectConfig} onChange={(event) => setProjectConfig(event.target.value)} disabled={!overview} className="min-h-72 font-mono text-[11px]" placeholder="Open a project to manage its extensions." /></Field><Button onClick={() => void saveProjectConfig()} disabled={!overview}><Save className="h-4 w-4" />Validate and save project config</Button>
            </SettingsPage>}
            {activeSection === "shortcuts" && <SettingsPage title="Keyboard shortcuts" description="Shortcuts follow standard desktop conventions and are currently fixed."><ShortcutTable /></SettingsPage>}
            {activeSection === "notifications" && <SettingsPage title="Notifications" description="Choose when Kernex should surface completion feedback."><SwitchRow label="Task completion feedback" description="Show a short in-app completion notice after background agent work." checked={notifyOnComplete} onChange={setNotifyOnComplete} /><InfoPanel icon={Bell} title="Critical errors stay visible" description="Errors that require action remain in the workspace instead of disappearing as temporary notifications." /></SettingsPage>}
            {activeSection === "updates" && <SettingsPage title="Updates" description="Distribution and release information for this installation."><InfoPanel icon={Package} title="Kernex 0.1.0" description="This build is distributed as a native Tauri application. Updates follow the repository release channel; no app-store dependency is required." /></SettingsPage>}
            {activeSection === "privacy" && <SettingsPage title="Privacy" description="Kernex keeps credentials and workspace access explicit."><InfoPanel icon={LockKeyhole} title="Secrets stay out of sessions" description="API keys and OAuth tokens are stored in native secure storage. Authentication values and sensitive file contents are excluded from diagnostics." /><InfoPanel icon={ShieldCheck} title="Permission decisions are auditable" description="Protected commands, file operations, network access, and external tools include their resource and risk before execution." /></SettingsPage>}
            {activeSection === "about" && <SettingsPage title="Application information" description="Kernex is a provider-independent AI coding agent."><div className="grid gap-3 sm:grid-cols-2"><InfoCard label="Desktop" value="Tauri 2 · React 19" /><InfoCard label="Shared core" value="Rust 2024" /><InfoCard label="State" value="Zustand · native sessions" /><InfoCard label="License" value="MIT" /></div><p className="text-xs leading-5 text-muted-foreground">The CLI and desktop clients share the same provider, authentication, session, permission, tool, MCP, and LSP implementation.</p></SettingsPage>}
            {activeSection === "diagnostics" && <SettingsPage title="Diagnostics" description="Agent events are shown without credentials or sensitive file contents."><pre className="max-h-[520px] overflow-auto whitespace-pre-wrap rounded-md border bg-code p-3 font-mono text-[10px] leading-5">{diagnostics.join("\n") || "No events yet."}</pre></SettingsPage>}
          </div>
        </main>
      </div>
      <div className="flex h-14 shrink-0 items-center justify-between border-t bg-card px-5"><span className="text-[10px] text-muted-foreground">Changes to provider, model, permissions, and theme are shared with the CLI.</span><div className="flex gap-2"><Button variant="ghost" onClick={() => { setDraft(settings); setError(undefined); setStatus("Unsaved changes reverted."); }}><RotateCcw className="h-4 w-4" />Reset</Button><Button onClick={() => void save()} disabled={saving}>{saving ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Save className="h-4 w-4" />}{saving ? "Saving…" : "Save settings"}</Button></div></div>
    </SheetContent>
  </Sheet>;
}

function SettingsPage({ title, description, children }: { title: string; description: string; children: React.ReactNode }) {
  return <section><div className="mb-6"><h2 className="text-lg font-semibold tracking-tight">{title}</h2><p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p></div><div className="space-y-5">{children}</div></section>;
}

function Field({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) {
  return <label className="block space-y-1.5"><span className="text-xs font-medium">{label}</span>{description && <span className="block text-[10px] leading-4 text-muted-foreground">{description}</span>}{children}</label>;
}

function InfoPanel({ icon: Icon, title, description }: { icon: typeof Info; title: string; description: string }) {
  return <div className="flex gap-3 rounded-md border bg-muted/20 p-4"><Icon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" /><div><h3 className="text-xs font-medium">{title}</h3><p className="mt-1 text-[10px] leading-5 text-muted-foreground">{description}</p></div></div>;
}

function InfoCard({ label, value }: { label: string; value: string }) {
  return <div className="rounded-md border p-3"><div className="text-[9px] uppercase tracking-wider text-muted-foreground">{label}</div><div className="mt-1 text-xs font-medium">{value}</div></div>;
}

function SwitchRow({ label, description, checked, onChange }: { label: string; description: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return <div className="flex items-center justify-between gap-5 rounded-md border p-4"><div><div className="text-xs font-medium">{label}</div><p className="mt-1 text-[10px] leading-4 text-muted-foreground">{description}</p></div><button role="switch" aria-checked={checked} className={`relative h-5 w-9 shrink-0 rounded-full border transition-colors ${checked ? "bg-foreground" : "bg-muted"}`} onClick={() => onChange(!checked)}><span className={`absolute top-0.5 h-3.5 w-3.5 rounded-full transition-transform ${checked ? "left-[18px] bg-background" : "left-0.5 bg-muted-foreground"}`} /></button></div>;
}

function PermissionSelect({ value, onChange }: { value: PermissionMode; onChange: (value: PermissionMode) => void }) {
  return <Select value={value} onValueChange={onChange}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="read-only">Read only</SelectItem><SelectItem value="ask">Ask on protected actions</SelectItem><SelectItem value="auto-safe">Automatically allow safe actions</SelectItem><SelectItem value="full-access">Full access</SelectItem></SelectContent></Select>;
}

function PermissionExplanation({ mode, text }: { mode: PermissionMode; text: string }) {
  return <div className="flex items-start gap-3 rounded-md border px-3 py-2.5"><Shield className="mt-0.5 h-3.5 w-3.5 text-muted-foreground" /><div><div className="text-[10px] font-medium">{mode.replaceAll("-", " ")}</div><p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">{text}</p></div></div>;
}

function AuthProfiles({ auth, provider, onRefresh }: { auth: AuthStatus[]; provider: ProviderKind; onRefresh: () => Promise<void> }) {
  const profiles = auth.filter((status) => status.profile.provider === provider);
  if (!profiles.length) return null;
  return <div className="space-y-2 border-t pt-5"><h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Saved profiles</h3>{profiles.map((status) => <div key={status.profile.name} className="flex items-center gap-3 rounded-md border p-3"><KeyRound className="h-4 w-4 text-muted-foreground" /><div className="min-w-0 flex-1"><div className="flex items-center gap-2 text-xs font-medium">{status.profile.name}{status.active && <Badge variant="success">active</Badge>}</div><div className="mt-0.5 text-[9px] text-muted-foreground">{status.profile.method} · {status.credential_available ? status.expired ? "expired" : "ready" : "unavailable"}</div></div><Button variant="ghost" size="sm" onClick={() => void api.useAuth(status.profile.name).then(onRefresh)}>Use</Button><Button variant="ghost" size="icon" onClick={() => void api.logout(status.profile.name).then(onRefresh)} aria-label={`Disconnect ${status.profile.name}`}><LogOut className="h-3.5 w-3.5" /></Button></div>)}</div>;
}

function ShortcutTable() {
  const shortcuts = [["New session", "Ctrl", "N"], ["Open project", "Ctrl", "O"], ["Command palette", "Ctrl", "K"], ["Global search", "Ctrl", "F"], ["Settings", "Ctrl", ","], ["Toggle sidebar", "Ctrl", "B"], ["Toggle context panel", "Ctrl", "Shift", "B"], ["Focus composer", "Ctrl", "L"], ["Send message", "Enter"], ["New line", "Shift", "Enter"], ["Stop generation", "Ctrl", "."], ["Previous session", "Alt", "ArrowUp"], ["Next session", "Alt", "ArrowDown"]];
  return <div className="divide-y rounded-md border">{shortcuts.map(([label, ...keys]) => <div key={label} className="flex items-center justify-between px-3 py-2.5"><span className="text-xs">{label}</span><span className="flex gap-1">{keys.map((key) => <KeyboardKey key={key}>{key}</KeyboardKey>)}</span></div>)}</div>;
}

export function CodexAuthPanel({ account, limits, phase, error, onLogin, onLogout, onRefresh }: { account?: CodexAccountStatus; limits?: CodexRateLimits; phase: CodexAuthPhase; error?: string; onLogin: () => void; onLogout: () => void; onRefresh: () => void }) {
  const chatgptAccount = account?.account?.type === "chatgpt" ? account.account : undefined;
  const otherAuthActive = Boolean(account?.account && !chatgptAccount);
  const busy = phase !== "idle";
  if (phase === "loading") return <section aria-label="ChatGPT account" aria-busy="true" className="rounded-md border bg-muted/20 p-6"><div role="status" className="flex min-h-36 flex-col items-center justify-center gap-3 text-center"><LoaderCircle className="h-5 w-5 animate-spin text-muted-foreground" /><div><p className="text-sm font-medium">Checking your ChatGPT connection</p><p className="mt-1 text-xs text-muted-foreground">Reading the session managed by Codex…</p></div></div></section>;
  return <section aria-label="ChatGPT account" aria-busy={busy} className="space-y-4">
    {error && <Alert className="border-destructive/40 bg-destructive/5"><AlertTitle>Couldn’t connect to ChatGPT</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {chatgptAccount ? <><div className="rounded-md border"><div className="flex items-start justify-between gap-4 p-5"><div className="flex min-w-0 items-center gap-3"><div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border bg-muted"><UserRound className="h-4 w-4" /></div><div className="min-w-0"><p className="truncate text-sm font-semibold">{chatgptAccount.email ?? "ChatGPT account"}</p><p className="mt-0.5 text-xs text-muted-foreground">{formatPlanName(chatgptAccount.planType)}</p></div></div><Badge variant="success"><CheckCircle2 className="h-3 w-3" />Connected</Badge></div><div className="border-t bg-muted/20 px-5 py-3 text-xs text-muted-foreground">Codex manages and refreshes this session for Kernex.</div></div>{limits?.rateLimits?.primary && <div className="rounded-md border p-4"><div className="mb-3 flex items-center justify-between"><div><p className="text-sm font-medium">Codex usage</p><p className="text-xs text-muted-foreground">Included with your ChatGPT plan</p></div>{limits.rateLimitResetCredits && <Badge>{limits.rateLimitResetCredits.availableCount} resets available</Badge>}</div><UsageMeter window={limits.rateLimits.primary} /></div>}<div className="flex gap-2"><Button className="flex-1" variant="outline" onClick={onLogout} disabled={busy}><LogOut className="h-4 w-4" />{phase === "signing-out" ? "Signing out…" : "Sign out"}</Button><Button variant="outline" size="icon" aria-label="Refresh ChatGPT account" title="Refresh ChatGPT account" onClick={onRefresh} disabled={busy}>{phase === "refreshing" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}</Button></div></> : <div className="rounded-md border p-5"><div className="mb-5 flex items-start justify-between gap-4"><div className="flex h-10 w-10 items-center justify-center rounded-md bg-foreground text-background"><Sparkles className="h-4 w-4" /></div><Badge>{otherAuthActive ? "API key active" : "Not connected"}</Badge></div><h3 className="text-base font-semibold">Log in to ChatGPT</h3><p className="mt-1.5 text-sm leading-6 text-muted-foreground">{otherAuthActive ? "Codex is currently using API key authentication. Log in to ChatGPT to switch to your subscription-backed account." : "Use the models and usage limits included with your ChatGPT plan—no API key setup required."}</p><div className="my-5 space-y-3 rounded-md border bg-muted/20 p-3 text-xs text-muted-foreground"><div className="flex gap-2.5"><ExternalLink className="mt-0.5 h-4 w-4 shrink-0" /><span>Sign-in opens securely in your default browser.</span></div><div className="flex gap-2.5"><ShieldCheck className="mt-0.5 h-4 w-4 shrink-0" /><span>Codex stores and refreshes the session; Kernex never asks for your password.</span></div></div>{phase === "signing-in" && <div role="status" aria-live="polite" className="mb-3 flex items-start gap-2.5 rounded-md border p-3 text-xs"><LoaderCircle className="mt-0.5 h-4 w-4 shrink-0 animate-spin" /><div><p className="font-medium">Complete sign-in in your browser</p><p className="mt-0.5 text-muted-foreground">This panel will update automatically when ChatGPT finishes.</p></div></div>}<Button className="h-10 w-full" onClick={onLogin} disabled={busy}>{phase === "signing-in" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <ExternalLink className="h-4 w-4" />}{phase === "signing-in" ? "Waiting for ChatGPT…" : "Continue with ChatGPT"}</Button></div>}
  </section>;
}

function UsageMeter({ window }: { window: NonNullable<NonNullable<CodexRateLimits["rateLimits"]>["primary"]> }) {
  const usedPercent = Math.min(100, Math.max(0, window.usedPercent));
  const reset = window.resetsAt ? new Date(window.resetsAt * 1000) : undefined;
  return <div className="space-y-2"><div className="flex justify-between text-xs"><span>{Math.round(usedPercent)}% used</span><span className="text-muted-foreground">{window.windowDurationMins ? `${window.windowDurationMins}-minute window` : "current window"}</span></div><div role="progressbar" aria-label="Codex usage" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(usedPercent)} className="h-1.5 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-foreground transition-[width]" style={{ width: `${usedPercent}%` }} /></div>{reset && !Number.isNaN(reset.getTime()) && <p className="text-[11px] text-muted-foreground">Resets {reset.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}</p>}</div>;
}

function formatPlanName(planType?: string) {
  if (!planType) return "ChatGPT plan";
  return `ChatGPT ${planType.replace(/[-_]/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase())}`;
}
