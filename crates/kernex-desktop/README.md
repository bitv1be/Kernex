# Kernex Desktop

This directory contains the React/Vite frontend and the Tauri 2 host in `src-tauri`. The native host delegates agent execution, authentication, sessions, permissions, project tools, and provider behavior to `kernex-agent-core`.

From the repository root:

```bash
bun install --cwd crates/kernex-desktop --frozen-lockfile
bun run desktop:dev
```

Use the root `frontend:*` scripts for type checking, linting, tests, and production frontend builds.
