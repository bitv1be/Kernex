# Security Model

Kernex assumes model output, repository content, plugin manifests, MCP servers, language servers, and remote providers may be untrusted. The user and operating system remain the security principals.

## Enforced controls

- Filesystem tools canonicalize the project root and reject absolute escapes, parent traversal, and symlinks resolving outside the project.
- File mutations require approval, include the proposed diff, and use same-directory atomic replacement while preserving existing permissions.
- Commands are launched as a program plus argument vector, never by implicit shell evaluation. Runtime and captured output are bounded; child processes are killed when their future is dropped.
- Destructive programs, shells, mutating Git operations, commands, writes, network calls, credentials, MCP servers, and language servers receive explicit capability/risk labels.
- Permission modes support read-only, per-operation review, safe-operation automation, and explicit full access. Grants are scoped to a capability/resource for one operation, one session, or one canonical project.
- Tool and extension child environments are filtered for names commonly associated with keys, tokens, passwords, authentication, sessions, cookies, and proxies. The explicitly selected Codex App Server executable inherits the user's environment so the official CLI can resolve its own configuration and authentication.
- Common credential paths such as `.env.*`, private keys, and cloud credential directories require secret access, are skipped by search, and are redacted from Git diffs.
- Provider response bodies, extension messages, pagination, command output, and runtime are bounded.
- Closing the desktop application, pressing Cancel, or Ctrl+C cancels provider/tool futures and rejects pending desktop approvals.
- The Tauri webview can invoke only declared commands under the application capability file; it has no generic filesystem or shell API.

## Credential handling

Stored API keys and OAuth tokens use the operating system keyring through the `keyring` crate: Windows Credential Manager, macOS Keychain, or Linux Secret Service. Normal configuration contains only named profile metadata and environment-variable references. `SecretValue` redacts `Debug`, has no serialization implementation, uses constant-time equality, and zeroizes its allocation on drop.

Kernex-owned OAuth uses Authorization Code with PKCE, a cryptographically random verifier and state value, a loopback callback listener, exact state validation, browser launch, token exchange, expiry tracking, refresh-token rotation, and deletion on logout. Kernex enables that built-in flow only for Google Gemini; a custom flow requires endpoints supplied from that provider's official public-client documentation. ChatGPT subscription access is different: the trusted Codex App Server owns its official browser OAuth flow, token persistence, and refresh, while Kernex receives only account metadata, plan metadata, and an opaque thread ID over stdio JSON-RPC.

Credentials are not placed in project configuration, source, session SQLite records, diagnostics, or model messages. Environment-based profiles retain only the variable name and resolve its value at request time.

## Important limitations

Approval is not an operating-system sandbox. An approved executable runs with the user's OS account and can access anything that account can access; working-directory confinement does not constrain the executable itself. Review the exact command, arguments, risk label, diff, and extension source before approval. `full-access` should be reserved for already trusted workspaces.

Native keyring security depends on the platform service and logged-in desktop session. Linux headless machines may not provide Secret Service; use an environment-variable profile there and protect the process environment. Environment-name filtering is defense in depth and cannot prove that an oddly named value is harmless.

A model provider receives the conversation and relevant tool results after network approval. Apply that provider's data-handling and billing policies. OAuth refresh can detect HTTP/token failures, but a provider may expose revocation only when the next refresh or API request occurs.

MCP servers and plugins are executable software. Keep project configuration under review, pin external versions where possible, and grant persistent permissions only to trusted components.

The executable selected by `KERNEX_CODEX_PATH` is also trusted executable software and inherits the user's environment. Point it only at an official, trusted Codex CLI. Kernex bounds App Server JSON lines and request waits, kills and awaits the child on shutdown, maps the selected permission mode to Codex sandbox/approval policy, and routes command, file-change, and additional-permission requests through its audited approval surface. Codex still enforces the actual OS sandbox, so its installed version and configuration remain part of the security boundary.

Do not include credentials or private data in public vulnerability reports. Contact repository maintainers privately before publishing a security issue.
