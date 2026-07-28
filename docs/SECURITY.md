# Security Model

Kernex assumes model output, repository content, plugin manifests, MCP servers, and language servers may be untrusted. The user and operating system remain the security principals.

## Enforced controls

- Filesystem tools canonicalize the project root and reject absolute escapes, `..` traversal, and symlinks resolving outside the project.
- File mutations require approval, include the proposed diff in the permission request, and use same-directory atomic replacement while preserving existing permissions.
- Commands are launched as a program plus argument vector, never by implicit shell evaluation.
- Command runtime and captured output are bounded. Destructive programs, shells, and mutating Git operations receive elevated risk labels.
- Child environments are filtered for names commonly associated with keys, tokens, passwords, authentication, sessions, cookies, and proxies.
- Provider and extension credentials are configured by environment-variable name. Sharing each credential requires permission.
- Session grants are scoped to both capability and resource; approving one credential or file does not approve other resources.
- Common credential paths such as `.env.*`, private keys, and cloud credential directories require secret access, are skipped by search, and are redacted from Git diffs.
- Model network requests, MCP/LSP startup, MCP tool calls, writes, and commands are visible and permission-gated.
- Provider response bodies, MCP/LSP messages, extension request time, pagination, command output, and command runtime are bounded.
- Closing the desktop application or pressing Cancel denies pending approvals and interrupts in-flight provider and tool futures; spawned commands are configured to terminate when their future is dropped.
- Edit events record the target and byte counts rather than duplicating complete file bodies; the resulting diff remains available for review.

## Important limitations

Approval is not an operating-system sandbox. An approved executable runs with the user's OS account and can access resources that account can access. Working-directory confinement does not constrain the executable itself. Review the exact command, arguments, risk label, and extension source before approval.

Environment-name filtering is defense in depth, not a guarantee that a value is non-sensitive. Do not store secrets in repository files or ordinary environment variables with misleading names. A model provider receives the conversation and relevant tool results after network approval; apply the provider's own data-handling policy.

MCP servers and plugins are executable software. Keep project configuration under review, pin extension versions outside Kernex where possible, and grant session-wide permissions only to trusted components.

Do not include credentials or private data in public vulnerability reports. Contact repository maintainers privately before publishing a security issue.
