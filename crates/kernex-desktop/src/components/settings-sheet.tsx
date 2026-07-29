import { ArrowRight, CheckCircle2, ExternalLink, KeyRound, LoaderCircle, LogOut, Plug, RefreshCw, Save, Settings2, ShieldCheck, Sparkles, Stethoscope, UserRound } from "lucide-react";
import { useEffect, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api } from "@/lib/api";
import type { AuthStatus, CodexAccountStatus, CodexRateLimits, ProviderKind, ProviderModel, ProviderSummary, Settings, WorkspaceOverview } from "@/lib/types";

const providers: ProviderKind[] = ["codex", "openai-compatible", "anthropic", "gemini", "local", "custom"];
export type CodexAuthPhase = "loading" | "idle" | "signing-in" | "signing-out" | "refreshing";

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
  const [draft, setDraft] = useState(settings);
  const [profile, setProfile] = useState("personal");
  const [secret, setSecret] = useState("");
  const [variable, setVariable] = useState("OPENAI_API_KEY");
  const [clientId, setClientId] = useState("");
  const [googleProject, setGoogleProject] = useState("");
  const [authError, setAuthError] = useState<string>();
  const [activeTab, setActiveTab] = useState("general");
  const [models, setModels] = useState<ProviderModel[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const [codexAccount, setCodexAccount] = useState<CodexAccountStatus>();
  const [codexLimits, setCodexLimits] = useState<CodexRateLimits>();
  const [codexPhase, setCodexPhase] = useState<CodexAuthPhase>(settings.provider.name === "codex" ? "loading" : "idle");
  const [extensionConfig, setExtensionConfig] = useState("");
  const [extensionStatus, setExtensionStatus] = useState<string>();
  useEffect(() => {
    let active = true;
    if (overview) {
      void api.projectConfig(overview.path)
        .then((contents) => { if (active) setExtensionConfig(contents); })
        .catch((error) => { if (active) setExtensionStatus(String(error)); });
    }
    return () => { active = false; };
  }, [overview]);
  useEffect(() => {
    let active = true;
    if (draft.provider.name === "codex") {
      void Promise.all([
        api.codexAccount(),
        api.codexRateLimits().catch(() => undefined),
      ]).then(([account, limits]) => {
        if (!active) return;
        setCodexAccount(account);
        setCodexLimits(limits);
      }).catch((error) => {
        if (active) setAuthError(String(error));
      }).finally(() => {
        if (active) setCodexPhase("idle");
      });
    }
    return () => { active = false; };
  }, [draft.provider.name]);
  const updateProvider = (patch: Partial<Settings["provider"]>) => setDraft({ ...draft, provider: { ...draft.provider, ...patch } });
  const selectProvider = (name: ProviderKind) => {
    const info = providerInfo.find((item) => item.kind === name);
    if (name === "codex") {
      setAuthError(undefined);
      setCodexPhase("loading");
    }
    updateProvider({ name, base_url: info?.base_url, auth_profile: name === "codex" ? undefined : draft.provider.auth_profile });
  };
  const login = async (method: "key" | "env" | "oauth") => {
    setAuthError(undefined);
    try {
      if (method === "key") await api.loginApiKey(profile, draft.provider.name, secret);
      if (method === "env") await api.loginEnvironment(profile, draft.provider.name, variable);
      if (method === "oauth") await api.loginOAuth(profile, draft.provider.name, clientId, googleProject);
      setSecret("");
      await refreshAuth();
    } catch (error) { setAuthError(String(error)); }
  };
  const discover = async () => {
    setAuthError(undefined);
    setDiscovering(true);
    try {
      setModels(await api.discoverModels(draft.provider.name, draft.provider.model, draft.provider.base_url, draft.provider.auth_profile));
    } catch (error) { setAuthError(String(error)); }
    finally { setDiscovering(false); }
  };
  const refreshCodex = async () => {
    setAuthError(undefined);
    setCodexPhase("refreshing");
    try {
      const [account, limits] = await Promise.all([
        api.codexAccount(),
        api.codexRateLimits().catch(() => undefined),
      ]);
      setCodexAccount(account);
      setCodexLimits(limits);
    } catch (error) {
      setAuthError(String(error));
    } finally {
      setCodexPhase("idle");
    }
  };
  const loginCodex = async () => {
    setAuthError(undefined);
    setCodexPhase("signing-in");
    try {
      setCodexAccount(await api.codexLogin());
      setCodexLimits(await api.codexRateLimits().catch(() => undefined));
    }
    catch (error) { setAuthError(String(error)); }
    finally { setCodexPhase("idle"); }
  };
  const logoutCodex = async () => {
    setAuthError(undefined);
    setCodexPhase("signing-out");
    try {
      await api.codexLogout();
      setCodexAccount(await api.codexAccount());
      setCodexLimits(undefined);
    }
    catch (error) { setAuthError(String(error)); }
    finally { setCodexPhase("idle"); }
  };
  const saveExtensions = async () => {
    if (!overview) return;
    setExtensionStatus(undefined);
    try {
      await api.saveProjectConfig(overview.path, extensionConfig);
      setExtensionStatus("Project extension configuration saved.");
      await refreshOverview();
    } catch (error) { setExtensionStatus(String(error)); }
  };
  return (
    <Sheet>
      <SheetTrigger asChild><Button variant="ghost" size="icon" title="Settings"><Settings2 className="h-4 w-4" /></Button></SheetTrigger>
      <SheetContent>
        <SheetHeader><SheetTitle className="text-lg font-semibold">Settings</SheetTitle><SheetDescription className="text-sm text-muted-foreground">Shared by the Kernex CLI and desktop application.</SheetDescription></SheetHeader>
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList className="grid w-full grid-cols-4"><TabsTrigger value="general">General</TabsTrigger><TabsTrigger value="auth">{draft.provider.name === "codex" ? "ChatGPT" : "Auth"}</TabsTrigger><TabsTrigger value="mcp">MCP</TabsTrigger><TabsTrigger value="logs">Logs</TabsTrigger></TabsList>
          <TabsContent value="general" className="space-y-4 pt-3">
            <Field label="Provider"><Select value={draft.provider.name} onValueChange={selectProvider}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{providers.map((provider) => <SelectItem key={provider} value={provider}>{provider}</SelectItem>)}</SelectContent></Select></Field>
            <Field label="Model"><div className="flex gap-2"><Input list="kernex-model-catalog" value={draft.provider.model} onChange={(event) => updateProvider({ model: event.target.value })} placeholder="Model ID" /><Button type="button" variant="outline" size="icon" title="Discover models" onClick={() => void discover()} disabled={discovering}><RefreshCw className={`h-4 w-4 ${discovering ? "animate-spin" : ""}`} /></Button><datalist id="kernex-model-catalog">{models.map((model) => <option key={model.id} value={model.id}>{model.display_name}</option>)}</datalist></div>{models.length > 0 && <span className="text-[11px] text-muted-foreground">{models.length} models discovered from the configured provider.</span>}</Field>
            {draft.provider.name === "codex" ? <Alert className="bg-muted/30"><AlertTitle>Use your ChatGPT account</AlertTitle><AlertDescription className="space-y-3"><p>The installed Codex CLI securely manages your browser sign-in and subscription-backed model access.</p><Button type="button" variant="outline" size="sm" onClick={() => setActiveTab("auth")}>{codexAccount?.account ? "Manage ChatGPT account" : "Log in to ChatGPT"}<ArrowRight className="h-3.5 w-3.5" /></Button></AlertDescription></Alert> : <><Field label="API base URL"><Input value={draft.provider.base_url ?? ""} onChange={(event) => updateProvider({ base_url: event.target.value })} placeholder="Provider default" /></Field><Field label="Authentication profile"><Select value={draft.provider.auth_profile ?? "none"} onValueChange={(value) => updateProvider({ auth_profile: value === "none" ? undefined : value })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="none">Environment/default</SelectItem>{auth.filter((status) => status.profile.provider === draft.provider.name).map((status) => <SelectItem key={status.profile.name} value={status.profile.name}>{status.profile.name}</SelectItem>)}</SelectContent></Select></Field></>}
            <Field label="Permission mode"><Select value={draft.permission_mode} onValueChange={(permission_mode: Settings["permission_mode"]) => setDraft({ ...draft, permission_mode })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="read-only">Read only</SelectItem><SelectItem value="ask">Ask on protected actions</SelectItem><SelectItem value="auto-safe">Automatically allow safe actions</SelectItem><SelectItem value="full-access">Full access</SelectItem></SelectContent></Select></Field>
            <Field label="Theme"><Select value={draft.theme} onValueChange={(theme) => setDraft({ ...draft, theme })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="system">System</SelectItem><SelectItem value="dark">Dark</SelectItem><SelectItem value="light">Light</SelectItem></SelectContent></Select></Field>
            <Button className="w-full" onClick={() => void onSave(draft)}><Save className="h-4 w-4" />Save settings</Button>
          </TabsContent>
          <TabsContent value="auth" className="space-y-4 pt-3">
            {authError && draft.provider.name !== "codex" && <Alert className="border-red-500/40"><AlertTitle>Authentication failed</AlertTitle><AlertDescription>{authError}</AlertDescription></Alert>}
            {draft.provider.name === "codex" ? <CodexAuthPanel account={codexAccount} limits={codexLimits} phase={codexPhase} error={authError} onLogin={() => void loginCodex()} onLogout={() => void logoutCodex()} onRefresh={() => void refreshCodex()} /> : <><Field label="Profile name"><Input value={profile} onChange={(event) => setProfile(event.target.value)} /></Field>
            <Tabs defaultValue="key">
              <TabsList className="grid w-full grid-cols-3"><TabsTrigger value="key">API key</TabsTrigger><TabsTrigger value="env">Environment</TabsTrigger><TabsTrigger value="oauth">OAuth</TabsTrigger></TabsList>
              <TabsContent value="key" className="space-y-3"><Input type="password" value={secret} onChange={(event) => setSecret(event.target.value)} placeholder="Stored in the native keyring" /><Button className="w-full" onClick={() => void login("key")} disabled={!secret}><KeyRound className="h-4 w-4" />Store API key</Button></TabsContent>
              <TabsContent value="env" className="space-y-3"><Input value={variable} onChange={(event) => setVariable(event.target.value)} placeholder="OPENAI_API_KEY" /><Button className="w-full" onClick={() => void login("env")}>Use environment variable</Button></TabsContent>
              <TabsContent value="oauth" className="space-y-3"><Input value={clientId} onChange={(event) => setClientId(event.target.value)} placeholder="Official OAuth client ID" />{draft.provider.name === "gemini" && <Input value={googleProject} onChange={(event) => setGoogleProject(event.target.value)} placeholder="Google Cloud project ID" />}<p className="text-xs text-muted-foreground">Kernex provides the official Google desktop PKCE flow for Gemini. Google requires the Cloud project for API quota. Other providers use API credentials unless an official custom flow is configured.</p><Button className="w-full" onClick={() => void login("oauth")} disabled={!clientId || (draft.provider.name === "gemini" && !googleProject) || !providerInfo.find((item) => item.kind === draft.provider.name)?.oauth_pkce}>Open provider sign-in</Button></TabsContent>
            </Tabs>
            <div className="space-y-2 border-t pt-4">{auth.map((status) => <div key={status.profile.name} className="flex items-center justify-between rounded border p-3"><div><div className="text-sm font-medium">{status.profile.name} {status.active && <span className="text-emerald-400">active</span>}</div><div className="text-xs text-muted-foreground">{status.profile.provider} · {status.profile.method} · {status.credential_available ? status.expired ? "expired" : "ready" : "unavailable"}</div></div><div className="flex"><Button size="sm" variant="ghost" onClick={() => void api.useAuth(status.profile.name).then(refreshAuth)}>Use</Button><Button size="icon" variant="ghost" onClick={() => void api.logout(status.profile.name).then(refreshAuth)}><LogOut className="h-4 w-4" /></Button></div></div>)}</div></>}
          </TabsContent>
          <TabsContent value="mcp" className="space-y-3 pt-3">
            <Alert><Plug className="mb-2 h-5 w-5 text-emerald-400" /><AlertTitle>Project extensions</AlertTitle><AlertDescription>Managed in <code>.kernex/config.toml</code> and executed through the shared permission system.</AlertDescription></Alert>
            <div><h3 className="mb-2 text-sm font-medium">MCP servers</h3>{overview?.mcpServers.length ? overview.mcpServers.map((name) => <div key={name} className="rounded border p-2 text-sm">{name}</div>) : <p className="text-xs text-muted-foreground">No MCP servers configured.</p>}</div>
            <div><h3 className="mb-2 text-sm font-medium">Language servers</h3>{overview?.languageServers.length ? overview.languageServers.map((name) => <div key={name} className="rounded border p-2 text-sm">{name}</div>) : <p className="text-xs text-muted-foreground">No language servers configured.</p>}</div>
            <Field label=".kernex/config.toml"><Textarea className="min-h-52 font-mono text-xs" value={extensionConfig} onChange={(event) => setExtensionConfig(event.target.value)} disabled={!overview} placeholder="Open a project to manage its MCP and language servers." /></Field>
            {extensionStatus && <p className="text-xs text-muted-foreground">{extensionStatus}</p>}
            <Button className="w-full" onClick={() => void saveExtensions()} disabled={!overview}><Save className="h-4 w-4" />Validate and save project extensions</Button>
          </TabsContent>
          <TabsContent value="logs" className="space-y-3 pt-3"><Alert><Stethoscope className="mb-2 h-5 w-5 text-emerald-400" /><AlertTitle>Diagnostics</AlertTitle><AlertDescription>Credentials and sensitive file contents are excluded from this event log.</AlertDescription></Alert><pre className="max-h-[58vh] overflow-auto whitespace-pre-wrap rounded border bg-black/25 p-3 text-[10px]">{diagnostics.join("\n") || "No events yet."}</pre></TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  );
}

export function CodexAuthPanel({ account, limits, phase, error, onLogin, onLogout, onRefresh }: {
  account?: CodexAccountStatus;
  limits?: CodexRateLimits;
  phase: CodexAuthPhase;
  error?: string;
  onLogin: () => void;
  onLogout: () => void;
  onRefresh: () => void;
}) {
  const chatgptAccount = account?.account?.type === "chatgpt" ? account.account : undefined;
  const otherAuthActive = Boolean(account?.account && !chatgptAccount);
  const busy = phase !== "idle";

  if (phase === "loading") {
    return <section aria-label="ChatGPT account" aria-busy="true" className="rounded-xl border bg-muted/20 p-6">
      <div role="status" className="flex min-h-36 flex-col items-center justify-center gap-3 text-center">
        <LoaderCircle className="h-6 w-6 animate-spin text-emerald-400" />
        <div><p className="text-sm font-medium">Checking your ChatGPT connection</p><p className="mt-1 text-xs text-muted-foreground">Reading the session managed by Codex…</p></div>
      </div>
    </section>;
  }

  return <section aria-label="ChatGPT account" aria-busy={busy} className="space-y-4">
    {error && <Alert className="border-red-500/40 bg-red-500/5"><AlertTitle>Couldn’t connect to ChatGPT</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {chatgptAccount ? <>
      <div className="overflow-hidden rounded-xl border bg-gradient-to-br from-emerald-500/10 via-background to-background">
        <div className="flex items-start justify-between gap-4 p-5">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full border bg-background text-sm font-semibold"><UserRound className="h-5 w-5" /></div>
            <div className="min-w-0"><p className="truncate text-sm font-semibold">{chatgptAccount.email ?? "ChatGPT account"}</p><p className="mt-0.5 text-xs text-muted-foreground">{formatPlanName(chatgptAccount.planType)}</p></div>
          </div>
          <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2.5 py-1 text-[11px] font-medium text-emerald-500"><CheckCircle2 className="h-3.5 w-3.5" />Connected</span>
        </div>
        <div className="border-t bg-background/45 px-5 py-3 text-xs text-muted-foreground">Codex manages and refreshes this session for Kernex.</div>
      </div>
      {limits?.rateLimits?.primary && <div className="rounded-xl border p-4">
        <div className="mb-3 flex items-center justify-between"><div><p className="text-sm font-medium">Codex usage</p><p className="text-xs text-muted-foreground">Included with your ChatGPT plan</p></div>{limits.rateLimitResetCredits && <span className="rounded-full bg-muted px-2 py-1 text-[11px] text-muted-foreground">{limits.rateLimitResetCredits.availableCount} resets available</span>}</div>
        <UsageMeter window={limits.rateLimits.primary} />
      </div>}
      <div className="flex gap-2">
        <Button className="flex-1" variant="outline" onClick={onLogout} disabled={busy}><LogOut className="h-4 w-4" />{phase === "signing-out" ? "Signing out…" : "Sign out"}</Button>
        <Button variant="outline" size="icon" aria-label="Refresh ChatGPT account" title="Refresh ChatGPT account" onClick={onRefresh} disabled={busy}>{phase === "refreshing" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}</Button>
      </div>
    </> : <div className="overflow-hidden rounded-xl border bg-gradient-to-br from-emerald-500/10 via-background to-background">
      <div className="p-5">
        <div className="mb-5 flex items-start justify-between gap-4">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-foreground text-background"><Sparkles className="h-5 w-5" /></div>
          <span className="rounded-full border bg-background/70 px-2.5 py-1 text-[11px] font-medium text-muted-foreground">{otherAuthActive ? "API key active" : "Not connected"}</span>
        </div>
        <h3 className="text-lg font-semibold">Log in to ChatGPT</h3>
        <p className="mt-1.5 text-sm leading-6 text-muted-foreground">{otherAuthActive ? "Codex is currently using API key authentication. Log in to ChatGPT to switch to your subscription-backed account." : "Use the models and usage limits included with your ChatGPT plan—no API key setup required."}</p>
        <div className="my-5 space-y-3 rounded-lg border bg-background/55 p-3 text-xs text-muted-foreground">
          <div className="flex gap-2.5"><ExternalLink className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500" /><span>Sign-in opens securely in your default browser.</span></div>
          <div className="flex gap-2.5"><ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500" /><span>Codex stores and refreshes the session; Kernex never asks for your password.</span></div>
        </div>
        {phase === "signing-in" && <div role="status" aria-live="polite" className="mb-3 flex items-start gap-2.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 p-3 text-xs"><LoaderCircle className="mt-0.5 h-4 w-4 shrink-0 animate-spin text-emerald-500" /><div><p className="font-medium text-foreground">Complete sign-in in your browser</p><p className="mt-0.5 text-muted-foreground">This panel will update automatically when ChatGPT finishes.</p></div></div>}
        <Button className="h-10 w-full" onClick={onLogin} disabled={busy}>{phase === "signing-in" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <ExternalLink className="h-4 w-4" />}{phase === "signing-in" ? "Waiting for ChatGPT…" : "Continue with ChatGPT"}</Button>
      </div>
    </div>}
  </section>;
}

function UsageMeter({ window }: { window: NonNullable<NonNullable<CodexRateLimits["rateLimits"]>["primary"]> }) {
  const usedPercent = Math.min(100, Math.max(0, window.usedPercent));
  const windowLabel = window.windowDurationMins ? `${window.windowDurationMins}-minute window` : "current window";
  const resetLabel = formatResetTime(window.resetsAt);
  return <div className="space-y-2">
    <div className="flex justify-between text-xs"><span>{Math.round(usedPercent)}% used</span><span className="text-muted-foreground">{windowLabel}</span></div>
    <div role="progressbar" aria-label="Codex usage" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(usedPercent)} className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-emerald-500 transition-[width]" style={{ width: `${usedPercent}%` }} /></div>
    {resetLabel && <p className="text-[11px] text-muted-foreground">Resets {resetLabel}</p>}
  </div>;
}

function formatPlanName(planType?: string) {
  if (!planType) return "ChatGPT plan";
  const plan = planType.replace(/[-_]/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
  return `ChatGPT ${plan}`;
}

function formatResetTime(resetsAt?: number) {
  if (!resetsAt) return undefined;
  const reset = new Date(resetsAt * 1000);
  if (Number.isNaN(reset.getTime())) return undefined;
  return reset.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label className="block space-y-1.5"><span className="text-xs font-medium text-muted-foreground">{label}</span>{children}</label>;
}
