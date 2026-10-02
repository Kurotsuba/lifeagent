# CLAUDE.md

This file tells Claude Code how to work in this repository. Read it fully before making changes.

## Project

**LifeAgent** is a Windows desktop AI agent that assists with daily life. The AI core is a
replaceable cloud LLM. Functionality is delivered by plugins. The first plugins track diet,
exercise, alcohol and smoking and generate a next-day plan from the user's health goals.

- Whole-project overview: `docs/ARCHITECTURE.md`
- Current work (Phase 1, detailed spec): `docs/PHASE1_SPEC.md`

When the spec and this file disagree, the spec wins for Phase 1 behavior; this file wins for
conventions. If something is ambiguous, pick the simplest option consistent with the spec,
note the decision in `docs/DECISIONS.md` (create it if missing), and continue.

## Primary language

Rust for everything with logic. The desktop UI is a thin TypeScript/React layer inside Tauri
that only renders data and forwards user input. No business logic in the frontend.

## Repository layout

```
Cargo.toml              # workspace
crates/llm/             # lifeagent-llm: model abstraction, providers, router (no dependency on core)
crates/core/            # lifeagent-core: storage, plugin API, orchestrator, planner, scheduler
crates/plugin-diet/     # lifeagent-plugin-diet: diet tracking + diet plan section
crates/api/             # lifeagent-api: command surface + error mapping shared by desktop and server
crates/cli/             # lifeagent-cli: developer harness (chat, plan, inspect data)
crates/server/          # lifeagent-server: HTTP backend for the web build (accounts, per-user apps)
apps/desktop/src-tauri/ # Tauri 2 shell (Rust), composition root for the desktop app
apps/desktop/ui/        # Vite + React + TypeScript, thin view layer
docs/
```

Dependency rule: `llm` <- `core` <- `plugin-*` <- `api` <- (`cli`, `desktop`, `server`).
`core` must never depend on any plugin crate. Plugins are registered in the composition roots
(`cli`, `desktop` and `server`).

## Commands

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p lifeagent-cli -- --help
cd apps/desktop && npm install && npm run tauri dev
```

Run fmt, clippy and tests after finishing each milestone. All tests must pass offline:
never call a real LLM in tests; use `MockChatModel` from `lifeagent-llm`.

## Conventions

- Async runtime: tokio. Traits used as trait objects use `async_trait`.
- Errors: `thiserror` enums in library crates, `anyhow` only in `cli` and `desktop`.
- No `unwrap()`/`expect()` outside tests, except for truly impossible states with a comment.
- Logging: `tracing`. Never log API keys, full prompts containing personal data at INFO or above
  (DEBUG is fine), or raw request headers.
- Database: SQLite via `sqlx` using runtime queries (`sqlx::query`, `query_as`), not the
  compile-time checked macros. Core migrations are embedded with `sqlx::migrate!`.
- Time: store instants as UTC RFC 3339 strings. Every user record also stores a
  `logical_date` (see "Time handling" in the spec). Always get "now" from the injected `Clock`,
  never from `Utc::now()` directly, so tests can control time.
- IDs: UUID v7 as strings.
- Serialization: `serde` with `snake_case` field names for everything crossing a boundary
  (DB JSON columns, Tauri commands, LLM tool arguments).
- Numbers shown to the user (calories, macros, targets) are always computed in Rust code.
  The LLM may propose foods and portions, never final nutrition totals.
- UI strings are Simplified Chinese (zh-CN) in Phase 1. Keep them in one file
  (`apps/desktop/ui/src/i18n/zh-CN.ts`). The assistant replies in the language the user writes in.
- Keep dependencies minimal. Add a crate only when it removes meaningful code or risk.

## Workflow

Implement Phase 1 milestone by milestone in the order given in `docs/PHASE1_SPEC.md` §14.
Each milestone lists "done when" criteria; do not start the next milestone until they hold.
Write unit tests alongside code, especially for the nutrition targets engine, the food
resolver, the plan validator, logical-date computation, and the orchestrator tool loop.
