# Kernex

Kernex is a native, cross-platform AI coding agent built around one shared Rust core. It combines a transparent terminal workflow with a desktop interface while keeping model choice, repository access, command execution, and file changes under user control.

The repository currently provides a working foundation: a CLI, an eframe desktop application, provider adapters, an iterative tool-calling agent, permission gates, project indexing, Git/diff review, tree-sitter analysis, LSP connectivity, MCP stdio servers, and process plugins.

The CI workflow is configured to run the complete workspace tests, optimized build, and CLI smoke test on native Linux, Windows, and macOS runners. This avoids relying on incomplete cross-linker toolchains when validating platform-specific GUI and TLS dependencies.

Every pull request merged into `master` starts the release workflow. It builds version-matched CLI and desktop archives on native Linux, macOS, and Windows runners, generates SHA-256 checksums, increments the semantic patch version, and publishes a GitHub Release with generated notes. An explicit higher version in `[workspace.package]` takes precedence over the automatic increment.

## Quick start

```bash
cargo build --workspace
cargo run -- inspect
cargo run -- search PermissionGate
cargo run -- outline crates/agent-core/src/agent.rs
cargo run -p kernex-desktop
```

Run an agent against an OpenAI-compatible endpoint:

```bash
export OPENAI_API_KEY="..."
cargo run -- run --model MODEL_ID "Inspect the project and run its tests"
```

Use a local OpenAI-compatible server without an API key:

```bash
cargo run -- run \
  --provider local \
  --model qwen3-coder \
  --base-url http://127.0.0.1:11434/v1 \
  "Explain the workspace architecture"
```

Supported adapters are `openai-compatible`, `anthropic`, `gemini`, `local`, and `custom`. API-key values are read from environment variables only. Use `kernex providers` to inspect defaults.

Custom authentication headers also refer to environment variables rather than literal values: `--header-env X-API-Key=SERVICE_API_KEY`. Repeat the option for multiple headers.

## Safety model

Read-only project inspection is allowed by default. File writes, commands, network calls, credentials, MCP servers, and language servers require explicit approval. Every file mutation presents a complete diff first. Commands use an argument vector rather than shell expansion, run with time and output limits, and receive a filtered environment that excludes likely credentials. Canonical path checks reject traversal and symlink escapes.

`--yes` is an explicit non-interactive session grant. It is intended for trusted automation; omit it for normal interactive review. See [SECURITY.md](docs/SECURITY.md) for the threat model and current limitations.

## Project extensions

Optional project configuration lives at `.kernex/config.toml`:

```toml
[[mcp_servers]]
name = "project-tools"
command = "project-mcp-server"
args = ["--stdio"]

[mcp_servers.env]
SERVICE_TOKEN = "PROJECT_SERVICE_TOKEN"

[[language_servers]]
language_id = "rust"
command = "rust-analyzer"
extensions = ["rs"]
```

MCP servers use the `2025-11-25` stdio protocol. Each server and explicitly mapped credential is permission-gated. Language servers use standard `Content-Length` JSON-RPC framing.

Process plugins are discovered from `.kernex/plugins/*/plugin.toml`. A plugin tool receives one JSON object on stdin and returns text on stdout:

```toml
[plugin]
id = "quality"
name = "Quality Tools"
version = "0.1.0"

[[tools]]
name = "audit"
description = "Run the project-specific audit"
command = "./audit"
args = []
input_schema = '{"type":"object","properties":{}}'
```

Repository instructions are loaded from `AGENTS.md`, `KERNEX.md`, `CONTRIBUTING.md`, `.kernex/instructions.md`, and nested agent guides. See [ARCHITECTURE.md](docs/ARCHITECTURE.md) for implementation details.

## License

Kernex is available under the [MIT License](LICENSE).
