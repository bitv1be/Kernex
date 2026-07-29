# Architecture

Kernex is a Rust 2024 workspace with two presentation layers and one shared business-logic crate:

- `kernex-cli` at the repository root owns terminal parsing, rendering, prompts, and Ctrl+C handling.
- `kernex-desktop` is a Tauri 2 native host. Its Rust backend lives in `crates/kernex-desktop/src-tauri`, while the React/TypeScript frontend lives in `crates/kernex-desktop/src` and communicates with Rust through typed commands and events.
- `kernex-agent-core` owns agent execution, provider protocols, authentication, SQLite sessions, global/project settings, project context, tools, permissions, filesystem access, commands, Git, MCP, LSP, and plugins.

Neither host implements a second agent loop. Both assemble a run through `runtime::run_agent_turn`, provide an `Approver` and `EventSink`, and use the same session/authentication stores.

## Agent flow

1. The host canonicalizes the selected workspace and loads global plus `.kernex/config.toml` settings.
2. For HTTP providers, the core discovers scoped repository instructions and configured extensions. The authentication layer resolves only the selected credential profile; the provider receives a short-lived secret wrapper rather than persisted secret text.
3. HTTP-provider requests are translated to OpenAI-compatible, Anthropic, or Gemini wire formats and consumed as a stream when supported.
4. For `codex`, the shared runtime instead launches `codex app-server`, performs the required initialize handshake, starts or resumes the provider-owned thread stored in the Kernex session, and streams one complete App Server turn.
5. Model tool calls are normalized into `ToolCall` values and dispatched only through the registered `Toolbox` definitions.
6. The permission gate evaluates mode, risk, resource-scoped session grants, and canonical-project grants before protected work.
7. Tool results return to the model until a final response, cancellation, or the configured step limit.
8. `SessionRecorder` persists structured transitions to SQLite while forwarding the same events to the active UI.

`EventSink` carries model progress, streamed text, tool starts/results/errors, diffs, usage, and completion. Tauri converts those events into window events; the CLI renders them directly.

## Core modules

- `agent.rs`: provider-independent loop and built-in registered tools.
- `runtime.rs`: shared host assembly for a complete agent turn.
- `provider.rs`: common model interface, wire-format adapters, SSE normalization, usage, and capabilities.
- `codex_app_server.rs`: JSONL/JSON-RPC transport, managed ChatGPT account and plan endpoints, model discovery, persistent threads, turn events, and approval translation.
- `auth.rs`: named profiles, PKCE OAuth, refresh/logout, secret wrappers, and native keyring access.
- `session.rs`: shared SQLite records and permission/event auditing.
- `permission.rs`: modes, scoped grants, project grants, and risk decisions.
- `workspace.rs`, `diff.rs`, `command.rs`, and `git.rs`: confined local operations and reviewable changes.
- `instructions.rs`, `index.rs`, and `syntax.rs`: project context and structural inspection.
- `mcp.rs`, `lsp.rs`, and `plugin.rs`: explicitly configured extension processes.

## Desktop boundary

The webview has no direct filesystem or process access. Tauri commands expose only bounded operations such as workspace overview, project-file reads, Git inspection, permissioned terminal execution, shared sessions/settings, authentication, and agent start/cancel. Risky operations still use the core permission system. The frontend uses Zustand for transient UI state and TanStack Query for backend state; CodeMirror renders source, xterm.js renders terminal output, and React Markdown renders streamed answers.

## Sessions and configuration

`directories::ProjectDirs` selects platform-native user locations. `sessions.sqlite3` is the compatibility boundary between CLI and desktop. Ordinary configuration stores provider/model/base URL, an authentication-profile name, permission mode, theme, and recent projects. Project configuration stores provider overrides and extension commands. Secret values are never serialized into these files or into session records. A Codex-backed session stores only the opaque App Server thread ID needed by `thread/resume`; Codex owns its rollout and OAuth storage.

## Project intelligence and extensions

`ProjectIndex` respects ignore rules and provides bounded search. The toolbox invalidates its cached index after edits or commands. `GitRepository` reports status, bounded log, and diffs including untracked text. Tree-sitter supports Rust, Python, JavaScript, TypeScript/TSX, and Go outlines; configured LSP servers add semantic symbols.

MCP clients launch permissioned stdio processes, negotiate protocol `2025-11-25`, paginate tool discovery defensively, and expose namespaced tools. Declarative process plugins use names such as `plugin__quality__audit`; both share the command runner and permission policy.
