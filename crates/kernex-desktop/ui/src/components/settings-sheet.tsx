import { KeyRound, LogOut, Plug, RefreshCw, Save, Settings2, Stethoscope } from "lucide-react";
import { useEffect, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api } from "@/lib/api";
import type { AuthStatus, ProviderKind, ProviderModel, ProviderSummary, Settings, WorkspaceOverview } from "@/lib/types";

const providers: ProviderKind[] = ["openai-compatible", "anthropic", "gemini", "local", "custom"];

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
  const [models, setModels] = useState<ProviderModel[]>([]);
  const [discovering, setDiscovering] = useState(false);
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
  const updateProvider = (patch: Partial<Settings["provider"]>) => setDraft({ ...draft, provider: { ...draft.provider, ...patch } });
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
        <Tabs defaultValue="general">
          <TabsList className="grid w-full grid-cols-4"><TabsTrigger value="general">General</TabsTrigger><TabsTrigger value="auth">Auth</TabsTrigger><TabsTrigger value="mcp">MCP</TabsTrigger><TabsTrigger value="logs">Logs</TabsTrigger></TabsList>
          <TabsContent value="general" className="space-y-4 pt-3">
            <Field label="Provider"><Select value={draft.provider.name} onValueChange={(name: ProviderKind) => { const info = providerInfo.find((item) => item.kind === name); updateProvider({ name, base_url: info?.base_url }); }}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{providers.map((provider) => <SelectItem key={provider} value={provider}>{provider}</SelectItem>)}</SelectContent></Select></Field>
            <Field label="Model"><div className="flex gap-2"><Input list="kernex-model-catalog" value={draft.provider.model} onChange={(event) => updateProvider({ model: event.target.value })} placeholder="Model ID" /><Button type="button" variant="outline" size="icon" title="Discover models" onClick={() => void discover()} disabled={discovering}><RefreshCw className={`h-4 w-4 ${discovering ? "animate-spin" : ""}`} /></Button><datalist id="kernex-model-catalog">{models.map((model) => <option key={model.id} value={model.id}>{model.display_name}</option>)}</datalist></div>{models.length > 0 && <span className="text-[11px] text-muted-foreground">{models.length} models discovered from the configured provider.</span>}</Field>
            <Field label="API base URL"><Input value={draft.provider.base_url ?? ""} onChange={(event) => updateProvider({ base_url: event.target.value })} placeholder="Provider default" /></Field>
            <Field label="Authentication profile"><Select value={draft.provider.auth_profile ?? "none"} onValueChange={(value) => updateProvider({ auth_profile: value === "none" ? undefined : value })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="none">Environment/default</SelectItem>{auth.filter((status) => status.profile.provider === draft.provider.name).map((status) => <SelectItem key={status.profile.name} value={status.profile.name}>{status.profile.name}</SelectItem>)}</SelectContent></Select></Field>
            <Field label="Permission mode"><Select value={draft.permission_mode} onValueChange={(permission_mode: Settings["permission_mode"]) => setDraft({ ...draft, permission_mode })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="read-only">Read only</SelectItem><SelectItem value="ask">Ask on protected actions</SelectItem><SelectItem value="auto-safe">Automatically allow safe actions</SelectItem><SelectItem value="full-access">Full access</SelectItem></SelectContent></Select></Field>
            <Field label="Theme"><Select value={draft.theme} onValueChange={(theme) => setDraft({ ...draft, theme })}><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="system">System</SelectItem><SelectItem value="dark">Dark</SelectItem><SelectItem value="light">Light</SelectItem></SelectContent></Select></Field>
            <Button className="w-full" onClick={() => void onSave(draft)}><Save className="h-4 w-4" />Save settings</Button>
          </TabsContent>
          <TabsContent value="auth" className="space-y-4 pt-3">
            {authError && <Alert className="border-red-500/40"><AlertTitle>Authentication failed</AlertTitle><AlertDescription>{authError}</AlertDescription></Alert>}
            <Field label="Profile name"><Input value={profile} onChange={(event) => setProfile(event.target.value)} /></Field>
            <Tabs defaultValue="key">
              <TabsList className="grid w-full grid-cols-3"><TabsTrigger value="key">API key</TabsTrigger><TabsTrigger value="env">Environment</TabsTrigger><TabsTrigger value="oauth">OAuth</TabsTrigger></TabsList>
              <TabsContent value="key" className="space-y-3"><Input type="password" value={secret} onChange={(event) => setSecret(event.target.value)} placeholder="Stored in the native keyring" /><Button className="w-full" onClick={() => void login("key")} disabled={!secret}><KeyRound className="h-4 w-4" />Store API key</Button></TabsContent>
              <TabsContent value="env" className="space-y-3"><Input value={variable} onChange={(event) => setVariable(event.target.value)} placeholder="OPENAI_API_KEY" /><Button className="w-full" onClick={() => void login("env")}>Use environment variable</Button></TabsContent>
              <TabsContent value="oauth" className="space-y-3"><Input value={clientId} onChange={(event) => setClientId(event.target.value)} placeholder="Official OAuth client ID" />{draft.provider.name === "gemini" && <Input value={googleProject} onChange={(event) => setGoogleProject(event.target.value)} placeholder="Google Cloud project ID" />}<p className="text-xs text-muted-foreground">Kernex provides the official Google desktop PKCE flow for Gemini. Google requires the Cloud project for API quota. Other providers use API credentials unless an official custom flow is configured.</p><Button className="w-full" onClick={() => void login("oauth")} disabled={!clientId || (draft.provider.name === "gemini" && !googleProject) || !providerInfo.find((item) => item.kind === draft.provider.name)?.oauth_pkce}>Open provider sign-in</Button></TabsContent>
            </Tabs>
            <div className="space-y-2 border-t pt-4">{auth.map((status) => <div key={status.profile.name} className="flex items-center justify-between rounded border p-3"><div><div className="text-sm font-medium">{status.profile.name} {status.active && <span className="text-emerald-400">active</span>}</div><div className="text-xs text-muted-foreground">{status.profile.provider} · {status.profile.method} · {status.credential_available ? status.expired ? "expired" : "ready" : "unavailable"}</div></div><div className="flex"><Button size="sm" variant="ghost" onClick={() => void api.useAuth(status.profile.name).then(refreshAuth)}>Use</Button><Button size="icon" variant="ghost" onClick={() => void api.logout(status.profile.name).then(refreshAuth)}><LogOut className="h-4 w-4" /></Button></div></div>)}</div>
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

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label className="block space-y-1.5"><span className="text-xs font-medium text-muted-foreground">{label}</span>{children}</label>;
}
