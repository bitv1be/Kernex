import { invoke } from "@tauri-apps/api/core";
import type { AuthStatus, CommandOutput, ProviderModel, ProviderSummary, SessionRecord, Settings, StartAgentRequest, WorkspaceOverview } from "./types";

export const api = {
  overview: (path: string) => invoke<WorkspaceOverview>("workspace_overview", { path }),
  readFile: (workspace: string, path: string) => invoke<string>("read_project_file", { workspace, path }),
  gitStatus: (workspace: string) => invoke<string>("git_status", { workspace }),
  gitDiff: (workspace: string, staged = false) => invoke<string>("git_diff", { workspace, staged }),
  gitLog: (workspace: string, limit = 30) => invoke<string>("git_log", { workspace, limit }),
  startAgent: (request: StartAgentRequest) => invoke<string>("start_agent", { request }),
  cancelAgent: () => invoke<void>("cancel_agent"),
  respondPermission: (id: number, decision: string) => invoke<void>("respond_permission", { id, decision }),
  sessions: (workspace?: string, limit = 50) => invoke<SessionRecord[]>("list_sessions", { workspace, limit }),
  session: (id: string) => invoke<SessionRecord>("load_session", { id }),
  deleteSession: (id: string) => invoke<boolean>("delete_session", { id }),
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  providers: () => invoke<ProviderSummary[]>("providers"),
  discoverModels: (provider: string, model: string, baseUrl?: string, authProfile?: string) => invoke<ProviderModel[]>("discover_models", { provider, model, baseUrl, authProfile }),
  projectConfig: (workspace: string) => invoke<string>("project_config", { workspace }),
  saveProjectConfig: (workspace: string, contents: string) => invoke<string>("save_project_config", { workspace, contents }),
  authStatus: () => invoke<AuthStatus[]>("auth_status"),
  loginApiKey: (profile: string, provider: string, apiKey: string) => invoke<void>("auth_login_api_key", { profile, provider, apiKey }),
  loginEnvironment: (profile: string, provider: string, variable: string) => invoke<void>("auth_login_environment", { profile, provider, variable }),
  loginOAuth: (profile: string, provider: string, clientId: string, resourceProject?: string) => invoke<void>("auth_login_oauth", { profile, provider, clientId, authorizationUrl: null, tokenUrl: null, scopes: [], resourceProject }),
  logout: (profile: string) => invoke<void>("auth_logout", { profile }),
  useAuth: (profile: string) => invoke<void>("auth_use", { profile }),
  runTerminal: (workspace: string, command: string[]) => invoke<CommandOutput>("run_terminal", { workspace, command }),
};
