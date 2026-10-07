# Agent activity log — specification

Status: implemented (2026-10-07), milestones AL0–AL4 (§10). Where the implementation refined
this design, the text below says so; see also `docs/DECISIONS.md`, "Agent activity log". Gap B in the
2026-10-07 hackathon review; see `docs/HACKATHON.md` §3, "Audit trail UI".

This document follows the conventions of `docs/PHASE1_SPEC.md`: it is the source of truth for
the feature's behavior, `CLAUDE.md` wins for conventions, and milestones are implemented in
order, each closed only when its "done when" criteria hold.

Decisions taken with the user (2026-10-07):

1. The Activity page is a top-level tab. Its visibility is a switch in Settings. Hiding the
   tab changes nothing else: runs are still recorded, and the API and Cloud Logging copy still
   work (§6.3).
2. Tool summaries become localized. They leave Rust as data and the UI words them, like
   approval previews (§3.6). This is milestone AL0. If it overruns, the fallback is to make
   the Rust summaries plain English (§3.6, "Fallback").
3. Runs are kept for 90 days (§4.5).

## 1. Purpose and scope

The judging asks whether the agent's autonomy is "well managed and controlled, with attention
to observability and security". Today the controls exist (tool argument validation, approval
for sensitive calls, the plan validator, the token budget, per-user database roles), but
nothing shows them working. Tool calls survive only as chat rows in `messages`, a plan made by
the scheduler leaves no trace of how it was made, and `llm_usage` rows can't be tied to what
caused them.

The activity log records **everything the agent does on the user's behalf**: who or what
started it, each model call, each tool call with its arguments and outcome, each guardrail
that stepped in, and how it ended. The user sees it on an Activity page, and the operator
gets the same trail in Cloud Logging.

### In scope

- A run for each agent entry point: chat turn, scheduled plan, manual plan, plan revision,
  approving or rejecting a pending action. Later, proactive follow-ups (gap A) reuse it.
- Steps inside a run: model calls (role, model, tokens, latency), tool calls (arguments,
  outcome), approval queued or resolved, guardrail blocks, plan validation results.
- An Activity page in the web UI, in all three locales, updated live. It is a top-level tab
  that Settings can hide without turning off recording.
- Tool summaries as data, worded by the UI in each locale. This also fixes the chat's activity
  chips, which today show Chinese (English for members) in every locale.
- One structured log line per step, for Cloud Logging (`docs/DECISIONS.md`, "Structured logs
  for Cloud Logging").

### Out of scope

- The user's own edits in the UI (forms on Settings, Members, Stock, …). They are not agent
  actions, and the events they create already carry `source = 'ui'`.
- Prompts and model responses. The chat already keeps its messages, and planner prompts hold
  personal data that we don't want to copy. The log records what the agent *did*, not what it
  was told.
- Side effects run by event hooks (e.g. stock used up by a logged meal). They run on the
  dispatcher's own task, outside the run (§3.2). The tool call that logged the meal is
  recorded.
- The server's per-IP and per-user rate limits. They refuse a request before any run starts,
  and they already log a WARNING line.
- Tamper-proofing against the app itself. See §8.

## 2. Terms

| Term | Meaning |
|---|---|
| Run | One unit of agent work with one trigger, for example one `send_message` call. It has a start, an end and a status. |
| Step | One thing that happened inside a run, in order. |
| Actor | Who started the run: `user` (a message or button) or `scheduler` (Cloud Scheduler's tick). |
| Trigger | What kind of run it is (§4.1). |
| Guardrail | A control that changed what the agent did: unknown tool, invalid arguments, approval required, token budget, plan validator. |

## 3. Architecture

### 3.1 Where it lives

A new core module, `crates/core/src/audit.rs`, with an `AuditLog` store next to
`ApprovalStore`, reachable as `CoreApp::audit()`. Plugins don't depend on it. Tool calls are
recorded by core around `ToolRegistry::call`, so every plugin is covered without changes.

### 3.2 Attaching steps to the current run

Model calls happen deep inside plugins (the diet parser, the food matcher) and inside the
planner. Threading a run handle through every signature would touch every plugin. Instead:

- `audit::scope(run, future)` runs a future with the run in a `tokio::task_local!`
  (`CURRENT_RUN`). Tokio is already a dependency, so nothing new is added.
- `DbUsageSink::record`, already called by the router for every model call, also appends a
  `model_call` step when `CURRENT_RUN` is set. The `llm` crate is unchanged.
- The tool loop, the approval flow and the planner append their steps through the same
  task-local.
- With no current run set (event hooks, CLI commands that don't open one, tests), steps are
  silently not recorded. That keeps the hook and CLI paths working unchanged.

Plan generation, revision and the chat loop all run on the caller's task (none of them spawn),
so the task-local reaches every model call. If a chat turn calls `core_generate_plan`, the
planner's steps belong to that chat run, which is correct.

### 3.3 Run lifecycle

```
CoreApp::send_message ─┐
CoreApp::generate_plan ┼─ audit.start(trigger, actor, input) ──> run (status running)
CoreApp::revise_plan   │      audit::scope(run, body).await
approve / reject       ┘      audit.finish(run, status, outcome)
```

`start` and `finish` are wrapped in a single helper,
`CoreApp::with_run(trigger, actor, input, |run| async { ... })`, so no entry point can forget
to finish its run. If the body returns an error, the run finishes as `failed` with the error's
`CoreError::code()`.

The actor comes from the entry point. Chat, revisions and approvals are `user`. For
`generate_plan` it is derived from `PlanTrigger` (`Schedule` → `scheduler`, otherwise
`user`); the planned extra `Actor` argument turned out to be unnecessary, because the two
always coincide today. Proactive follow-ups will open their own scheduler runs.

### 3.4 When a write fails

- `start` fails → **fail closed**. The entry point returns the error and the agent does
  nothing. In practice the database is down, so the action would fail anyway.
- A step or `finish` fails → **fail open**. An ERROR log line goes out, and the Cloud Logging
  copy of the step (§7) is still written. Losing one audit row is better than half-applying
  a tool call. This matches `DbUsageSink::record`.

### 3.6 Tool summaries as data (AL0)

**Today:** `ToolOutput::summary` is a `String` composed in Rust. Of the 29 tools' summaries,
most are hard-coded Chinese (`已记录午餐 · 376 kcal`) and the multimember ones are English.
The string reaches three places:
- the chat's activity chips (`ChatTurn::activities`)
- `pending_actions.result_summary`, shown when an approved action fails, and quoted in the
  system prompt's "recent actions" section
- the CLI

A Japanese or English user sees Chinese chips.

**Change:** the summary becomes data, in the same shape as approval previews:

```rust
/// What a tool call did, as data each UI words in its own language: `{"kind": ..., ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSummary(pub serde_json::Value);

impl ToolSummary {
    pub fn new(kind: &str) -> Self;                         // {"kind": kind}
    pub fn with(self, key: &str, value: impl Serialize) -> Self;
}

pub struct ToolOutput {
    pub content: Value,
    pub summary: ToolSummary,
}
```

- **Kinds.** Kinds are `snake_case` and prefixed by the owning plugin's id, like tool names,
  so plugins can't collide. Examples: `diet_meal_logged {meal, kcal}`,
  `diet_intake_viewed {date}`, `core_plan_generated {date, version}`, `stock_added {names}`,
  `multimember_attendance_set {name, meal, date, attending}`. Read-only tools may share a kind
  per plugin (`*_viewed`). Numbers in params are the ones Rust computed, as everywhere else.
- **Wording.** In `apps/web/src/i18n/`, a new `toolSummaries` table maps each kind to a
  function of its params, in `zh-CN.ts` (which defines the keys), `ja.ts` and `en.ts`. An
  unknown kind falls back to the tool's label from the `tools` table (§6.2), and then to the
  raw tool name. So a plugin added later without translations still renders.
- **Users of the summary:**
  - `ChatTurn::activities` becomes `Vec<ToolSummary>`, and `Chat.tsx` words each chip.
  - `pending_actions` gets a `result_summary_json` column, holding the summary of an approved
    action. `result_summary` stays for failure reasons, which are tool error text and not
    part of this change.
  - The system prompt's "recent actions" quotes the summary JSON. The model reads JSON as
    well as prose, and this removes the only reason Rust would need its own wording.
  - The activity log's `tool_call` steps store it (§4.2).
  - The CLI prints `kind key=value …`. It is a developer tool and has no locale tables.
- **Untrusted text in params** (food, dish and member names) goes through the same React
  escaping as everything else. When quoted in the system prompt, it is inside the existing
  fenced block.

**Fallback.** If AL0 can't be finished in time, keep `summary: String` and rewrite every
summary as English in Rust. That touches only the five `tools.rs` files and the tests that
assert summaries. AL1–AL4 don't depend on which of the two lands: a step stores whatever
`ToolOutput::summary` holds.

## 4. Data model (core migration `0005_activity_log.sql`)

```sql
CREATE TABLE agent_runs (
  id             TEXT PRIMARY KEY,      -- UUID v7, so ordering by id is ordering by start
  trigger        TEXT NOT NULL,         -- §4.1
  actor          TEXT NOT NULL,         -- 'user' | 'scheduler'
  status         TEXT NOT NULL,         -- 'running' | 'succeeded' | 'needs_approval' | 'failed'
  input_excerpt  TEXT,                  -- §4.3
  outcome_json   TEXT,                  -- §4.4
  error_code     TEXT,                  -- CoreError::code() when failed
  logical_date   TEXT NOT NULL,
  started_at     TEXT NOT NULL,
  finished_at    TEXT
);
CREATE INDEX agent_runs_started_at ON agent_runs(started_at);

CREATE TABLE agent_steps (
  id           TEXT PRIMARY KEY,        -- UUID v7
  run_id       TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  seq          BIGINT GENERATED ALWAYS AS IDENTITY,  -- order within the run
  kind         TEXT NOT NULL,           -- §4.2
  detail_json  TEXT NOT NULL,           -- per kind, §4.2
  created_at   TEXT NOT NULL
);
CREATE INDEX agent_steps_run ON agent_steps(run_id, seq);

ALTER TABLE pending_actions ADD COLUMN run_id TEXT;  -- the run that queued it
```

Times follow the core conventions: UTC RFC 3339 strings from the injected `Clock`, plus a
`logical_date`.

A run that is still `running` and older than 15 minutes is reported as `interrupted` when
read, the same way `pending_actions` derives `expired`. That covers an instance killed in the
middle of a turn.

Likewise, a `needs_approval` run reads as `succeeded` once none of the actions it queued is
still pending (each was approved, rejected, failed or expired). The stored status is left as
written; the approval itself is its own `action_approved` / `action_rejected` run.

### 4.1 Triggers

| Trigger | Entry point | Actor |
|---|---|---|
| `chat_turn` | `send_message` | user |
| `plan_scheduled` | `Scheduler::plan_if_due` | scheduler |
| `plan_manual` | `generate_plan` with `PlanTrigger::Manual` | user |
| `plan_revision` | `revise_plan` | user |
| `action_approved` | `approve_action` | user |
| `action_rejected` | `reject_action` | user |
| *(gap A)* `follow_up` | proactive follow-ups | scheduler |

### 4.2 Step kinds

All `detail_json` fields are `snake_case`. Free text comes from tools and plugins, never from
the model directly, except the tool arguments, which are shown as data.

| Kind | `detail_json` |
|---|---|
| `model_call` | `{role, provider, model, prompt_tokens, completion_tokens, latency_ms, ok}` |
| `tool_call` | `{tool, arguments, outcome, summary?, error?}`. `summary` is the `ToolSummary` data (§3.6). `outcome`: `ran` \| `invalid_args` \| `not_found` \| `failed` |
| `approval_requested` | `{tool, action_id, arguments, preview}`. `preview` is the same `{kind, …}` data the approval card uses |
| `approval_resolved` | `{tool, action_id, decision, requested_in, result?}`. `decision`: `approved` \| `rejected` \| `failed`; `requested_in` is the run that queued it; `result` is the `ToolSummary` |
| `guardrail` | `{rule, …}`. `rule`: `unknown_tool` \| `invalid_args` (with `tool`) \| `token_budget` (with `role`, `used`, `budget`) \| `iteration_cap` (with `limit`) |
| `plan_validation` | `{date, sections, violations: [string], retried, violations_left}`, one per planning stage: the violations found first, whether they sent the planner back, and how many were left (they become warnings) |
| `plan_saved` | `{date, version, trigger}` |

A `tool_call` that was blocked records both the `tool_call` step (with its outcome) and a
`guardrail` step. That way the Activity page can count guardrail interventions without
parsing outcomes.

### 4.3 Input excerpt

- For `chat_turn`: the user's message, cut to 200 characters.
- For plans: the request or revision instruction, if any.
- For approvals: nothing (the step already names the action).

It's stored in the user's own schema, like the message itself.

### 4.4 Outcome

`outcome_json` is a small summary the list view shows without loading steps:
`{model_calls, tool_calls, tokens, latency_ms, guardrails, pending_actions, plan?: {date, version}}`.
It is computed from the run's steps in `finish`.

### 4.5 Retention

Runs older than 90 days are deleted, with their steps by cascade, once a day by the scheduler
tick. Deleting the account drops the schema and the log with it.

## 5. Recording points

| Where | Records |
|---|---|
| `CoreApp::send_message` | Runs as `chat_turn`. `MAX_ITERATIONS` reached → `guardrail{iteration_cap}`. |
| `CoreApp::run_or_queue` | Unknown tool → `tool_call{not_found}` + `guardrail{unknown_tool}`. Approval needed → `approval_requested`, and `pending_actions.run_id` is set. Ran → `tool_call{ran, summary}`. |
| `ToolRegistry::call` errors | Argument schema check → `tool_call{invalid_args}` + `guardrail{invalid_args}`. A plugin's `InvalidArgs`/`NotFound` → the same outcomes. Anything else → `tool_call{failed}`. |
| `approve_action` / `reject_action` | Own run. `approval_resolved`, plus `tool_call` for the call it runs. |
| `DbUsageSink::record` | `model_call`. |
| `DbUsageSink::check_budget` refusal | `guardrail{token_budget}`. The run then fails with code `budget_exceeded` (`LlmError::BudgetExceeded`). |
| `plan::plan_stage` | `plan_validation` after the first finalize (`retried: true` if violations triggered the retry), and its final state. |
| `PlanStore::insert` callers | `plan_saved`. |

Tool arguments are stored as the model sent them. The stored value is truncated at 4 KB and
marked `"truncated": true`, so a runaway call can't bloat the table.

## 6. API and UI

### 6.1 Commands (`lifeagent-api`, `crates/server/src/routes.rs`)

- `get_activity { before?: run_id, limit?: u32 (default 30, max 100), actor?, status? }`
  returns runs newest first, each with its `outcome`, plus `next_before` for paging. Paging by
  id works because the ids are UUID v7.
- `get_activity_run { id }` returns the run with its steps in order, and for each
  `approval_requested` step, the action's current status (pending, approved, rejected, failed
  or expired).
- `get_activity_summary { days?: u32 (default 7) }` returns counts for the header strip (§6.2).

The server adds no logic: three more match arms. The `AppEvent::ActivityRecorded { run_id }`
event (published on `finish`) reaches the browser over the existing SSE stream, and the page
refetches.

### 6.2 Activity page (`apps/web/src/activity/`)

A new **Activity** tab (アクティビティ / Activity / 活动), placed after Chat. It is a
top-level tab rather than a Settings section because it is the main thing to show for the
governance theme. On phones it goes in the bottom bar, where the optional plugin tabs are
often hidden anyway. It can be hidden (§6.3).

- **Header strip, last 7 days:**
  - Runs started by you and by the scheduler
  - Approvals: asked, approved, rejected
  - Guardrail interventions
  - Tokens used, and the share of the daily budget if one is set
- **Run list, grouped by logical date:**
  - Actor icon (person / clock)
  - Trigger label
  - Time
  - Status badge (succeeded, needs approval, failed, interrupted)
  - The input excerpt
  - Counts from `outcome`
  - Filters for actor and status
- **Run detail (expand in place):** a timeline of the run's steps:
  - Model calls as one compact line each (role, model, tokens, latency)
  - Tool calls with the localized tool name, the outcome, the localized summary (§3.6), and
    the arguments as collapsible JSON
  - Approval steps linking to their current status. A still-pending one has the same
    approve/reject buttons as Chat (`PendingActions.tsx`).
  - Guardrail steps highlighted

Wording follows the existing i18n rules:
- A new `activity` table in `ja.ts`, `en.ts` and `zh-CN.ts` holds triggers, statuses, step
  kinds and guardrail rules.
- A `tools` table maps each tool name to a label. An unknown name falls back to the raw name.
- Summaries are worded by `toolSummaries` (§3.6).

No business logic lives in the frontend. Counts and totals come from Rust.

### 6.3 Showing and hiding the tab

- **Setting.** A per-account core setting `activity_tab_visible` (bool, default `true`) in
  the `settings` table, read and written through `Settings` with the injected `Clock`, like
  the plugin switches.
- **API.**
  - `get_app_state` gains `activity_tab_visible`. The shell already loads it at start to
    decide which plugin tabs to show.
  - A new command `set_activity_tab_visible { visible: bool }` writes it.
- **UI.**
  - `FeaturesSection.tsx` gets a switch, "Show the Activity tab", below the plugin switches,
    with a note that the agent's actions are recorded either way.
  - `App.tsx`'s `isTabShown` hides the tab when the setting is off.
  - If the tab is open when it's hidden, the page moves to Today, as for a plugin switched off.
- **Hidden means only hidden.** Recording (§5), retention (§4.5), the `get_activity*`
  commands, the SSE event and the Cloud Logging copy (§7) all keep working. Turning the tab
  back on shows the complete history, including runs made while it was hidden.
- **It is a display preference, not a privacy control.** There is no switch to stop
  recording: an agent whose audit trail the agent's own user can switch off is not the story
  we want to tell. The Settings note says so in plain words.

## 7. Cloud Logging copy

Every step also writes one INFO line with target `lifeagent_core::audit`:
- `message = "agent step"`
- `run_id`, `trigger`, `actor`, `kind`
- for tool calls, `tool` and `outcome`
- for guardrail steps, `rule`

Arguments, excerpts, summaries and previews are left out: they hold personal data, and
`CLAUDE.md` keeps that off INFO. The server's `command` and `tick` spans add `user_id`. The
operator can then follow any user's agent activity in Logs Explorer, or build log-based
metrics, for example guardrail interventions per hour or scheduler runs that failed.

## 8. Security and privacy notes

- **Isolation.** The log lives in the user's own schema, owned by the user's own role. The
  isolation that protects meals also protects the log, and a prompt-injected tool call can't
  reach another user's log.
- **Append-only by design, not by enforcement.** There is no API to edit or delete runs or
  steps (except retention and account deletion). The tenant role owns its tables, so a
  database trigger could not stop it either. The out-of-band copy that the app can't rewrite
  is the Cloud Logging trail (§7). We say this plainly in the submission instead of claiming
  tamper-proof logs.
- **Content shown is data.** Tool arguments and excerpts are model or user text. React
  escapes them, and they are never fed back into a prompt. The system prompt's "recent
  actions" section keeps reading `pending_actions`, not this log.

## 9. Interaction with existing features

- `ChatTurn.activities` becomes `Vec<ToolSummary>` in AL0 (§3.6). It stays a field of the
  live reply rather than being read back from the run's steps.
- `usage_summary` (Settings) keeps reading `llm_usage`. The budget check stays on `llm_usage`,
  which remains the source of truth for tokens. `model_call` steps copy the numbers for display
  only.
- `pending_actions.run_id` lets the Activity page connect "the agent asked" (chat run) to
  "you decided" (approval run).
- The CLI gains `activity [--limit N]`, which prints runs and steps, for development.

## 10. Milestones

### AL0 — Tool summaries as data
§3.6: `ToolSummary`, every tool's summary in core and the four plugins, `ChatTurn.activities`,
`pending_actions.result_summary_json` (core migration `0004_tool_summaries.sql`), the
recent-actions prompt section, the CLI, `toolSummaries` and `tools` tables in three locales,
`Chat.tsx` and `PendingActions.tsx`.
**Done when:**
- Every tool's unit test asserts its summary's `kind` and params instead of a string.
- `zh-CN.ts` has a `toolSummaries` entry for every kind a tool can emit. Implemented as the
  `ToolSummary` union in `api/types.ts`, which `tsc` checks the table against (like approval
  previews); a Rust-side list of kinds was dropped as a second copy to keep in sync.
- In `ja` and `en`, the chat chips for logging a meal and approving a preference read in
  that language.
- fmt, clippy, tests, `tsc --noEmit` and `npm run build` pass.

### AL1 — Store and run scope
Migration, `audit.rs` (`AuditLog`, `scope`, `CURRENT_RUN`, `with_run`), `Actor`,
derived `interrupted`, retention, Cloud Logging line.
**Done when:** unit tests cover start/step/finish, ordering, `interrupted` derivation via the
injected `Clock`, retention, steps outside a scope being ignored, and argument truncation;
fmt, clippy and tests pass.

### AL2 — Recording points
§5 in core: chat loop, `run_or_queue`, approvals, usage sink, budget guardrail, planner
validation, plan saved; the scheduler passes `Actor::Scheduler`.
**Done when:** orchestrator tests with `MockChatModel` show the expected step sequence for:
- a plain reply
- a tool call
- an invalid-arguments call
- an unknown tool
- a queued then approved action, with both runs linked
- the iteration cap
- a token-budget refusal

A scheduler tick with a mock planner records a `plan_scheduled` run with `plan_validation`
and `plan_saved` steps, and a planner retry shows `retried: true`.

### AL3 — API
§6.1 commands and the SSE event, in `lifeagent-api` and the server.
**Done when:**
- Server route tests cover paging, filters, run detail and the summary.
- Another user's run id returns `not_found`, because the lookup happens in the caller's own
  schema.

### AL4 — Activity page
§6.2 and §6.3 in core, `lifeagent-api` and `apps/web`, three locales, CLI `activity`.
**Done when:**
- `tsc --noEmit` and `npm run build` pass.
- Manually, with the mock LLM: chat a meal, have a preference queued and approved, trigger a
  tick, and see all three runs with their steps and live updates.
- A test covers the setting's default and round trip.
- Manually: hide the tab, chat once, show it again, and see that run listed.

## 11. Testing notes

- Never call a real LLM; use `MockChatModel`. Take time from the injected `Clock`.
- Assert that no `agent step` log line at INFO contains tool arguments or the input excerpt.
- The task-local must not leak between runs. A test runs two chat turns concurrently on two
  tenants and checks that each run holds only its own steps.
