# LifeAgent

A Windows desktop AI agent that assists with daily life. The AI core is a replaceable cloud
LLM; functionality comes from plugins. Phase 1 ships one plugin: diet tracking and a next-day
plan generated from your health goals.

This README covers setup and day-to-day development commands. For what the app actually does,
see `docs/ARCHITECTURE.md` (whole-project overview) and `docs/PHASE1_SPEC.md` (the detailed
Phase 1 spec this codebase implements). `CLAUDE.md` has the repository conventions.

## Prerequisites

- **Rust**, stable, 2021 edition (`rustup default stable`).
- **Node.js** 20+ and npm, for the desktop app's frontend.
- **An LLM provider API key** — either an Anthropic key, or any OpenAI-compatible Chat
  Completions endpoint's key (see "Configuring a model provider" below). Nothing in this repo
  works end to end without one; the CLI's `--mock` flag and the test suite are the only paths
  that don't need one.
- **Windows**, to build and run the actual desktop app: the bundler targets NSIS
  (`apps/desktop/src-tauri/tauri.conf.json`), autostart uses the Windows registry, and API
  keys are stored in Windows Credential Manager. The Rust crates (`lifeagent-core`,
  `lifeagent-llm`, `lifeagent-plugin-diet`, `lifeagent-cli`) are plain, cross-platform Rust and
  build and test fine on Linux/macOS too — that's normal for iterating on core/plugin logic
  without a Windows machine, but `apps/desktop` itself needs Windows (or a Windows VM) to run.
  See "Cross-compiling the desktop app from Linux" below if you only have Linux/WSL.

On Linux (including WSL2), the OS keyring backend needs a running D-Bus Secret Service, which
most minimal setups (WSL2 in particular) don't have by default. This only affects the desktop
app and `lifeagent-cli models set-key` — the test suite never touches a real keyring
(`MemorySecretStore` stands in). If you hit `secret store error: ...` while testing manually
on WSL2, see the 2026-09-25 "Setting up a working OS keyring in WSL" entry in
`docs/DECISIONS.md` for the workaround (and its limits — it's for manual testing only, and
irrelevant on the real target platform, Windows).

## Setup

```
git clone <this repo>
cd life-agent
cargo build --workspace
cd apps/desktop && npm install
```

`npm install` at `apps/desktop` installs both the Tauri CLI (`apps/desktop/package.json`) and
the frontend's own dependencies (`apps/desktop/ui`, an npm workspace member) in one step.

## Running it

**Desktop app** (Windows):

```
cd apps/desktop
npm run tauri dev
```

First run opens onboarding: enter your profile and a weight goal, then configure a model
provider (see below) — "测试连接" must succeed before you can finish. After that you land on
Today/Chat/Settings.

**CLI** (any platform the Rust toolchain runs on), a developer harness — chat, inspect data,
generate plans without opening the desktop app:

```
cargo run -p lifeagent-cli -- --help
cargo run -p lifeagent-cli -- chat                       # interactive REPL
cargo run -p lifeagent-cli -- say "早餐吃了两个鸡蛋和一杯牛奶"
cargo run -p lifeagent-cli -- today                      # TodayView as JSON
cargo run -p lifeagent-cli -- plan generate
```

Data lives in Postgres. Both the desktop app and the CLI read `LIFEAGENT_DATABASE_URL`
(default `postgres://lifeagent:lifeagent@localhost/lifeagent_dev`) and `LIFEAGENT_DB_SCHEMA`
(default `public`); the schema is created and migrated on start. The CLI also accepts
`--database-url` and `--schema`. A separate schema is handy for trying things out without
touching your real data:

```
cargo run -p lifeagent-cli -- --schema scratch today
```

The desktop app still writes its logs to the OS's standard app-data directory
(`%APPDATA%\LifeAgent\data\logs` on Windows). Setting up the local Postgres (PostgreSQL 17,
role `lifeagent`, databases `lifeagent_dev` and `lifeagent_test`) is described in
`docs/HACKATHON.md` §1 and the 2026-10-02 "Postgres port" entry in `docs/DECISIONS.md`.

`--mock` swaps in a `MockChatModel` that just echoes input, for smoke-testing plumbing without
an API key or network access — it will not produce a real reply, meal parse or plan.

### Configuring a model provider

Onboarding's model step (or Settings → 模型 later) needs, for whichever provider you use:

- **Anthropic**: provider kind "Anthropic", base URL `https://api.anthropic.com`, your API
  key, and a model name per role (defaults suggested: Haiku for parsing, Opus for planning,
  Sonnet for conversation — you can also use one model for all three roles).
- **An OpenAI-compatible endpoint**: provider kind "OpenAI-compatible", that endpoint's base
  URL and key, and its model name(s).
- **Google Gemini** (through its OpenAI-compatible endpoint): provider kind "OpenAI-compatible",
  base URL `https://generativelanguage.googleapis.com/v1beta/openai`, a Google AI Studio API
  key, and Gemini model names (e.g. `gemini-2.5-flash` for parsing, a Pro or Gemini 3 model for
  planning and conversation). For Gemini 3 models the per-role temperature is not sent, so they
  run at Google's recommended default.

From the CLI instead, without onboarding's UI:

```
cargo run -p lifeagent-cli -- models set-json <path-to-a-models-config.json>
cargo run -p lifeagent-cli -- models set-key <provider_id>   # key read from stdin, hidden
```

See `docs/PHASE1_SPEC.md` §6.3 for the exact `ModelsConfig` JSON shape.

### Cross-compiling the desktop app from Linux

Building and running `apps/desktop` day to day needs Windows, but a one-off NSIS installer can
be cross-built from Linux/WSL with [`cargo-xwin`](https://github.com/rust-cross/cargo-xwin)
(`cargo install cargo-xwin`, add the `x86_64-pc-windows-msvc` target via `rustup target add`,
then point `apps/desktop`'s `npm run tauri build` at that target with `cargo-xwin` as the
runner — see that project's own docs for the exact Tauri-CLI flags, which change between
versions). This is how the M7.6 acceptance pass produced an installer to test on a real Windows
machine from this sandbox; see `docs/DECISIONS.md`'s 2026-09-26 M7.6 entries for what was
actually verified working (the installer itself, single instance, quick-entry, and the tray's
生成明日计划 toast).

## Development commands

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop/ui && npx tsc -b        # frontend typecheck; no frontend test runner (thin view layer, no logic to test)
```

Run `fmt`, `clippy` and `cargo test --workspace` after finishing each milestone's work — see
`CLAUDE.md`. All tests run offline: they never call a real LLM (`MockChatModel` from
`lifeagent-llm` stands in) and never touch a real OS keyring (`MemorySecretStore` stands in).

Database tests need a local Postgres: `LIFEAGENT_TEST_DATABASE_URL`, default
`postgres://lifeagent:lifeagent@localhost/lifeagent_test`, a superuser. On first use it creates
the non-superuser role the tests actually run as (`lifeagent_test_admin`, override with
`LIFEAGENT_TEST_ADMIN_URL`). Each test is its own tenant, a `test_<uuid>` schema and role
dropped afterwards, so tests run in parallel and leave nothing behind. If
the server is down, start it with `sudo service postgresql start`.

## Repository layout

```
Cargo.toml              # workspace
crates/llm/              lifeagent-llm: model abstraction, providers, router
crates/core/             lifeagent-core: storage, plugin API, orchestrator, planner, scheduler
crates/plugin-diet/      lifeagent-plugin-diet: diet tracking + diet plan section
crates/cli/              lifeagent-cli: developer harness (chat, plan, inspect data)
apps/desktop/src-tauri/  Tauri 2 shell (Rust), composition root for the desktop app
apps/desktop/ui/         Vite + React + TypeScript, thin view layer
docs/                    ARCHITECTURE.md, PHASE1_SPEC.md, DECISIONS.md, ...
```

Dependency rule: `llm` <- `core` <- `plugin-*` <- (`cli`, `desktop`). `core` never depends on
any plugin crate.

## Troubleshooting

- **`secret store error: ...`** on Linux/WSL2 — see "Prerequisites" above.
- **Onboarding's "测试连接" fails** — check the base URL (no trailing path for Anthropic;
  include the full completions path an OpenAI-compatible endpoint expects), the key, and that
  the model name(s) exist for that provider. The error message names which role failed.
- **A command fails with "尚未完成模型配置..."** — no provider is configured yet, or the
  stored one no longer builds (e.g. a keyring entry went missing); redo the model step in
  Settings.
- Logs live in `<data dir>/logs/lifeagent.<date>.log` (14 days kept), INFO by default; set
  `LIFEAGENT_LOG=debug` (or another `tracing_subscriber::EnvFilter` string) to get more detail
  for development. Per `CLAUDE.md`, API keys and full prompts never appear at INFO or above —
  DEBUG can be more verbose but still never logs a key.
