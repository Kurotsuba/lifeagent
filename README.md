# LifeAgent

A web-based AI agent that assists with daily life, shown in the UI as **HumanHealthHandler**.
The AI core is a replaceable cloud LLM (Gemini on Google Cloud's Agent Platform in the deployed
build); functionality comes from plugins. This `g-hackathon` branch is the entry for the
第5回 Agentic AI Hackathon with Google Cloud and ships the web build only, on Cloud Run
(`docs/HACKATHON.md`).

What it does today:

- **Diet** (always on): log meals in chat, nutrition computed in Rust from food tables (the
  LLM never produces final totals), daily targets from your profile and goals, and a next-day
  plan from the planner model, checked by a validator that blocks unsafe plans.
- **Optional plugins**, switched on in Settings, each with its own tab:
  - **Stock**: a pantry kept from chat, consumed by logged meals, used in plans and shopping lists.
  - **Training**: a workout log and a training plan section; the diet plan follows it.
  - **Members**: other people you eat with, their allergies and dislikes, and shared meals.
- **Agent governance**: sensitive tool calls (allergy changes, past-day deletes, member
  changes, …) wait for approval in the app; a read-only chat mode; an Assistant log of every
  agent run and its steps; per-user rate limits and a daily token budget.
- **Proactive follow-ups**: missed-meal check-ins and an evening review that offers to adjust
  tomorrow's plan, decided by plain code and answered with one tap.
- **Languages**: Japanese (default), English and Simplified Chinese; the assistant replies in
  the UI's language.

This README covers setup and day-to-day development commands. For design and behavior, see
`docs/ARCHITECTURE.md` (whole-project overview), `docs/PHASE1_SPEC.md` (the Phase 1 spec),
`docs/MULTIMEMBER_SPEC.md`, `docs/AUDIT_LOG_SPEC.md` and `docs/DECISIONS.md`. `CLAUDE.md` has the
repository conventions. `docs/archive/OPTIMIZATION_2026-10-11.md` describes the daily-life chat scope,
read-only chat, label nutrition calculations and blocking plan checks (merged in `437f2cf`).

## Prerequisites

- **Rust**, stable, 2021 edition (`rustup default stable`).
- **Node.js** 20+ and npm, for the web frontend.
- **PostgreSQL** 17 for local development and tests (see "Running it").
- **A model**: Gemini on Google Cloud's Agent Platform (no API key, see below), or an
  Anthropic / OpenAI-compatible key for the CLI. The CLI's `--mock` flag,
  `LIFEAGENT_MOCK_LLM=1` on the server and the test suite need none.

## Setup

```
git clone <this repo>
cd lifeagent
cargo build --workspace
cd apps/web && npm install
```

## Running it

**CLI** (any platform the Rust toolchain runs on), a developer harness — chat, inspect data,
generate plans without opening the web app:

```
cargo run -p lifeagent-cli -- --help
cargo run -p lifeagent-cli -- chat                       # interactive REPL
cargo run -p lifeagent-cli -- say "早餐吃了两个鸡蛋和一杯牛奶"
cargo run -p lifeagent-cli -- today                      # TodayView as JSON
cargo run -p lifeagent-cli -- plan generate
```

Data lives in Postgres. The CLI reads `LIFEAGENT_DATABASE_URL`
(default `postgres://lifeagent:lifeagent@localhost/lifeagent_dev`) and `LIFEAGENT_DB_SCHEMA`
(default `public`); the schema is created and migrated on start. The CLI also accepts
`--database-url` and `--schema`. A separate schema is handy for trying things out without
touching your real data:

```
cargo run -p lifeagent-cli -- --schema scratch today
```

Setting up the local Postgres (PostgreSQL 17,
role `lifeagent`, databases `lifeagent_dev` and `lifeagent_test`) is described in
`docs/HACKATHON.md` §1 and the 2026-10-02 "Postgres port" entry in `docs/DECISIONS.md`.

`--mock` swaps in a `MockChatModel` that just echoes input, for smoke-testing plumbing without
an API key or network access — it will not produce a real reply, meal parse or plan.

**Web server** (the hackathon's Cloud Run build, `docs/HACKATHON.md` §1): accounts with
per-user Postgres schemas, and the commands of `crates/api` as `POST /api/{command}`
(JSON arguments, snake_case). Locally, with the echo model and no UI:

```
LIFEAGENT_MOCK_LLM=1 cargo run -p lifeagent-server        # http://localhost:8080
curl -c jar -H 'content-type: application/json' \
  -d '{"username":"me","password":"password123"}' localhost:8080/api/auth/register
curl -b jar -H 'content-type: application/json' -d '{"text":"你好"}' localhost:8080/api/send_message
```

The UI lives in `apps/web`. To work on it, run the server as above and
`cd apps/web && npm run dev`, then open http://localhost:1420 (Vite proxies
`/api` to the server). For a single-process setup, `npm run build` and point
`LIFEAGENT_STATIC_DIR` at `apps/web/dist`.

With a real model, Gemini on Google Cloud's Agent Platform needs no API key: run
`gcloud auth application-default login` once, then set `LIFEAGENT_MODELS` to the
`"auth": "google_cloud"` config in `docs/HACKATHON.md` §2 instead of `LIFEAGENT_MOCK_LLM=1`.

Configuration is all environment variables (`crates/server/src/config.rs`):
`LIFEAGENT_DATABASE_URL` (its role needs `CREATEROLE`), `LIFEAGENT_MODELS` (models config JSON,
keys in `LIFEAGENT_KEY_<PROVIDER_ID>`) or `LIFEAGENT_MOCK_LLM=1`, `LIFEAGENT_STATIC_DIR` (built
UI to serve), `LIFEAGENT_TICK_TOKEN` (enables `/api/internal/tick`),
`LIFEAGENT_INPROCESS_SCHEDULER=1`, `LIFEAGENT_DB_PASSWORD`, `LIFEAGENT_DAILY_TOKEN_BUDGET`
(LLM tokens per user per 24 hours; unset means no cap), `LIFEAGENT_LOG_FORMAT=json` (Cloud
Logging structured logs instead of text), `PORT`.

Deploying to Cloud Run: every push to `g-hackathon` runs `.github/workflows/ci.yml`, which
runs fmt, clippy, the tests (against a Postgres 17 service) and the frontend build, then
deploys with keyless Workload Identity Federation (set up once by `deploy/setup-ci.sh`).
`deploy/deploy.sh` (Cloud Build, Cloud Run, Cloud Scheduler) does the same by hand. The
non-secret environment, including the per-role Gemini models and the 500k daily token budget,
is in `deploy/env.yaml`. The one-time Google Cloud setup is the console checklist in
`docs/HACKATHON.md` §1.

### Configuring a model provider for the CLI

In the web build the operator configures models for everyone through `LIFEAGENT_MODELS`. The
CLI stores its own config:

```
cargo run -p lifeagent-cli -- models set-json <path-to-a-models-config.json>
cargo run -p lifeagent-cli -- models set-key <provider_id>   # key read from stdin, hidden
```

See `docs/PHASE1_SPEC.md` §6.3 for the exact `ModelsConfig` JSON shape. On Linux the key goes
to the D-Bus Secret Service; on WSL2 see the 2026-09-25 "Setting up a working OS keyring in
WSL" entry in `docs/DECISIONS.md`.

## Development commands

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/web && npx tsc -b               # frontend typecheck; no frontend test runner (thin view layer, no logic to test)
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
crates/plugin-stock/     lifeagent-plugin-stock: optional pantry, consumption, stock-aware plan section
crates/plugin-training/  lifeagent-plugin-training: optional training log + training plan section
crates/plugin-multimember/ lifeagent-plugin-multimember: optional other members + shared-meal section
crates/api/              lifeagent-api: command surface + error mapping used by the server
crates/cli/              lifeagent-cli: developer harness (chat, plan, inspect data)
crates/server/           lifeagent-server: HTTP backend for the web build (accounts, per-user apps)
apps/web/                Vite + React + TypeScript, thin view layer (i18n tables in src/i18n/)
deploy/                  Cloud Run deploy script, environment and CI setup
docs/                    ARCHITECTURE.md, PHASE1_SPEC.md, DECISIONS.md, HACKATHON.md, ...
```

Dependency rule: `llm` <- `core` <- `plugin-*` <- `api` <- `server`; `cli` uses `core` and
the plugins directly. `core` never depends on any plugin crate, and plugins never depend on each
other (they share data only by event type and plan section name). Plugins are registered in
`crates/cli/src/main.rs` and `crates/server/src/lib.rs`.

## Troubleshooting

- **`secret store error: ...`** from `lifeagent-cli models set-key` on Linux/WSL2 — see
  "Configuring a model provider for the CLI" above.
- **A command fails with a "model not configured" error** (`NotConfigured`) — set
  `LIFEAGENT_MODELS` (server) or run `lifeagent-cli models set-json` (CLI).
- Logs go to stdout (Cloud Logging on Cloud Run), INFO by default; set `RUST_LOG=debug` for the
  server or `LIFEAGENT_LOG=debug` for the CLI (any `tracing_subscriber::EnvFilter` string). Per `CLAUDE.md`, API keys and full prompts never appear at INFO or above —
  DEBUG can be more verbose but still never logs a key.

## License

LifeAgent is released under the [MIT License](LICENSE). Third-party Rust crates and npm packages
keep their own licenses; see each package's metadata (`cargo metadata`, `apps/web/node_modules`).
