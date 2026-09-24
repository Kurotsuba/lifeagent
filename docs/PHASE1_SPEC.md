# LifeAgent — Phase 1 Specification: Core Loop

This is the implementation spec for Phase 1. It is written to be implemented directly.
Read `CLAUDE.md` for conventions and `ARCHITECTURE.md` for context.

## 0. How to use this document

- Implement milestones in §14 in order. Each has "done when" criteria.
- Code sketches show intended shapes, not exact code. Adjust signatures if needed but keep
  the responsibilities and boundaries.
- Where this spec gives a number (limits, defaults, formulas), use it exactly.
- Record any deviation or unresolved ambiguity in `docs/DECISIONS.md`.

## 1. Scope

### In scope
1. Cargo workspace with crates `llm`, `core`, `plugin-diet`, `cli`, and the Tauri app.
2. LLM layer: `ChatModel` trait, two providers — OpenAI-compatible Chat Completions and
   Anthropic Messages — both with tool calling, role router (parser / planner / conversation),
   retries, usage logging, mock model for tests.
3. Core: SQLite storage + migrations, settings, secrets (API keys in Windows Credential
   Manager), clock, logical dates, profile, goals, weight logging, event store + bus,
   plugin API, tool registry, context builder, memory notes, conversation store,
   orchestrator tool loop, plan service, scheduler.
4. Diet plugin: meal logging from free text, food resolver with local food table and cached
   LLM estimates, daily intake, nutrition targets engine, plan input + plan section
   finalization and validation, context snippet, dashboard card data.
5. Desktop app: onboarding, Today page, Chat page, Settings page, tray, close-to-tray,
   notifications, optional autostart.
6. CLI harness.

### Out of scope (later phases)
Exercise/alcohol/smoking plugins, trends charts, further providers (Gemini, ...), fallback models,
budget cap enforcement, food database import, photo logging, MCP, streaming responses,
multi-conversation threads, sync, mobile.

## 2. Repository layout and responsibilities

```
lifeagent/
├─ Cargo.toml                    # [workspace], shared [workspace.dependencies]
├─ CLAUDE.md
├─ docs/
├─ crates/
│  ├─ llm/          lifeagent-llm
│  │   src/lib.rs, types.rs, provider/mod.rs, provider/openai_compat.rs,
│  │       provider/anthropic.rs, router.rs, mock.rs, json.rs, error.rs
│  ├─ core/         lifeagent-core
│  │   migrations/  (sqlx migrations for core tables)
│  │   src/lib.rs, app.rs (CoreApp builder), clock.rs, time.rs, db.rs, settings.rs,
│  │       secrets.rs, profile.rs, goals.rs, events.rs, memory.rs, conversation.rs,
│  │       plugin.rs, tools.rs, core_tools.rs, context.rs, orchestrator.rs,
│  │       plan.rs, scheduler.rs, usage.rs, error.rs
│  ├─ plugin-diet/  lifeagent-plugin-diet
│  │   src/lib.rs, model.rs, foods.rs, resolver.rs, parser.rs, targets.rs,
│  │       intake.rs, tools.rs, plan.rs, validate.rs, prompts.rs
│  └─ cli/          lifeagent-cli
└─ apps/desktop/
   ├─ src-tauri/    Tauri 2 app: main.rs, commands.rs, tray.rs, notify.rs
   └─ ui/           Vite + React + TypeScript
```

Dependency rule: `llm` ← `core` ← `plugin-diet` ← (`cli`, `desktop`). `core` never imports
a plugin. The composition root builds `CoreApp` and registers plugins.

### Suggested crates

| Need | Crate |
|---|---|
| async | `tokio` (full), `async-trait` |
| serde | `serde`, `serde_json` |
| DB | `sqlx` (sqlite, runtime-tokio, migrate) |
| HTTP | `reqwest` (json, rustls-tls) |
| time | `chrono` |
| ids | `uuid` (v7) |
| errors | `thiserror`, `anyhow` (apps only) |
| logging | `tracing`, `tracing-subscriber`, `tracing-appender` |
| secrets | `keyring` |
| paths | `directories` |
| CLI | `clap` |
| desktop | `tauri` 2, `tauri-plugin-notification`, `tauri-plugin-autostart`, `tauri-plugin-single-instance` |
| UI | React, TypeScript, Vite, Tailwind CSS, `@tauri-apps/api` |

## 3. Paths

Data directory: `directories::ProjectDirs::from("", "", "LifeAgent").data_dir()`
(on Windows `%APPDATA%\LifeAgent\data`). Contents: `lifeagent.db`, `logs/`.
The CLI accepts `--data-dir <path>` to override (used for development and tests).

## 4. Time handling

- `Clock` trait: `fn now(&self) -> DateTime<Utc>`; `SystemClock` and `FixedClock` (tests,
  settable).
- Local timezone: `chrono::Local` in Phase 1 (store the IANA name is out of scope).
- **Day boundary** setting, default `04:00`. Anything at local time before the boundary belongs
  to the previous day (late-night snacks count toward the day the user is still living).
- `fn logical_date(instant: DateTime<Utc>, boundary: NaiveTime) -> NaiveDate`:
  convert to local, subtract `boundary` duration, take the date.
- "Today" = `logical_date(clock.now())`. "Tomorrow" = today + 1 day.
- Stored: `occurred_at` (UTC RFC 3339) and `logical_date` (`YYYY-MM-DD`) on every event.

Tests: 03:59 local on the 10th → logical date 9th; 04:00 → 10th.

## 5. Database schema

### Core migrations (`crates/core/migrations/0001_init.sql`)

```sql
CREATE TABLE settings (
  key         TEXT PRIMARY KEY,
  value_json  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE profile (
  id          INTEGER PRIMARY KEY CHECK (id = 1),   -- single row
  data_json   TEXT NOT NULL,                        -- Profile struct
  updated_at  TEXT NOT NULL
);

CREATE TABLE goals (
  id          TEXT PRIMARY KEY,
  domain      TEXT NOT NULL,          -- 'body' in Phase 1
  kind        TEXT NOT NULL,          -- 'weight'
  params_json TEXT NOT NULL,
  active      INTEGER NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE events (
  id            TEXT PRIMARY KEY,
  event_type    TEXT NOT NULL,        -- 'diet.meal_logged', 'body.weight_logged', ...
  occurred_at   TEXT NOT NULL,
  logical_date  TEXT NOT NULL,
  payload_json  TEXT NOT NULL,
  source        TEXT NOT NULL,        -- 'chat' | 'ui' | 'system'
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  deleted_at    TEXT
);
CREATE INDEX idx_events_type_date ON events(event_type, logical_date) WHERE deleted_at IS NULL;

CREATE TABLE plans (
  plan_date     TEXT NOT NULL,
  version       INTEGER NOT NULL,
  status        TEXT NOT NULL,        -- 'proposed' | 'accepted'
  trigger       TEXT NOT NULL,        -- 'schedule' | 'manual' | 'revision'
  instruction   TEXT,                 -- revision instruction, if any
  content_json  TEXT NOT NULL,        -- PlanContent
  warnings_json TEXT NOT NULL,        -- Vec<String>
  created_at    TEXT NOT NULL,
  PRIMARY KEY (plan_date, version)
);

CREATE TABLE memory_notes (
  id          TEXT PRIMARY KEY,
  note        TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  deleted_at  TEXT
);

CREATE TABLE messages (
  id              TEXT PRIMARY KEY,
  role            TEXT NOT NULL,      -- 'user' | 'assistant' | 'tool'
  content         TEXT,
  tool_calls_json TEXT,               -- for assistant messages with tool calls
  tool_call_id    TEXT,               -- for tool messages
  tool_name       TEXT,
  created_at      TEXT NOT NULL
);

CREATE TABLE llm_usage (
  id                 TEXT PRIMARY KEY,
  role               TEXT NOT NULL,
  provider_id        TEXT NOT NULL,
  model              TEXT NOT NULL,
  prompt_tokens      INTEGER NOT NULL,
  completion_tokens  INTEGER NOT NULL,
  latency_ms         INTEGER NOT NULL,
  ok                 INTEGER NOT NULL,
  created_at         TEXT NOT NULL
);

CREATE TABLE plugin_migrations (
  plugin_id   TEXT NOT NULL,
  version     INTEGER NOT NULL,
  applied_at  TEXT NOT NULL,
  PRIMARY KEY (plugin_id, version)
);
```

### Plugin migrations
Plugins supply `Vec<PluginMigration { version: u32, sql: &'static str }>`. Core applies
missing ones in order inside a transaction and records them in `plugin_migrations`.
Plugin tables must be prefixed with the plugin id (`diet_`).

### Diet plugin migration 1

```sql
CREATE TABLE diet_foods (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,        -- canonical display name, user's language
  name_key      TEXT NOT NULL UNIQUE, -- normalized: lowercase, trimmed, collapsed spaces
  aliases_json  TEXT NOT NULL DEFAULT '[]',   -- normalized alias keys
  kcal          REAL NOT NULL,        -- per 100 g
  protein_g     REAL NOT NULL,
  carbs_g       REAL NOT NULL,
  fat_g         REAL NOT NULL,
  fiber_g       REAL NOT NULL DEFAULT 0,
  source        TEXT NOT NULL,        -- 'llm_estimate' | 'user'
  use_count     INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
```

Meals are stored as core `events` (`diet.meal_logged`), not in a plugin table.

## 6. LLM layer (`lifeagent-llm`)

### 6.1 Types

```rust
pub enum Role { Parser, Planner, Conversation }

pub enum Message {
    System { content: String },
    User { content: String },
    Assistant { content: Option<String>, tool_calls: Vec<ToolCall> },
    Tool { tool_call_id: String, content: String },
}

pub struct ToolCall { pub id: String, pub name: String, pub arguments: serde_json::Value }

pub struct ToolSpec {
    pub name: String,                 // ^[a-z][a-z0-9_]{0,63}$
    pub description: String,
    pub parameters: serde_json::Value // JSON Schema object
}

pub enum ResponseFormat { Text, JsonObject }

pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub response_format: ResponseFormat,
}

pub struct Usage { pub prompt_tokens: u32, pub completion_tokens: u32 }

pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: String,
}

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
}
```

`LlmError` variants: `Http`, `Status { code, body_excerpt }`, `RateLimited`, `Timeout`,
`InvalidResponse`, `Auth`, `Config`.

### 6.2 Providers

Two providers implement `ChatModel` in Phase 1. Anthropic's Messages API is not an
OpenAI-compatible endpoint — it differs in URL, auth header, message shape, tool schema nesting,
tool-result placement and parameter surface — so it cannot be reached by pointing the
OpenAI-compatible provider at a different `base_url`.

**Policy shared by all providers:**

- Request timeout 180 s (raised from 90 s because Anthropic models with adaptive thinking spend
  thinking tokens before emitting any output).
- Retries on 429, 5xx, 529 and timeout — up to 2 retries, backoff 1 s then 4 s (respect
  `Retry-After` if present, capped at 20 s). 401/403 → `Auth`, no retry. 400 → `Status`,
  no retry. (529 `overloaded_error` is Anthropic-specific but costs nothing to include here.)

#### 6.2.1 OpenAI-compatible

- `POST {base_url}/chat/completions`, header `Authorization: Bearer <key>`.
- Works with any OpenAI-compatible endpoint (OpenAI, DeepSeek, Qwen/DashScope compatible mode,
  OpenRouter, etc.); the user supplies `base_url`, which includes the `/v1` suffix if the
  endpoint needs one.
- Map `tools` to `[{ "type": "function", "function": { name, description, parameters } }]`.
- Parse `choices[0].message.tool_calls[*].function.arguments` (a JSON string) into `Value`;
  if it fails to parse, return `InvalidResponse`.
- `ResponseFormat::JsonObject` → `"response_format": {"type": "json_object"}`.
- `Message::System` is sent in the `messages` array as `{"role": "system", ...}`, and each
  `Message::Tool` is its own `{"role": "tool", "tool_call_id": ..., "content": ...}` message.

#### 6.2.2 Anthropic Messages

- `POST {base_url}/v1/messages`, default `base_url` `https://api.anthropic.com`. Unlike the
  OpenAI-compatible `base_url`, this one does **not** include `/v1` — the provider appends it.
- Headers: `x-api-key: <key>`, `anthropic-version: 2023-06-01`,
  `content-type: application/json`. Note that Anthropic authenticates with `x-api-key`, not
  `Authorization: Bearer`.
- `max_tokens` is **required** by the API. Default to 16000 when `ChatRequest.max_tokens` is
  `None`. Thinking tokens count against `max_tokens`, so a small value truncates mid-reasoning.
- **System hoisting.** `Message::System` has no place in the `messages` array. Concatenate the
  content of every `System` message, separated by a blank line, into the top-level `system`
  string field and drop them from `messages`.
- **Tool spec mapping.** Flat, not nested under `function`:
  `[{ "name": ..., "description": ..., "input_schema": <JSON Schema object> }]`. `ToolSpec`
  maps field for field, with `parameters` → `input_schema`.
- **Assistant mapping.** `content` is a block array: an optional
  `{"type": "text", "text": ...}` block followed by one
  `{"type": "tool_use", "id": ..., "name": ..., "input": <object>}` block per tool call.
  `input` is a JSON **object**, not the JSON-encoded string OpenAI uses, so there is no
  argument-parsing step and no `InvalidResponse` path for malformed tool arguments.
- **Tool-result merging.** OpenAI takes one `tool` message per call; Anthropic takes all results
  for one assistant turn as `tool_result` blocks inside a **single user message**. Collapse each
  run of consecutive `Message::Tool` values into one user message whose content is
  `[{"type": "tool_result", "tool_use_id": ..., "content": ...}, ...]`, preserving order.
  Splitting them across several user messages degrades parallel tool calling and breaks the
  strict user/assistant alternation the endpoint requires.
- **Response parsing.** Join all `content[]` blocks of `type == "text"` into
  `ChatResponse.content` (`None` if there are none); collect every `type == "tool_use"` block
  into `tool_calls` as `ToolCall { id, name, arguments: input }`. Map `stop_reason` straight
  through to `finish_reason`, and `usage.input_tokens` / `usage.output_tokens` to
  `Usage { prompt_tokens, completion_tokens }`.
- **`stop_reason: "refusal"`** returns HTTP 200 with no usable content. It surfaces as empty
  content and no tool calls, which the orchestrator tolerates (§7.11).
- **No `temperature`.** Current Anthropic models (`claude-opus-5`, `claude-sonnet-5`, the
  4.7/4.8 family) reject `temperature`, `top_p` and `top_k` with a 400. This provider never
  sends them; `RoleBinding.temperature` is ignored for `ProviderKind::Anthropic`.
- **No JSON mode.** There is no `response_format: {"type": "json_object"}` equivalent. For
  `ResponseFormat::JsonObject` the provider sends nothing extra and relies on the prompt plus
  the fence-stripping and one-shot repair loop of §6.4 — which is why that repair loop exists
  and must not be dropped. (Anthropic structured outputs via `output_config.format` would be
  stricter but need a JSON Schema, which `complete_json` does not have; revisit in Phase 3.)

### 6.3 Router

```rust
pub struct ProviderConfig { pub id: String, pub kind: ProviderKind, pub base_url: String }
pub enum ProviderKind { OpenAiCompatible, Anthropic }
pub enum Effort { Low, Medium, High }
pub struct RoleBinding {
    pub provider_id: String,
    pub model: String,
    pub temperature: Option<f32>,   // OpenAI-compatible only
    pub effort: Option<Effort>,     // Anthropic only
}
pub struct ModelsConfig {
    pub providers: Vec<ProviderConfig>,
    pub parser: RoleBinding,
    pub planner: RoleBinding,
    pub conversation: RoleBinding,
}

pub struct LlmRouter { /* providers: HashMap<String, Arc<dyn ChatModel>>, config, usage sink */ }
impl LlmRouter {
    pub async fn complete(&self, role: Role, req: RoleRequest) -> Result<ChatResponse, LlmError>;
}
```

`RoleRequest` is `ChatRequest` without `model`/`temperature`; the router fills them from the
binding. The router calls a `UsageSink` trait (implemented in core, writes `llm_usage`) after
every call, success or failure.

Per-role tuning depends on the provider kind, because the two APIs expose different levers:

| Kind | Lever | Parser | Planner | Conversation |
|---|---|---|---|---|
| `OpenAiCompatible` | `temperature` | 0.0 | 0.7 | 0.5 |
| `Anthropic` | `output_config.effort` | `low` | `high` | `medium` |

Each provider ignores the other's field. Effort matters: the parser role is the app's most
frequent call, and leaving it at the API default (`high`) makes every food lookup slow and
expensive.

API keys are not part of `ModelsConfig`; the router is constructed with a
`KeyProvider` (implemented in core by the secrets module).

### 6.4 JSON helper

```rust
pub async fn complete_json<T: DeserializeOwned>(
    router: &LlmRouter, role: Role, messages: Vec<Message>
) -> Result<T, LlmError>
```
Calls with `JsonObject`; strips accidental ```json fences; deserializes. On failure, one
repair attempt: appends the model's output as an assistant message and a user message
"Your previous output was not valid for the required schema: {serde error}. Return only
corrected JSON." Second failure → `InvalidResponse`.

### 6.5 Mock

`MockChatModel` holds a queue of scripted `ChatResponse`s (or closures receiving the request)
and records received requests for assertions. Used by all core and plugin tests.

## 7. Core (`lifeagent-core`)

### 7.1 CoreApp

```rust
pub struct CoreApp { /* db, clock, settings, secrets, router, events, plugins, registry, ... */ }

pub struct CoreAppBuilder { /* data_dir, clock, plugins: Vec<Arc<dyn Plugin>>, llm override */ }
impl CoreAppBuilder {
    pub fn new(data_dir: PathBuf) -> Self;
    pub fn clock(self, clock: Arc<dyn Clock>) -> Self;
    pub fn plugin(self, p: Arc<dyn Plugin>) -> Self;
    pub fn chat_model_override(self, m: Arc<dyn ChatModel>) -> Self; // tests & CLI --mock
    pub async fn build(self) -> Result<CoreApp, CoreError>;           // opens DB, runs migrations
}

impl CoreApp {
    pub async fn send_message(&self, text: &str) -> Result<ChatTurn, CoreError>;
    pub async fn today_view(&self) -> Result<TodayView, CoreError>;
    pub async fn get_plan(&self, date: NaiveDate) -> Result<Option<StoredPlan>, CoreError>;
    pub async fn generate_plan(&self, date: NaiveDate, trigger: PlanTrigger) -> Result<StoredPlan, CoreError>;
    pub async fn revise_plan(&self, date: NaiveDate, instruction: &str) -> Result<StoredPlan, CoreError>;
    pub async fn accept_plan(&self, date: NaiveDate) -> Result<(), CoreError>;
    pub async fn delete_event(&self, id: &str) -> Result<(), CoreError>;
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent>;
    pub fn start_scheduler(&self) -> JoinHandle<()>;
    // profile / goals / settings / secrets / memory / history getters & setters
}
```

`CoreApp` is `Clone` (inner `Arc`) so shells can share it.

### 7.2 Settings

Key-value in `settings` table, typed accessors with defaults:

| Key | Type | Default |
|---|---|---|
| `plan_time` | "HH:MM" | "21:00" |
| `day_boundary` | "HH:MM" | "04:00" |
| `models` | `ModelsConfig` | none (onboarding required) |
| `notifications_enabled` | bool | true |
| `autostart` | bool | false |
| `onboarding_complete` | bool | false |

Validation: `plan_time ≥ day_boundary`, rejected otherwise — the scheduler's catch-up logic
(§7.13) relies on this to anchor `plan_time` to the correct calendar date (B3 in
`docs/DECISIONS.md`).

### 7.3 Secrets

`SecretStore` trait with `get(provider_id)`, `set(provider_id, key)`, `delete(provider_id)`.
`KeyringSecretStore` uses service name `LifeAgent`, account `provider:{id}`.
`MemorySecretStore` for tests. Implements the LLM layer's `KeyProvider`.

### 7.4 Profile and goals

```rust
pub struct Profile {
    pub display_name: Option<String>,
    pub sex: Sex,                         // Male | Female
    pub birth_date: NaiveDate,
    pub height_cm: f64,
    pub activity_level: ActivityLevel,    // Sedentary | Light | Moderate | Active | VeryActive
    pub dietary_pattern: DietaryPattern,  // Omnivore | Vegetarian | Vegan | Pescatarian
    pub allergies: Vec<String>,
    pub dislikes: Vec<String>,
    pub medical_notes: Option<String>,    // free text; if non-empty, plans include a disclaimer
    pub meals_per_day: u8,                // 3..=5, default 3
}

pub struct WeightGoal {
    pub direction: GoalDirection,         // Lose | Maintain | Gain
    pub target_weight_kg: Option<f64>,
    pub rate_kg_per_week: f64,            // ignored for Maintain
}
```

Weight is not in the profile: it is the latest `body.weight_logged` event
(`{ "kg": f64 }`). Onboarding records the initial weight as an event.
Validation: height 120–230 cm, weight 30–300 kg, age 16–100, rate 0–1.0 kg/week.

Default target: when a Lose or Gain goal is saved without `target_weight_kg`, it is set without
asking the user to the latest weight − 1 kg (Lose) or + 1 kg (Gain), clamped to 30–300 kg, and
stored on the goal record. It is set once, at save time, and not recomputed. After the user
loses or gains that 1 kg, §8.6's "goal reached → treat as Maintain" rule applies. If no weight
has been logged when the goal is saved, the target stays unset and §8.6 falls back to current
weight. See B1 in `docs/DECISIONS.md`.

### 7.5 Events and event bus

```rust
pub struct Event {
    pub id: String, pub event_type: String,
    pub occurred_at: DateTime<Utc>, pub logical_date: NaiveDate,
    pub payload: serde_json::Value, pub source: EventSource,
}

pub struct EventStore { /* db, clock, settings, bus */ }
impl EventStore {
    pub async fn append(&self, event_type: &str, occurred_at: DateTime<Utc>,
                        payload: Value, source: EventSource) -> Result<Event, CoreError>;
    pub async fn list(&self, event_type: &str, from: NaiveDate, to: NaiveDate) -> Result<Vec<Event>, CoreError>;
    pub async fn latest(&self, event_type: &str) -> Result<Option<Event>, CoreError>;
    pub async fn soft_delete(&self, id: &str) -> Result<Event, CoreError>;
}
```

After `append`/`soft_delete` commit, the store publishes on the internal bus. A dispatcher
task delivers each event to every plugin's `on_event` (errors are logged, not propagated)
and emits `AppEvent::DataChanged { event_type, logical_date }` to shells.

```rust
pub enum AppEvent {
    DataChanged { event_type: String, logical_date: NaiveDate },
    PlanReady { date: NaiveDate, version: u32 },
    PlanFailed { date: NaiveDate, error: String },
}
```

### 7.6 Plugin API

```rust
pub struct PluginManifest {
    pub id: &'static str,            // "diet"; also the tool-name and table prefix
    pub name: &'static str,
    pub version: &'static str,
    pub event_types: Vec<&'static str>,
    pub migrations: Vec<PluginMigration>,
}

pub struct PluginContext {
    pub db: SqlitePool,
    pub events: EventStore,
    pub llm: Arc<LlmRouter>,
    pub clock: Arc<dyn Clock>,
    pub settings: Settings,
    pub profile: ProfileService,     // profile, active goals, latest weight
    pub memory: MemoryService,
}

pub struct ToolOutput {
    pub content: serde_json::Value,  // returned to the LLM as JSON text
    pub summary: String,             // short human-readable line for UI activity chips
}

pub struct PlanInput {
    pub section: String,             // = plugin id
    pub brief: String,               // plain-text context and constraints for the planner
    pub targets: serde_json::Value,  // deterministic numbers the plan must meet
    pub output_schema: serde_json::Value, // JSON Schema of the section the planner must produce
}

pub struct PlanSection {
    pub data: serde_json::Value,     // resolved, enriched section stored in the plan
    pub warnings: Vec<String>,       // shown to the user, not retried
    pub violations: Vec<String>,     // hard failures, trigger one planner retry
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;
    fn tools(&self) -> Vec<ToolSpec>;
    async fn call_tool(&self, ctx: &PluginContext, name: &str, args: Value)
        -> Result<ToolOutput, PluginError>;
    async fn on_event(&self, _ctx: &PluginContext, _e: &Event) -> Result<(), PluginError> { Ok(()) }
    async fn context_snippet(&self, _ctx: &PluginContext, _date: NaiveDate)
        -> Result<Option<String>, PluginError> { Ok(None) }
    async fn today_card(&self, _ctx: &PluginContext, _date: NaiveDate)
        -> Result<Option<Value>, PluginError> { Ok(None) }
    async fn plan_input(&self, _ctx: &PluginContext, _date: NaiveDate)
        -> Result<Option<PlanInput>, PluginError> { Ok(None) }
    async fn finalize_plan_section(&self, _ctx: &PluginContext, _date: NaiveDate, _raw: Value)
        -> Result<PlanSection, PluginError> { Err(PluginError::Unsupported) }
}
```

`PluginError` variants: `InvalidArgs(String)` (returned to the LLM so it can fix the call),
`NotFound(String)`, `Llm(LlmError)`, `Db`, `Unsupported`, `Other(String)`.

### 7.7 Tool registry

- Tool names are globally unique and must start with `{plugin_id}_`; core tools start with
  `core_`. Registration fails on duplicates or bad prefixes.
- `ToolRegistry::specs()` returns all specs; `ToolRegistry::call(name, args)` routes to core or
  the owning plugin.
- Before dispatch, arguments are validated against the tool's JSON Schema for required fields
  and basic types (a small hand-written checker for `object`, `string`, `number`, `integer`,
  `boolean`, `array`, `enum`, `required` is enough; no full JSON Schema crate needed).

### 7.8 Core tools

| Tool | Args | Behavior |
|---|---|---|
| `core_log_weight` | `kg: number`, `measured_at?: string` (local ISO datetime) | Validates 30–300; appends `body.weight_logged` |
| `core_remember` | `note: string` (≤ 300 chars) | Adds a memory note for durable preferences/facts |
| `core_forget` | `note_id: string` | Soft-deletes a memory note |
| `core_get_plan` | `date?: string` (YYYY-MM-DD, default tomorrow if a plan exists, else today) | Returns current plan version |
| `core_generate_plan` | `date?: string` (default tomorrow) | Runs plan generation (trigger manual) |
| `core_revise_plan` | `instruction: string`, `date?: string` | Runs plan revision |
| `core_delete_entry` | `entry_id: string` | Soft-deletes any event by id |

Tool descriptions must state when to use them, e.g. `core_remember`: "Save a durable
preference or fact about the user that should influence future plans (e.g. 'no cilantro',
'weekday lunch is at the company canteen'). Do not use for one-off events."

### 7.9 Context builder

Builds the conversation system prompt (§9.1) from:
1. Current local date/time, weekday, logical date.
2. Profile summary (age computed from birth date, sex, height, latest weight and its date,
   activity level, dietary pattern, allergies, dislikes).
3. Active goals in one line each.
4. Each plugin's `context_snippet(today)` under a heading with the plugin name.
5. Today's plan and tomorrow's plan (if any), compact form (meal titles and item lists).
6. Memory notes (newest first, max 50), each with its id.

Target size < 2,500 tokens; truncate memory notes first if exceeded (estimate tokens as
chars / 3 for CJK-heavy text).

### 7.10 Conversation store and history window

Single conversation thread in Phase 1. History sent to the model: the last 30 stored messages,
trimmed so the window starts at a `user` message (never with orphan `tool` messages).
Tool message content stored and sent as compact JSON, truncated to 4,000 chars.

### 7.11 Orchestrator

```text
send_message(text):
  store user message
  for iteration in 1..=6:
      req = [system prompt] + history + tools = registry.specs()
      resp = router.complete(Conversation, req)
      store assistant message (content + tool_calls)
      if resp.tool_calls is empty:
          return ChatTurn { reply: resp.content or EMPTY_REPLY, activities }
      for call in resp.tool_calls (sequentially, in order):
          result = registry.call(call.name, call.arguments)
          on Ok(out):  content = out.content; activities.push(out.summary)
          on Err(InvalidArgs|NotFound): content = {"error": msg}
          on Err(other): content = {"error": "internal error"}; log it
          store tool message(call.id, content)
  return ChatTurn { reply: "（处理步骤过多，已停止。请换个说法再试一次。）", activities }
```

```rust
pub struct ChatTurn { pub reply: String, pub activities: Vec<String> }
```

A response with neither content nor tool calls ends the loop like any other tool-call-free
response. Since `ChatResponse.content` is `Option<String>` and `ChatTurn.reply` is `String`,
substitute the fixed zh-CN `EMPTY_REPLY` = "（模型没有返回内容，请再试一次。）" rather than
unwrapping. This is the normal shape of an Anthropic `stop_reason: "refusal"` (§6.2.2).

LLM errors in the conversation role are returned as `CoreError::Llm` and the shell shows a
friendly message (auth errors point the user to Settings).

A global `tokio::sync::Mutex` serializes `send_message` calls (one turn at a time).

### 7.12 Plan service

```rust
pub struct PlanContent {
    pub summary: String,                       // short message to the user
    pub sections: BTreeMap<String, Value>,     // plugin id -> PlanSection.data
    pub disclaimer: Option<String>,
}
pub struct StoredPlan {
    pub plan_date: NaiveDate, pub version: u32, pub status: PlanStatus,
    pub trigger: PlanTrigger, pub content: PlanContent, pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
}
```

Generation algorithm for date `D`:
1. Collect `plan_input(D)` from all plugins; skip `None`. If none, error `NothingToPlan`.
2. Build the planner prompt (§9.4) with all inputs, profile summary, memory notes, and for
   revisions the previous plan version plus the user's instruction.
3. `complete_json` with role `Planner`, expecting
   `{ "summary": string, "sections": { "<section>": <object> } }`.
4. For each input section, call the owner's `finalize_plan_section(D, raw)`.
   Missing section → violation "section missing".
5. If any violations: one retry — append the model output and a user message listing all
   violations per section, ask for a corrected full JSON; finalize again.
6. If violations remain after the retry: store the plan anyway with violations moved into
   `warnings`, prefixed "未完全满足：".
7. If `profile.medical_notes` is non-empty, set `disclaimer` to the fixed zh-CN text
   "你的档案中记录了健康状况，本计划仅为生活方式建议，请以医生或营养师意见为准。"
8. Insert as version = previous max + 1, status `proposed`. Emit `AppEvent::PlanReady`.

Revision uses the same algorithm with trigger `revision` and the instruction stored.
Only one generation runs at a time (mutex); a second request waits.

### 7.13 Scheduler

A tokio task started by the shell:
- Tick every 60 s and once immediately at start (catch-up).
- Let `today` = logical date now, `tomorrow` = today + 1. If local time is past `plan_time` on
  logical day `today` and no plan exists for `tomorrow`, generate one (trigger `schedule`).
  "Past `plan_time` on logical day `today`" means: `local_now ≥ due_at`, where `due_at` is the
  local datetime of `plan_time` **within logical day `today`** — its calendar date is `today`
  itself, **not** the calendar date of `local_now` at the moment of the check. Those two dates
  differ in the window between midnight and `day_boundary`, where the calendar date has already
  advanced but the logical day hasn't — exactly the window a relaunch after downtime spanning
  the previous evening's `plan_time` lands in (B3 in `docs/DECISIONS.md`; this is acceptance
  scenario §15.9). This calendar-date mapping is only valid because `plan_time ≥ day_boundary`
  is enforced by settings validation (§7.2) — a `plan_time` before the boundary would need a
  second anchoring rule, so it's rejected instead. Compute carefully and unit test it across
  several `plan_time`/`day_boundary` combinations, including a relaunch shortly after midnight
  with `plan_time` already passed the previous evening.
- Catch-up only ever targets `tomorrow`. If an outage is long enough that `today` itself also
  has no plan, catch-up does not generate one for `today` retroactively — the Today page's
  existing empty state (`today_plan: None`) and its 立即生成 button cover that case.
- Failure: emit `PlanFailed`, retry after 15 min, at most 3 attempts per target date
  (attempt counter in memory is fine).
- Skip entirely if onboarding is not complete or `models` is not configured.

### 7.14 Today view

```rust
pub struct TodayView {
    pub logical_date: NaiveDate,
    pub cards: BTreeMap<String, Value>,      // plugin id -> today_card(date)
    pub today_plan: Option<StoredPlan>,
    pub tomorrow_plan: Option<StoredPlan>,
    pub latest_weight: Option<WeightPoint>,
}
```

## 8. Diet plugin (`lifeagent-plugin-diet`)

Plugin id `diet`. Event types: `diet.meal_logged`.

### 8.1 Model

```rust
pub struct Macros { pub kcal: f64, pub protein_g: f64, pub carbs_g: f64, pub fat_g: f64, pub fiber_g: f64 }
// Macros supports Add, scaling by grams/100, rounding for display (kcal to 1, grams to 0.1).

pub enum MealType { Breakfast, Lunch, Dinner, Snack }
pub enum NutritionSource { User, LlmEstimate }

pub struct MealItem {
    pub name: String,            // canonical food name
    pub grams: f64,
    pub food_id: String,
    pub source: NutritionSource,
    pub macros: Macros,          // for this portion
}

pub struct MealLogged {          // payload of diet.meal_logged
    pub meal_type: MealType,
    pub raw_text: String,
    pub items: Vec<MealItem>,
    pub total: Macros,
}
```

Totals are computed at logging time and stored, so later food corrections do not silently
rewrite history.

### 8.2 Food table and resolver

`normalize(name)`: trim, lowercase ASCII, collapse whitespace, remove trailing punctuation.

`FoodResolver::resolve(name) -> Food`:
1. Look up `name_key = normalize(name)` or any alias key. Hit → increment `use_count`, return.
2. Miss → estimate per-100 g nutrition with the parser model (§9.3), validate (§8.3),
   insert with `source = 'llm_estimate'`, return.

Resolving a list of names runs misses concurrently (max 4 in flight) but inserts in order.

### 8.3 Nutrition sanity check (per 100 g)

**Convention: `carbs_g` is total carbohydrate, fiber included** (matches standard food
composition tables, so it matches what the parser/estimator prompts (§9.2/§9.3) produce by
default) — every rule below follows from this; fiber is a subset of carbs, not counted
separately (see B2 in `docs/DECISIONS.md`).

- All values ≥ 0; `fiber ≤ carbs`; `protein + carbs + fat ≤ 100`; `kcal ≤ 900`.
- Computed energy `e = 4·protein + 4·(carbs − fiber) + 9·fat + 2·fiber` (the non-fiber part of
  carbs at 4 kcal/g, fiber itself at 2 kcal/g — not double-counted). If
  `|kcal − e| > max(20, 0.15·e)`, replace `kcal` with `e` (trust the macros) and log at DEBUG.
- Failure of the first two rules → one re-ask with the error; still invalid →
  `PluginError::Other("无法估算该食物营养")`.

### 8.4 Meal parser

`parse_meal(description, hint_meal_type, eaten_at) -> ParsedMeal` with the parser model (§9.2).

```rust
pub struct ParsedMeal { pub meal_type: MealType, pub items: Vec<ParsedItem> }
pub struct ParsedItem { pub name: String, pub grams: f64 }
```

The prompt includes up to 150 known food names (highest `use_count` first) so the model reuses
canonical names and hits the local table. Validation: 1–20 items, each grams in 1–2000.
Composite dishes are decomposed into main components (e.g. "牛肉拉面" → noodles, beef, broth,
vegetables with grams) unless the dish already exists in the food table under that name.

Default meal type when neither the user nor the parser gives one, by local time:
05:00–10:29 breakfast, 10:30–14:29 lunch, 14:30–16:59 snack, 17:00–21:29 dinner, else snack.

### 8.5 Tools

| Tool | Args | Behavior / returned content |
|---|---|---|
| `diet_log_meal` | `description: string` (required, user's words), `meal_type?: enum`, `eaten_at?: string` (local ISO datetime; default now) | Parse → resolve → append `diet.meal_logged` (source `chat`). Returns `{ entry_id, meal_type, items:[{name, grams, kcal, protein_g, estimated: bool}], total, day_total, day_target, remaining }` |
| `diet_get_intake` | `date?: string` | `{ date, total, target, remaining, meals:[{entry_id, meal_type, time, items, total}] }` |
| `diet_update_food` | `food_name: string`, `kcal`, `protein_g`, `carbs_g`, `fat_g`, `fiber_g?` (per 100 g) | Upserts the food with `source='user'`. Affects future entries only; says so in the result |
| `diet_add_alias` | `food_name: string`, `alias: string` | Adds an alias key to an existing food |

Deleting a meal uses `core_delete_entry`. Tool descriptions tell the model: always pass the
user's original wording in `description`; when the user corrects a portion, delete the entry
and log it again.

### 8.6 Targets engine (pure functions, fully unit tested)

Inputs: profile, latest weight `w` (kg), age `a`, active weight goal.

```
BMR  = 10·w + 6.25·height_cm − 5·a + 5      (male)
BMR  = 10·w + 6.25·height_cm − 5·a − 161    (female)
TDEE = BMR × factor   Sedentary 1.2, Light 1.375, Moderate 1.55, Active 1.725, VeryActive 1.9

rate = min(goal.rate_kg_per_week, 0.01·w)                  (cap 1% body weight / week)
effective_direction = goal.direction
If Lose and target_weight_kg is set and w ≤ target: effective_direction = Maintain.
If Gain and target_weight_kg is set and w ≥ target: effective_direction = Maintain.
(effective_direction, not goal.direction, drives delta and the macro split below)

Lose:     delta = −min(rate·7700/7, 0.25·TDEE)
Gain:     delta = +min(rate·7700/7, 500)
Maintain: delta = 0

floor = max(BMR, 1500 if male else 1200)
kcal  = round_up_to_10(max(TDEE + delta, floor))
(round UP to the next multiple of 10, never down — rounding down could push kcal below floor)

anchor_w  = target_weight_kg.unwrap_or(w)                  (goal weight when set, else current;
 Lose/Gain goals normally always have one, since §7.4 stores a ∓1 kg default at save time)
protein_g = max(anchor_w × (1.8 Lose, 1.6 Gain, 1.2 Maintain), 0.8·w)
(the 0.8·w term is an RDA-level safety floor on the CURRENT body; it only binds when the goal
 weight is far below current weight — see B1 in docs/DECISIONS.md for why protein is no longer
 anchored to current weight alone)
fat_g     = max(0.27·kcal/9, 0.6·w)
carbs_g   = (kcal − 4·protein_g − 9·fat_g) / 4

if carbs_g < 100: fat_g = max(0.6·w, (kcal − 4·protein_g − 400)/9); recompute carbs_g
if carbs_g < 100:
  (still infeasible: protein_g + the 0.6·w fat floor + a 100 g carb minimum exceed the kcal
   computed above — raise kcal to fit all three rather than shrink any of them further)
  kcal          = round_up_to_10(4·protein_g + 9·(0.6·w) + 400)
  fat_g         = 0.6·w
  carbs_g       = 100
  floor_applied = true

fiber_g   = 14·kcal/1000
```

Output `NutritionTargets { kcal, protein_g, carbs_g, fat_g, fiber_g, bmr, tdee, delta, floor_applied: bool }`.
`floor_applied` is `true` whenever `kcal` was raised above `round_up_to_10(TDEE + delta)` by
either safety floor above — the absolute `max(BMR, 1500/1200)` floor, or the macro-feasibility
floor — so the UI can show one "计划已按最低安全值调整" note regardless of which one fired.
If the profile or weight is missing, tools still work but targets are `null` and the plan
input returns `None` with a warning logged.

### 8.7 Context snippet (today)

Plain text, e.g.:
```
今日摄入：1320 / 2050 kcal；蛋白质 72 / 150 g；碳水 160 / 210 g；脂肪 41 / 62 g
已记录：早餐(entry 0192…) 燕麦牛奶 420 kcal；午餐(entry 0192…) 牛肉拉面 900 kcal
今日计划中尚未吃的餐：晚餐 鸡胸肉糙米饭
```

### 8.8 Today card

```json
{
  "date": "2026-09-23",
  "total": { "kcal": 1320, "protein_g": 72, "carbs_g": 160, "fat_g": 41, "fiber_g": 12 },
  "target": { "...": "same shape or null" },
  "meals": [ { "entry_id": "...", "meal_type": "lunch", "time": "12:40",
               "items": [ { "name": "...", "grams": 250, "kcal": 400, "estimated": true } ],
               "total": { "...": "..." } } ]
}
```

### 8.9 Plan input

For date `D`:
- `targets`: `NutritionTargets` for `D` (computed with the latest weight).
- `brief` includes: meals per day, dietary pattern, allergies (as hard constraints), dislikes,
  memory notes relevant to food (all notes are passed by core anyway), the last 3 days of
  plan-vs-actual kcal and protein (`D−3..D−1` where data exists), and the list of dish titles
  planned in the last 5 days (to encourage variety).
- `output_schema`:

```json
{
  "type": "object",
  "required": ["meals"],
  "properties": {
    "meals": { "type": "array", "items": {
      "type": "object",
      "required": ["meal_type", "title", "items"],
      "properties": {
        "meal_type": { "enum": ["breakfast", "lunch", "dinner", "snack"] },
        "title": { "type": "string" },
        "items": { "type": "array", "items": {
          "type": "object", "required": ["name", "grams"],
          "properties": { "name": { "type": "string" }, "grams": { "type": "number" } } } },
        "prep_note": { "type": "string" }
      } } },
    "tips": { "type": "array", "items": { "type": "string" } }
  }
}
```

### 8.10 Finalize and validate plan section

1. Deserialize; on failure → violation "invalid structure: …".
2. Resolve every item through `FoodResolver`; compute per-meal and day totals.
3. Violations (trigger retry):
   - day kcal outside target ± 8%
   - protein < 90% of target
   - number of main meals (breakfast/lunch/dinner) ≠ 3, or total meals outside 3..=`meals_per_day`+1
   - any item name contains an allergy term (case-insensitive substring, both directions)
   - any item grams outside 1..=1500
4. Warnings (no retry): contains a disliked term; fat or carbs outside target ± 20%;
   fiber < 70% of target. Items whose nutrition comes from an LLM estimate are not warnings;
   they are marked `estimated: true` per item instead.
5. Output `data`:

```json
{
  "targets": { "kcal": 2050, "protein_g": 150, "carbs_g": 210, "fat_g": 62, "fiber_g": 29 },
  "total":   { "kcal": 2010, "...": "..." },
  "meals": [ { "meal_type": "breakfast", "title": "...", "prep_note": "...",
               "items": [ { "name": "...", "grams": 60, "kcal": 230, "protein_g": 8, "estimated": false } ],
               "total": { "...": "..." } } ],
  "tips": [ "..." ]
}
```

### 8.11 Plan-vs-actual

`adherence(date) -> Option<{ planned_kcal, actual_kcal, planned_protein, actual_protein }>`:
current plan version for that date vs logged totals. Used in the plan brief and returned by
`diet_get_intake` for past dates.

## 9. Prompts

Prompts live in `prompts.rs` files as `const` strings with `{placeholders}` filled by simple
string replacement. All prompts are English; model output shown to the user follows the
user's language (zh-CN by default).

### 9.1 Conversation system prompt (core)

```
You are LifeAgent, a personal assistant running on the user's computer. You help the user
track daily life and follow plans that move them toward their goals.

Rules:
- Record things the user reports (meals, weight, ...) by calling the matching tool. Do not
  just acknowledge without recording.
- Never invent or compute nutrition numbers yourself. Numbers come from tool results; quote
  them as given.
- If a report is ambiguous in a way that changes the numbers a lot (e.g. "ate some rice"),
  log it with a reasonable typical portion and tell the user which portion you assumed.
- When the user corrects a logged entry, delete it with core_delete_entry and log it again.
- Save durable preferences with core_remember; do not save one-off events.
- To change a plan, call core_revise_plan with the user's request as the instruction.
- Be brief and warm. Reply in the language the user writes in (default Simplified Chinese).
- You give lifestyle guidance, not medical advice. If the user describes symptoms or a medical
  condition, suggest consulting a professional.

Current time: {now_local} ({weekday}). Logical date: {logical_date}.

## User profile
{profile_summary}

## Goals
{goals}

## Today's status
{plugin_snippets}

## Plans
{plans}

## Memory notes
{memory_notes}
```

### 9.2 Meal parser (diet, role Parser, JSON)

```
Extract the foods from the user's meal description and estimate the edible weight in grams
of each item as eaten.

Return JSON: {"meal_type": "breakfast|lunch|dinner|snack|null", "items": [{"name": string, "grams": number}]}

Rules:
- Use a name from KNOWN_FOODS when it refers to the same food. Otherwise use a short,
  specific, generic food name in the user's language (e.g. "熟米饭", "卤蛋", "牛肉(熟)").
- Split composite dishes into main components with separate grams, unless the whole dish
  appears in KNOWN_FOODS.
- Include cooking oil for fried or stir-fried dishes as a separate item ("食用油") with a
  typical amount.
- Use typical portions when not stated. Counts ("两个鸡蛋") become grams.
- Drinks with calories count as items. Plain water, black coffee and tea without sugar are
  omitted.
- meal_type only if the text states or clearly implies it; otherwise null.

KNOWN_FOODS: {known_foods}
Meal time: {eaten_at_local}
Description: {description}
```

### 9.3 Food estimate (diet, role Parser, JSON)

```
Give typical nutrition values per 100 g of the edible portion of this food, as eaten
(cooked if it is normally eaten cooked). Use values consistent with standard food
composition tables.

`carbs_g` is total carbohydrate, fiber included — the same convention standard food composition
tables use. Do not subtract fiber from it.

Return JSON: {"kcal": number, "protein_g": number, "carbs_g": number, "fat_g": number, "fiber_g": number}

Food: {name}
```

### 9.4 Planner (core, role Planner, JSON)

```
You create tomorrow's plan for the user. The plan has one section per domain below. Each
section must satisfy its targets and constraints and match its output schema exactly.

General rules:
- Propose realistic, easy-to-prepare meals that fit the user's habits, preferences and memory
  notes. Prefer common foods available in ordinary supermarkets and canteens.
- Use generic food names in the user's language, with grams for every item as eaten.
- Hit the numeric targets; the app recomputes nutrition with its own food table and will
  reject plans that miss them.
- Allergies are absolute. Avoid disliked foods.
- Vary dishes compared with recent days.
- "summary": 2–4 sentences to the user in {user_language}: the plan's focus for tomorrow,
  referencing recent adherence if helpful. Encouraging, never guilt-tripping.

Return JSON: {"summary": string, "sections": {<section id>: <section object>}}

Date: {plan_date} ({weekday})

## User profile
{profile_summary}

## Memory notes
{memory_notes}

{for each section:}
## Section "{section}"
Targets: {targets_json}
Brief:
{brief}
Output schema: {output_schema_json}

{if revision:}
## Current plan (version {version})
{previous_content_json}
## User's change request
{instruction}
Keep everything not affected by the request as close to the current plan as possible.
```

## 10. Desktop app (`apps/desktop`)

### 10.1 Rust side (`src-tauri`)

- Composition root: build `CoreApp` with `SystemClock`, `KeyringSecretStore`, and the diet
  plugin; store it in Tauri managed state; start the scheduler.
- Forward `AppEvent`s to the webview as Tauri events: `data-changed`, `plan-ready`,
  `plan-failed`.
- On `PlanReady` (trigger `schedule`) and `PlanFailed`, show a Windows notification if
  `notifications_enabled`. Title "明日计划已生成" / "明日计划生成失败"; body = first sentence of
  the summary / short error. Clicking focuses the main window on the Today page.
- Tray icon with menu: 打开, 快速记录, 生成明日计划, 退出. Closing the main window hides it
  to the tray; only 退出 ends the process (so the scheduler keeps running).
- Single instance (`tauri-plugin-single-instance`): a second launch focuses the window.
- Autostart via `tauri-plugin-autostart`, toggled by the `autostart` setting.
- Global quick-entry window: small always-on-top window with one text box; submitting sends
  the text through `send_message` and shows the reply briefly. Opened from the tray menu.
  (A global hotkey is optional; skip if it adds friction.)

### 10.2 Tauri commands

All commands return `Result<T, CommandError>` where `CommandError { code: String, message: String }`
(message already in zh-CN). Codes: `not_configured`, `auth`, `llm`, `invalid_input`,
`not_found`, `internal`.

| Command | Input | Output |
|---|---|---|
| `get_app_state` | — | `{ onboarding_complete, models_configured, logical_date }` |
| `send_message` | `{ text }` | `ChatTurn` |
| `get_history` | `{ limit }` | user/assistant messages only (tool messages hidden), with activities |
| `get_today` | — | `TodayView` |
| `get_plan` | `{ date }` | `StoredPlan \| null` |
| `generate_plan` | `{ date? }` | `StoredPlan` |
| `accept_plan` | `{ date }` | — |
| `delete_entry` | `{ id }` | — |
| `get_profile` / `save_profile` | `Profile` | |
| `get_weight_goal` / `save_weight_goal` | `WeightGoal` | |
| `log_weight` | `{ kg }` | — |
| `preview_targets` | `{ profile, weight_kg, goal }` | `NutritionTargets` (pure computation, nothing saved; the desktop crate calls `lifeagent_plugin_diet::targets::compute` directly, since it already depends on the plugin) |
| `get_settings` / `save_settings` | settings object (all keys of §7.2 except `models`) | |
| `get_models` / `save_models` | `ModelsConfig` | |
| `set_api_key` | `{ provider_id, key }` | — (never returns keys) |
| `has_api_key` | `{ provider_id }` | `bool` |
| `test_model` | `{ role }` | `{ ok, latency_ms, error? }` — sends "Reply with OK." |
| `get_memory` / `delete_memory` | | |
| `get_usage_summary` | — | tokens per role for the current month |
| `complete_onboarding` | — | — |

### 10.3 UI

Thin React app; all state comes from commands; refetch on `data-changed` / `plan-ready`.
Layout: left sidebar (今日 / 对话 / 设置), content area. Tailwind, clean and calm; supports
light and dark following the system.

**Onboarding (first run, blocking)** — three steps:
1. 基本信息: sex, birth date, height, current weight, activity level (with one-line
   explanations), dietary pattern, allergies, dislikes, meals per day, medical notes (optional).
2. 目标: lose / maintain / gain, target weight (optional), rate (0.25 / 0.5 / 0.75 kg per week
   choices, default 0.5 for lose, 0.25 for gain). Show the computed daily kcal target
   immediately (via `preview_targets`) so the user sees the effect.
3. 模型: provider kind (OpenAI 兼容 / Anthropic), provider name, base URL, API key, model for
   each role (one model can be used for all three), a 测试连接 button that must succeed before
   finishing. Choosing Anthropic prefills the base URL `https://api.anthropic.com` (no `/v1`
   suffix) and suggests `claude-opus-5` for planner, `claude-sonnet-5` for conversation and
   `claude-haiku-4-5` for parser.

**Today page**
- Header: logical date, latest weight with a small "记录体重" button.
- Diet card: kcal ring (eaten / target), protein/carbs/fat bars, list of today's meals
  (time, meal type, items, kcal; items with `estimated` show a subtle "估" badge), delete
  button per meal with confirm.
- Today's plan: meals with ✓ marks when a logged meal of the same type exists (simple heuristic).
- Tomorrow's plan: summary, meals with items and grams, day totals vs targets, warnings in a
  muted callout, disclaimer if present, buttons 接受计划 / 重新生成 / 调整 (opens Chat
  with "调整明天的计划：" prefilled). If no plan yet: "将于 21:00 自动生成" + 立即生成 button.

**Chat page**
- Message list; assistant messages show activity chips (e.g. "已记录午餐 · 780 kcal").
- Input box, Enter sends, Shift+Enter newline; disabled while a turn is running with a
  typing indicator.
- Errors show inline with a retry button; `auth`/`not_configured` link to Settings.

**Settings page**
- 个人档案 and 目标 (same forms as onboarding).
- 模型: providers, keys (masked, replace only), role bindings, test buttons, this month's
  token usage per role.
- 计划: plan time, day boundary.
- 通用: notifications, autostart.
- 记忆: list of memory notes with delete.

## 11. CLI (`lifeagent-cli`)

`clap` subcommands, global `--data-dir`, `--mock` (uses a `MockChatModel` that echoes; for
smoke-testing plumbing only):

```
lifeagent-cli chat                      # interactive REPL; prints reply and activities
lifeagent-cli say "<text>"              # one turn
lifeagent-cli today                     # prints TodayView as pretty JSON
lifeagent-cli plan generate [--date D]
lifeagent-cli plan show [--date D]
lifeagent-cli plan revise "<instruction>" [--date D]
lifeagent-cli foods list [--limit N]
lifeagent-cli profile show | profile set-json <file>
lifeagent-cli models set-json <file> | models set-key <provider_id>   # key read from stdin, hidden
lifeagent-cli targets                   # prints NutritionTargets for today
```

## 12. Errors, logging, security

- Logs: `tracing-appender` daily rolling file in `data_dir/logs`, keep 14 files; INFO default,
  `LIFEAGENT_LOG` env var overrides the filter.
- Log each LLM call at INFO with role, model, tokens, latency, ok — never content.
  Prompts/responses may be logged at DEBUG.
- API keys only in the OS credential store; never in DB, logs, or command outputs.
- What is sent to cloud models: the system prompt (profile summary, goals, summaries, memory
  notes), recent conversation, tool results, and planner inputs. Document this in the
  Settings page with one short paragraph.
- Database opened with WAL mode and `foreign_keys=ON`; a daily backup copy of the DB file
  (`lifeagent.db.bak`, keep last 7 dated copies) taken by the scheduler at first tick after
  the day boundary.

## 13. Testing strategy

All tests offline and deterministic (`FixedClock`, `MemorySecretStore`, `MockChatModel`,
temp directory DB).

Required unit tests:
- `logical_date` around the boundary, including DST-free and midnight cases.
- Scheduler "is it time" logic for several plan_time / day_boundary combinations, including a
  relaunch shortly after midnight with plan_time already passed the previous evening (B3 in
  `docs/DECISIONS.md`) — the case that broke under the old "today's calendar date" wording.
- Settings validation rejects `plan_time < day_boundary`.
- Targets engine: male/female, each goal direction, rate cap, floor application, carbs
  minimum adjustment, goal reached → maintain, goal-weight anchor (target set vs. unset).
  Include at least one hand-computed example, plus a heavy-body-weight case that would have
  produced zero/negative carbs under the old current-weight-only protein anchor (B1 in
  `docs/DECISIONS.md`), asserting `carbs_g > 0` and, separately, a case extreme enough to
  trigger the macro-feasibility `floor_applied` path.
- Goal save default target (§7.4): Lose/Gain without a target stores latest weight ∓ 1 kg;
  clamped at 30/300 kg; an explicit target is kept as given; no logged weight → stays unset.
- Nutrition sanity check and kcal replacement, including a high-fiber food (e.g. wheat bran:
  protein 15.5, carbs 64.5, fat 4.3, fiber 42.8 — passes under the fiber-included-in-carbs
  convention) and a `fiber > carbs` input that is rejected.
- Food resolver: hit by name, hit by alias, miss → estimate → cached (second call makes no LLM
  request).
- Meal parser validation and default meal type by time.
- Plan validator: each violation and warning rule.
- Tool registry: prefix/duplicate rules, argument validation.
- JSON helper: fenced JSON, repair retry success, repair failure.
- OpenAI-compatible and Anthropic providers: request mapping and response parsing using
  recorded JSON fixtures (no network; test the (de)serialization functions directly). For
  Anthropic, cover specifically: system messages hoisted out of `messages` into the top-level
  `system` field, and consecutive tool messages collapsed into one user message carrying
  several `tool_result` blocks.

Required integration tests (in `core/tests` and `plugin-diet/tests`):
- Orchestrator: scripted model calls `diet_log_meal` then answers; assert event stored,
  tool message stored, reply returned, activity summary present.
- Orchestrator: invalid tool args → error returned to model → model retries correctly.
- Orchestrator: loop cap at 6 iterations.
- Plan generation: scripted planner output that violates kcal → retry with violation message
  (assert the second request contains it) → valid plan stored as version 1.
- Plan revision creates version 2 with trigger `revision` and instruction stored.
- Scheduler catch-up: FixedClock after plan_time with no plan → generation happens once,
  including the 01:00-relaunch case from §7.13 (calendar date has advanced, logical day hasn't)
  and a case confirming today's own missing plan is *not* backfilled (catch-up only targets
  tomorrow).

## 14. Milestones

**M0 — Workspace skeleton.** Workspace, crates, Tauri app scaffold, shared dependencies,
`tracing` setup in apps, empty `docs/DECISIONS.md`.
Done when: `cargo build --workspace` and `npm run tauri dev` start; clippy clean.

**M1 — Foundations.** Clock, logical date, data dir, DB open + core migrations, settings,
secrets, profile/goal services with validation, event store with soft delete and bus,
memory notes, conversation store.
Done when: unit tests for time, settings, profile validation and event store pass.

**M2 — LLM layer.** Types, both providers (OpenAI-compatible and Anthropic) with retries,
router with role bindings and usage sink, JSON helper, mock.
Done when: both provider mapping test suites pass with fixtures; JSON helper tests pass.

**M3 — Plugin API and orchestrator.** Plugin trait, plugin migrations, tool registry with
argument validation, core tools (except plan tools, stubbed), context builder, orchestrator loop,
`CoreApp` builder and `send_message`.
Done when: orchestrator integration tests pass with a tiny test plugin.

**M4 — Diet plugin: logging.** Food table, resolver, sanity check, parser, tools, intake,
targets engine, context snippet, today card, plan-vs-actual.
Done when: diet unit tests pass; an integration test logs a meal via the orchestrator and
`today_view` shows correct totals.

**M5 — Planning.** Plan service (generate, retry, store, revise, accept), diet plan input
and finalize/validate, plan core tools, scheduler, daily DB backup.
Done when: planning and scheduler integration tests pass.

**M6 — CLI.** All subcommands in §11.
Done when: with a real API key, `say "早餐吃了两个鸡蛋和一杯牛奶"` logs a meal and
`plan generate` produces a valid plan (manual check, noted in DECISIONS.md).

**M7 — Desktop.** Commands, events, onboarding, Today, Chat, Settings, tray, close-to-tray,
notifications, single instance, autostart, quick-entry window.
Done when: the acceptance scenario in §15 passes manually.

**M8 — Hardening.** Error messages in zh-CN for every `CommandError`, empty states, loading
states, log review for secrets, README with setup steps.
Done when: fmt/clippy/tests clean; README complete.

## 15. Acceptance scenario (manual, Phase 1 exit)

1. Fresh install → onboarding: enter profile and goal (lose 0.5 kg/week); the kcal target
   preview appears; configure a provider of either kind; connection test succeeds.
2. Chat: "早上吃了一碗燕麦牛奶，一根香蕉" → breakfast logged with items and grams; the
   reply quotes totals and remaining kcal; Today page updates without manual refresh.
3. Chat: "中午吃了牛肉拉面，加了个卤蛋" → lunch logged, noodle dish decomposed into components.
4. Chat: "卤蛋其实是两个" → entry deleted and re-logged with two eggs.
5. Chat: "我不吃香菜，记住" → memory note created and visible in Settings.
6. Today page → 立即生成 → tomorrow's plan appears within targets (± 8% kcal), no cilantro,
   warnings (if any) shown.
7. Chat: "明天中午公司聚餐，吃不了计划里的午餐，帮我调整" → plan version 2 with an adjusted
   lunch approach and rebalanced other meals.
8. Close the window → app stays in tray. On a day with no plan for tomorrow yet, set
   `plan_time` to one minute from now; the plan is generated and a notification arrives.
9. Quit, change system time past plan_time on a new day, relaunch → catch-up generation runs.
10. Logs contain no API key and no meal text at INFO level.

## 16. Defaults and decisions already made

- One conversation thread; no streaming; tool calls executed sequentially.
- Single active weight goal; exercise is not tracked yet, so activity level alone drives TDEE.
- Food nutrition from a local table seeded only by LLM estimates and user corrections;
  corrected values (`source='user'`) always win and are never overwritten by estimates.
- A logged meal's numbers never change after logging.
- Plans are proposals; "accept" only marks status (used later for adherence emphasis).
- If anything here proves impractical during implementation, choose the simplest compliant
  alternative and record it in `docs/DECISIONS.md`.
