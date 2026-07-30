export type ProviderKind = "codex" | "openai-compatible" | "anthropic" | "gemini" | "local" | "custom";
export type PermissionMode = "read-only" | "ask" | "auto-safe" | "full-access";
export type SessionStatus = "active" | "completed" | "cancelled" | "failed";

export interface Message {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  name?: string;
  tool_call_id?: string;
  tool_calls: ToolCall[];
}

export interface ToolCall { id: string; name: string; arguments: unknown }
export interface ToolResult { call_id: string; name: string; result: string; error?: string; timestamp: string }

export interface SessionRecord {
  id: string;
  workspace_path: string;
  provider: string;
  model: string;
  provider_thread_id?: string;
  messages: Message[];
  tool_calls: ToolCall[];
  tool_results: ToolResult[];
  permission_decisions: PermissionAudit[];
  created_at: string;
  updated_at: string;
  token_usage: { input_tokens?: number; output_tokens?: number };
  generated_diffs: string[];
  status: SessionStatus;
}

export interface PermissionAudit {
  capability: string;
  risk: string;
  resource: string;
  summary: string;
  decision: string;
  timestamp: string;
}

export interface FileRecord { path: string; size: number; language?: string }
export interface WorkspaceOverview {
  path: string;
  isGitRepository: boolean;
  files: FileRecord[];
  instructions: string[];
  gitStatus: string;
  mcpServers: string[];
  languageServers: string[];
}

export interface ProviderSummary {
  kind: ProviderKind;
  base_url: string;
  api_key_environment?: string;
  oauth_pkce: boolean;
  managed_oauth: boolean;
}

export interface ProviderModel {
  id: string;
  display_name?: string;
  description?: string;
  is_default: boolean;
  owned_by?: string;
  input_token_limit?: number;
  output_token_limit?: number;
}

export interface ProviderSettings { name: ProviderKind; model: string; base_url?: string; auth_profile?: string }
export interface Settings {
  provider: ProviderSettings;
  permission_mode: PermissionMode;
  recent_projects: string[];
  theme: "system" | "light" | "dark" | string;
}

export interface AuthProfile {
  name: string;
  provider: ProviderKind;
  method: "api_key" | "environment" | "oauth_pkce";
  environment_variable?: string;
  account_label?: string;
  expires_at?: number;
  oauth_resource_project?: string;
}
export interface AuthStatus { profile: AuthProfile; active: boolean; credential_available: boolean; expired: boolean }

export interface CodexAccountStatus {
  account?: { type: string; email?: string; planType?: string } | null;
  requiresOpenaiAuth: boolean;
}

export interface CodexRateLimitWindow { usedPercent: number; windowDurationMins?: number; resetsAt?: number }
export interface CodexRateLimitSnapshot {
  limitId?: string;
  limitName?: string;
  planType?: string;
  primary?: CodexRateLimitWindow;
  secondary?: CodexRateLimitWindow;
  credits?: { hasCredits: boolean; unlimited: boolean; balance?: string };
}
export interface CodexRateLimits {
  rateLimits?: CodexRateLimitSnapshot;
  rateLimitsByLimitId?: Record<string, CodexRateLimitSnapshot>;
  rateLimitResetCredits?: { availableCount: number };
}

export interface StartAgentRequest {
  workspace: string;
  task: string;
  provider: ProviderKind;
  model: string;
  baseUrl?: string;
  authProfile?: string;
  permissionMode: PermissionMode;
  maxSteps: number;
  sessionId?: string;
}

export type ProviderStreamEvent =
  | { type: "text_delta"; text: string }
  | { type: "tool_call_delta"; index: number; id?: string; name?: string; arguments_delta: string }
  | { type: "usage"; usage: { input_tokens?: number; output_tokens?: number } };

export type AgentEvent =
  | { type: "started"; task: string; provider: string; model: string }
  | { type: "model_requested"; step: number }
  | { type: "model_delta"; step: number; event: ProviderStreamEvent }
  | { type: "model_responded"; step: number; content: string; tool_calls: ToolCall[] }
  | { type: "tool_started"; step: number; call: ToolCall }
  | { type: "tool_finished"; step: number; call_id: string; name: string; result: string; diff?: string }
  | { type: "tool_failed"; step: number; call_id: string; name: string; error: string }
  | { type: "completed"; steps: number };

export interface PendingApproval {
  id: number;
  request: { capability: string; risk: string; summary: string; resource: string; details: string[] };
}

export interface CommandOutput {
  command: string;
  exit_code?: number;
  success: boolean;
  stdout: string;
  stderr: string;
  truncated: boolean;
}
