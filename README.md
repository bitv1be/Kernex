# Kernex

Kernex is a cross-platform AI coding agent with an interactive terminal client and a native Tauri desktop application. Both clients use the same Rust core for agent execution, provider protocols, authentication, sessions, project access, tools, Git, extensions, and permissions.

The project evolved from its original Rust CLI and eframe desktop foundation. Existing workspace confinement, provider adapters, tool-calling loop, permission gates, Git/diff review, tree-sitter analysis, LSP, MCP, and process-plugin support remain in the shared core. The desktop host is now Tauri 2 with React, TypeScript, Vite, Tailwind CSS, and shadcn/ui-style components.

## Prerequisites

- stable Rust with Cargo;
- Bun 1.3 or newer;
- the Codex CLI on `PATH` when using ChatGPT subscription access;
- the [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/) for desktop development;
- a native credential service (Windows Credential Manager, macOS Keychain, or Linux Secret Service) for stored credentials.

On Debian or Ubuntu, install the Linux desktop libraries with:

```bash
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

Install the frontend dependencies once:

```bash
bun install --cwd crates/kernex-desktop --frozen-lockfile
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
bun run desktop:dev
```

Create a production application bundle with:

    bun run desktop:build

The desktop interface includes recent-project selection, shared session history, streaming chat, file and code views, Git status/diffs/log, a permissioned terminal, tool activity, approval dialogs, provider and model settings, authentication profiles, project MCP/LSP visibility, diagnostics, and light/dark themes.

## Releases

Merges to `main` automatically build, install, smoke test, and publish native desktop packages for each supported platform; the current `master` default branch remains supported during migration. The same workflow also accepts pushed semantic tags such as `v0.1.0` and manual runs that name an existing semantic tag. A release is published only after all required packages are present and their downloaded copies pass the release-asset verifier:

- Kernex-VERSION-linux-x86_64.AppImage;
- Kernex-VERSION-windows-x86_64-setup.exe and Kernex-VERSION-windows-x86_64.msi;
- Kernex-VERSION-macos-universal.dmg and a complete Kernex-VERSION-macos-universal.app.tar.gz application bundle;
- SHA256SUMS for every downloadable package.

The Windows installers include the offline WebView2 installer. Published packages contain the production frontend and do not require Bun, Node.js, Rust, a development server, the source tree, or project dependency directories at runtime.

## Authentication and providers

Supported adapters are `codex`, `openai-compatible`, `anthropic`, `gemini`, `local`, and `custom`. The `codex` adapter delegates complete turns to the installed [Codex App Server](https://learn.chatgpt.com/docs/app-server), uses its managed ChatGPT OAuth session, discovers the models available to the current subscription plan, reports plan rate limits, and persists the App Server thread ID with the shared Kernex session. Other adapters use API keys from a native keyring profile or named environment variable; Google Gemini and documented custom providers can use Kernex's PKCE client.

```bash
cargo run -p kernex-cli -- auth login --provider codex --method o-auth
cargo run -p kernex-cli -- models --discover
cargo run -p kernex-cli -- run --provider codex --model MODEL "Fix the failing tests"
cargo run -p kernex-cli -- auth login
cargo run -p kernex-cli -- auth status
cargo run -p kernex-cli -- auth use personal
cargo run -p kernex-cli -- auth logout personal
```

The Codex executable defaults to `codex`; set `KERNEX_CODEX_PATH` to an explicit executable path when it is installed elsewhere. Codex owns token persistence and refresh. Kernex exchanges newline-delimited JSON-RPC over stdio, never reads or serializes the ChatGPT tokens, and forwards Codex command, file-change, and additional-sandbox approvals through the normal Kernex approval surface.

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
bun run frontend:typecheck
bun run frontend:lint
bun run frontend:test
bun run frontend:build
```

See [Architecture](docs/ARCHITECTURE.md) and [Security](docs/SECURITY.md) for implementation and trust-boundary details.

## License

Kernex is available under the [MIT License](LICENSE).
