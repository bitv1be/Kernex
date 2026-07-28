# Repository Guidelines

## Project Structure & Module Organization

Kernex is a Rust 2024 workspace. `src/main.rs` contains the `kernex` CLI. Shared agent behavior belongs in `crates/agent-core/src/`; keep provider, authentication, session, permission, tool, and protocol concerns in focused modules there. `crates/kernex-desktop/` hosts the Tauri 2 backend and its React/TypeScript UI in `ui/`; both clients must consume the shared core rather than duplicate it. User-facing architecture and security notes live in `docs/`. Generated `target/`, `node_modules/`, and Vite `dist/` directories remain untracked.

## Build, Test, and Development Commands

- `cargo run -- inspect` builds the CLI and summarizes this project.
- `cargo run -- run --provider local --model MODEL "TASK"` exercises the agent against a local compatible endpoint.
- `npm run desktop:dev` launches the Vite frontend and native Tauri application.
- `npm run frontend:typecheck`, `npm run frontend:lint`, `npm run frontend:test`, and `npm run frontend:build` validate the desktop frontend.
- `cargo check` performs a fast type and borrow check without producing a final executable.
- `cargo build --workspace` builds the CLI, core, and desktop crates.
- `cargo test --workspace` runs all unit, integration, and documentation tests.
- `cargo fmt --all -- --check` verifies formatting without changing files.
- `cargo clippy --all-targets --all-features -- -D warnings` runs the standard Rust lints and treats warnings as failures.

Run formatting, Clippy, and tests before opening a pull request.

## Coding Style & Naming Conventions

Use standard `rustfmt` output and four-space indentation; format changes with `cargo fmt --all`. Follow Rust naming conventions: `snake_case` for functions, variables, and modules; `PascalCase` for structs, enums, and traits; and `SCREAMING_SNAKE_CASE` for constants. Prefer focused modules, explicit error handling, and descriptive names over abbreviations. Document public APIs with `///` comments, including examples when behavior is not obvious.

## Testing Guidelines

Use Rust's built-in test harness. Keep unit tests beside their implementation in a `#[cfg(test)] mod tests` block, and use each crate's `tests/` directory for behavior spanning public interfaces. Name tests after observable behavior, such as `rejects_parent_traversal`. Cover provider wire formats, permission boundaries, path escapes, protocol framing, and regression cases. No coverage threshold is configured; prioritize success, failure, and security boundaries.

## Commit & Pull Request Guidelines

The repository has no commit history from which to infer an established convention. Use concise, imperative subjects such as `Add argument parsing`, and keep each commit focused on one concern. Pull requests should explain the motivation and approach, list validation commands run, and link relevant issues. Include sample terminal output when command-line behavior changes. Keep unrelated refactors out of feature or bug-fix pull requests.
