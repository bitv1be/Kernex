# Architecture

Kernex is a Cargo workspace with three presentation/runtime layers:

- `kernex-cli` (repository root) parses commands, renders progress, and asks for terminal approvals.
- `kernex-desktop` provides a native eframe UI with task configuration, a live action timeline, approval controls, diff review, results, and cancellation.
- `kernex-agent-core` owns all provider, agent, workspace, tool, permission, and extension behavior. Neither host reimplements agent logic.

## Agent flow

1. The host opens and canonicalizes one project root.
2. The core discovers repository instructions and configured extensions.
3. A provider-neutral request is translated to OpenAI-compatible, Anthropic, or Gemini wire formats.
4. Model tool calls are normalized into `ToolCall` values.
5. `Toolbox` validates paths, requests permission where required, executes the action, and emits a reviewable event.
6. Results return to the model until it produces a final answer, the user cancels, or the step limit is reached.

All meaningful transitions are emitted through `EventSink`, allowing the CLI and desktop application to show the same action history.

## Project intelligence

`ProjectIndex` walks the project with `.gitignore` support and provides bounded literal search. The toolbox reuses that snapshot during a run and invalidates it after edits or commands, avoiding repeated full-tree walks without returning stale agent-authored changes. `GitRepository` reports status and diffs, including untracked text files. `SyntaxAnalyzer` uses tree-sitter for Rust, Python, JavaScript, TypeScript/TSX, and Go structural outlines. Configured LSP servers add semantic document symbols through standard JSON-RPC framing.

## Extension boundaries

MCP clients launch permissioned stdio server processes, negotiate protocol version `2025-11-25`, discover tools, and expose namespaced tool definitions to the model. Declarative process plugins use names such as `plugin__quality__audit`; they execute through the same command runner and permission policy as built-in command tools.

Secrets are represented by environment-variable names in configuration. Values are resolved only immediately before an approved provider or extension request and are not serialized into configuration, events, or model messages.
