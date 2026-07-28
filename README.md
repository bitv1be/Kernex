# Kernex

Kernex is a cross-platform AI coding agent with an interactive terminal client and a native Tauri desktop application. Both clients use the same Rust core for agent execution, provider protocols, authentication, sessions, project access, tools, Git, extensions, and permissions.

The project evolved from its original Rust CLI and eframe desktop foundation. Existing workspace confinement, provider adapters, tool-calling loop, permission gates, Git/diff review, tree-sitter analysis, LSP, MCP, and process-plugin support remain in the shared core. The desktop host is now Tauri 2 with React, TypeScript, Vite, Tailwind CSS, and shadcn/ui-style components.

## Prerequisites

- stable Rust with Cargo;
- Node.js 20 or newer and npm;
- the [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/) for desktop development;
- a native credential service (Windows Credential Manager, macOS Keychain, or Linux Secret Service) for stored credentials.

On Debian or Ubuntu, install the Linux desktop libraries with:

```bash
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

Install the frontend dependencies once:

```bash
npm ci
```

## CLI

Run Kernex from the project it should work on:

```bash
cargo run -p kernex-cli --
```

With no subcommand, an interactive terminal starts in the current directory. It can select a provider and model, stream the response and tool activity, request scoped approvals, cancel the current turn with Ctrl+C, and continue until `/exit`. Useful commands include:

```bash
cargo run -p kernex-cli -- "Fix the failing tests"
cargo run -p kernex-cli -- run --provider local --model qwen3-coder \
  --base-url http://127.0.0.1:11434/v1 "Explain this workspace"
cargo run -p kernex-cli -- review
cargo run -p kernex-cli -- sessions
cargo run -p kernex-cli -- resume
cargo run -p kernex-cli -- init
cargo run -p kernex-cli -- models
```

`kernex init` creates `KERNEX.md` and will not silently replace an existing file. Repository instructions are discovered from `KERNEX.md`, `AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `.kernex/instructions.md`, and nested guides.

## Desktop

Start the Vite frontend and native Tauri host together:

```bash
npm run desktop:dev
```

Create a production application bundle with:

```bash
npm run desktop:build
```

The desktop interface includes recent-project selection, shared session history, streaming chat, file and code views, Git status/diffs/log, a permissioned terminal, tool activity, approval dialogs, provider and model settings, authentication profiles, project MCP/LSP visibility, diagnostics, and light/dark themes.

## Authentication and providers

Supported adapters are `openai-compatible`, `anthropic`, `gemini`, `local`, and `custom`. API keys can come from a native keyring profile or a named environment variable. OAuth Authorization Code with PKCE is implemented for Google Gemini (using a desktop client ID and Cloud project ID) and for user-configured custom providers only when their official documentation supports a public-client flow; OpenAI and Anthropic continue to use their supported API credentials.

```bash
cargo run -p kernex-cli -- auth login
cargo run -p kernex-cli -- auth status
cargo run -p kernex-cli -- auth use personal
cargo run -p kernex-cli -- auth logout personal
```

See [Authentication](docs/AUTHENTICATION.md) for profile storage, OAuth setup, custom endpoints, refresh behavior, and platform notes.

## Safety and project extensions

Permission modes are `read-only`, `ask`, `auto-safe`, and `full-access`. Approvals can be granted once, for the session, or for the canonical project. Normal project reads and Git inspection are low risk; file mutations show a diff, and commands are argument vectors with bounded time/output and a filtered environment. Workspace path and symlink checks prevent filesystem-tool escapes. `full-access` is an explicit trust decision, not an OS sandbox.

Project settings live in `.kernex/config.toml`; credentials do not. MCP server environment entries map a child variable to the name of a host environment variable:

```toml
[provider]
name = "local"
model = "qwen3-coder"
base_url = "http://127.0.0.1:11434/v1"

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

MCP uses the `2025-11-25` stdio protocol. Process plugins are discovered from `.kernex/plugins/*/plugin.toml`; both execute through the shared command and permission boundaries.

Sessions from both clients are structured records in the same local SQLite database and contain messages, tool calls/results, approvals, token usage, diffs, timestamps, and status. Repository contents and credentials are not copied into the session database.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
npm run frontend:typecheck
npm run frontend:lint
npm run frontend:test
npm run frontend:build
```

See [Architecture](docs/ARCHITECTURE.md) and [Security](docs/SECURITY.md) for implementation and trust-boundary details.

## License

Kernex is available under the [MIT License](LICENSE).
