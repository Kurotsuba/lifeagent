# Decisions

Deviations from `PHASE1_SPEC.md` and resolutions to ambiguities it left open, in the order they
were made. Per `CLAUDE.md`: "If something is ambiguous, pick the simplest option consistent with
the spec, note the decision here, and continue."

Open items awaiting a decision are tracked in `docs/BLOCKERS.md`, not here — an entry moves from
there to here once resolved.

## 2026-09-23 — M0: closed. `lifeagent-desktop` build/run verified after system packages installed

`docs/PHASE1_SPEC.md` §14 M0 "done when" requires `cargo build --workspace` and
`npm run tauri dev` to start, with clippy clean. Initially blocked in this Linux/WSL sandbox:
`pkg-config` itself was not installed, and the sandbox had no non-interactive `sudo` to install
it or the GTK/WebKit/D-Bus dev packages Tauri 2 needs to link on Linux.

Resolved by installing, on this machine:

```
sudo apt install -y pkg-config libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  libdbus-1-dev patchelf
```

After that, full verification passed:

- `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo test --workspace` — all clean, including
  `lifeagent-desktop`.
- `npm run tauri dev` (run from `apps/desktop`, see the layout fix below) opened the main
  window at the configured 1100×720 size with the WebKit webview rendering the React/Tailwind
  page and the `zh-CN.ts` string, confirmed by screenshot (`import -window <id>`, with
  `GDK_BACKEND=x11` forced — WSLg's default GTK/Wayland surfaces aren't visible to X11
  screenshot tools, so this override was needed for verification only; not something the app
  needs to set itself, and not something the desktop app's own code should force at runtime).

**Layout fix made during this verification:** the `ui/package.json` originally owned the
`tauri` npm script and `@tauri-apps/cli` devDependency, colocated with the Vite/React files.
Run from `apps/desktop/ui`, the Tauri CLI searches only *subfolders* of its cwd for
`tauri.conf.json` and cannot find `apps/desktop/src-tauri/tauri.conf.json`, which is a sibling,
not a child. This also didn't match `CLAUDE.md`'s own documented command
(`cd apps/desktop && npm install && npm run tauri dev`).

Fixed with an npm workspace: `apps/desktop/package.json` (workspace root, `"workspaces":
["ui"]`) now owns `@tauri-apps/cli` and the `tauri` script; `apps/desktop/ui/package.json` keeps
the frontend-only `dev`/`build`/`preview` scripts and dependencies. `tauri.conf.json`'s
`beforeDevCommand`/`beforeBuildCommand` use the object form (`{"script": "npm run dev", "cwd":
"../ui"}`) since those hooks run relative to `tauri.conf.json`'s own directory (`src-tauri`),
not the invocation cwd. `CLAUDE.md`'s documented command now works as written.

## 2026-09-23 — M1: closed. Foundations built without pulling `lifeagent-llm` types forward

`docs/PHASE1_SPEC.md` §14 M1 covers Clock, logical date, DB + core migrations, settings,
secrets, profile/goal services, the event store + bus, memory notes and the conversation store.
Two places in the spec's own text for these modules refer to a type that, per the milestone
order, does not exist yet:

- §7.2 lists a `models: ModelsConfig` setting, but `ModelsConfig` is defined in
  `lifeagent-llm` §6.3, which is M2.
- §7.10's conversation store speaks in terms of `lifeagent-llm::Message` variants
  (assistant/tool/...), same M2 dependency.

Resolved both without touching `lifeagent-llm` in M1, so the crate boundary and milestone order
stay intact:

- `settings.rs` implements `get_json<T>`/`set_json<T>` generically (`T: Serialize` /
  `DeserializeOwned`) over the existing `key -> value_json` table, plus concrete accessors for
  the four settings whose type is already known (`plan_time`, `day_boundary`,
  `notifications_enabled`, `autostart`, `onboarding_complete`). `models`/`set_models` will be a
  thin `get_json::<ModelsConfig>("models")` / `set_json(...)` wrapper added in M2 — no change to
  `settings.rs` needed then.
- `conversation.rs` defines its own `MessageRole`/`StoredMessage`/`NewMessage`, shaped directly
  after the `messages` table's columns (role/content/tool_calls_json/tool_call_id/tool_name)
  rather than `lifeagent-llm::Message`. This is a reasonable fit regardless of the type-ordering
  issue: the table is already row-shaped, not enum-shaped, so a `StoredMessage` matches it more
  directly than the wire-format `Message` would. Translating `StoredMessage` into
  `lifeagent-llm::Message` for a `ChatRequest` becomes the orchestrator's job in M3, which is
  also the first place in `core` that needs to depend on `lifeagent-llm`'s types for real.

Also clarified: `CoreApp`/`CoreAppBuilder` (§7.1) are **not** part of M1 despite being the first
thing §7 describes — §14 explicitly assigns "`CoreApp` builder and `send_message`" to M3, since
building it needs the plugin trait, tool registry and orchestrator loop that don't exist until
then. M1 delivers the standalone service modules (`db`, `clock`, `time`, `settings`, `secrets`,
`profile`, `goals`, `events`, `memory`, `conversation`) that M3's `CoreAppBuilder` will wire
together; each is independently constructed (`SqlitePool` + `Arc<dyn Clock>`, mostly) and
independently tested, which is also why M1's "done when" in §14 names per-module test coverage
rather than an integration test through `CoreApp`.

Two SQL ordering bugs surfaced by the tests themselves, fixed rather than filed: `memory.rs`'s
`list()` and `events.rs`'s `list()`/`latest()` originally ordered only by a stored timestamp
column; under a `FixedClock` (and in principle under real time, for two events logged within
the same second) two rows can share that timestamp, making "newest/oldest first" order
unspecified. Added `rowid` as a secondary sort key (SQLite's implicit, strictly-increasing
insertion-order column) everywhere this could bite.

## 2026-09-24 — M2: closed. LLM layer built; `effort` added to `ChatRequest`

`docs/PHASE1_SPEC.md` §6 gives `ChatRequest` six fields (`model`, `messages`, `tools`,
`temperature`, `max_tokens`, `response_format`) and separately gives `RoleBinding` an
`effort: Option<Effort>` used "Anthropic only" (§6.3's table: parser=low, planner=high,
conversation=medium, mirroring `temperature`'s per-role defaults for the OpenAI-compatible
kind). But `ChatModel::complete` takes only `ChatRequest`, and the router dispatches to
whichever provider by calling that one method — so there is no path left for a resolved
`effort` value to reach the Anthropic provider once `RoleRequest` (built by the caller) is
merged with the binding. This is the same class of defect as `docs/BLOCKERS.md`'s B1–B4
(expression-level gap, not a design problem), just not filed there since it blocks M2 itself
rather than M4/M5.

Resolved by adding `pub effort: Option<Effort>` to `ChatRequest` (`crates/llm/src/types.rs`),
symmetric with `temperature`: each provider reads the field it needs and ignores the other,
exactly as §6.2.2 already says of `temperature` ("ignored for `ProviderKind::Anthropic`").
`RoleRequest` (§6.3: "`ChatRequest` without `model`/`temperature`") is defined to also exclude
`effort`, for the same reason it excludes `temperature`: both are router-resolved from the
binding, not caller-supplied. The router computes `(temperature, effort)` once per call from
`ProviderKind` and either the binding's explicit value or the per-role default table.

Also: `Effort` itself is defined in `types.rs` rather than `router.rs` (where §6.3's code block
places it), purely because `ChatRequest` needed it and `types.rs` (§6.1) is a lower module than
`router.rs` (§6.3); `router.rs` uses `crate::types::Effort`. No shape change, just which file
owns the `enum`.

Two smaller gaps, resolved without a decision needed (simplest compliant choice per
`CLAUDE.md`):

- `LlmError::InvalidResponse` and `LlmError::Config` carry a `String` (spec names only the
  variants, not their fields, for these two — `Status`'s and `RateLimited`'s shapes were
  already spelled out).
- `KeyProvider::get_key` and `UsageSink::record`/`UsageEvent` aren't given field-level shapes
  in §6.3 beyond "a `KeyProvider`" / "a `UsageSink` trait ... writes `llm_usage`". `UsageEvent`
  mirrors the `llm_usage` table columns (§5) minus `id`/`created_at`, which the sink
  implementation (in `lifeagent-core`, M3/M5 per `usage.rs`'s existing stub) assigns.

Provider mapping (`build_request_body`/`parse_response_body` in both `provider/*.rs`) is kept
as pure functions separate from the HTTP-calling `ChatModel::complete`, so the "provider
mapping test suites... with fixtures" `§14` done-when criterion is testable without a live or
mocked HTTP server. Retry/backoff policy (timeout, retryable-status classification, `Retry-After`
parsing, body-excerpt truncation) is similarly split into pure functions in `provider/mod.rs`
and tested directly; the HTTP-calling glue around them is exercised only by real network use
(out of scope for offline tests, per `CLAUDE.md`).

Verification: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings` and
`cargo fmt --all -- --check` all pass for `lifeagent-llm`, `lifeagent-core`,
`lifeagent-plugin-diet` and `lifeagent-cli` (122 tests total, including the new provider
mapping fixture tests, router role-resolution tests, and `complete_json` repair-loop tests).
`lifeagent-desktop` was not rebuilt in this session: the sandbox's apt mirror returned 404 for
the WebKit/GTK dev packages M0's `DECISIONS.md` entry lists (mirror snapshot mismatch, not a
code issue); M2 doesn't touch `apps/desktop` or `lifeagent-core`'s M3+ modules, so this doesn't
gate M2's own done-when criteria.

## 2026-09-24 — M3: closed. Plugin API, tool registry, orchestrator, `CoreApp`

`docs/PHASE1_SPEC.md` §14 M3 covers the `Plugin` trait, plugin migrations, the tool registry,
core tools "except plan tools, stubbed", the context builder, the orchestrator loop, and
`CoreAppBuilder`/`send_message`. Several shapes needed judgment calls beyond what §7 spells out:

**Plan tools are not registered at all, not registered-and-erroring.** §14's parenthetical
"(except plan tools, stubbed)" could mean either "register `core_get_plan` /
`core_generate_plan` / `core_revise_plan` with handlers that return an error" or "don't register
them yet." Chose the latter: there is no plan service (M5) for them to call into, so a
registered-but-broken tool would just be a `PluginError::Unsupported` wrapped in ceremony, and
an LLM that tries to call an unregistered tool already gets a graceful
`{"error": "unknown tool ..."}` result through the normal `NotFound` path (§7.11's error
handling covers it for free). Revisit when M5 adds the plan service: those three tools get real
specs and handlers then.

**`usage.rs` (`DbUsageSink`) was implemented now, not deferred to M5.** `lib.rs`'s M1-era doc
comment groups `usage` with `plan`/`scheduler` as M5 work, but `LlmRouter::new` and
`with_providers` both require a `UsageSink` to be constructed at all — including the
`chat_model_override` / mock path tests use. Since the `llm_usage` table already exists from
the M1 core migration, implementing the real sink (one `INSERT`, matching `EventStore`'s
pattern) was less work and less indirection than a throwaway no-op sink that M5 would then
delete. `lib.rs`'s module doc comment is updated accordingly.

**`CoreApp.router` is `Option<Arc<LlmRouter>>`, not required.** Onboarding can leave `models`
unset (§7.2: "none (onboarding required)"), and `CoreAppBuilder::build` must still succeed in
that state — a fresh install has to reach the onboarding UI before any model exists to route
to. `send_message` and the private `plugin_context()` helper both fail fast with
`CoreError::Validation` when the router is `None`, which also means **no `PluginContext` can be
constructed at all until models are configured** — so `today_view`/`today_card`-style read paths
(M4) will hit the same wall even though they don't need the LLM themselves. Accepted for now
because M3's actual surface (`send_message`) always needs the router anyway; if M4's `today_view`
proves this too restrictive (e.g. the Today page should render before onboarding finishes
model setup), the fix is to give `PluginContext.llm` its own `Option`-free "not configured" error
path independent of `send_message`'s, not to change M3's code.

**`PluginContext`'s spec-listed `profile: ProfileService // profile, active goals, latest
weight` comment is realized as three fields, not one.** `ProfileService` (M1) only reads/writes
the `Profile` struct; it has no notion of goals or weight. Rather than bloat `ProfileService`
with unrelated responsibilities, `PluginContext` keeps `profile: ProfileService` for the profile
itself and adds `goals: GoalsService` (already exists from M1) for active goals; latest weight
is already reachable via `events.latest("body.weight_logged")`, also already on the context. Net
effect is the same three pieces of data the spec's comment names, just not folded into a single
field.

**The tool-argument validator (§7.7) is intentionally shallow.** Implemented `object`, `string`,
`number`, `integer`, `boolean`, `array` and `enum`, with recursion through `properties` and
`items` — exactly the spec's list, no `oneOf`/`anyOf`/`pattern`/numeric bounds/etc. This is
sufficient for every core and (planned) diet-plugin tool spec, which all validation happens
against; extending it only becomes necessary if a future tool spec needs a schema feature these
don't.

**The context builder's "Plans" section (§7.9) is a fixed English placeholder
(`"No plan for today or tomorrow yet."`)**, since the plan service (M5) doesn't exist yet and
there is nothing real to report. `build_system_prompt`'s signature does not thread plan data
through in anticipation of M5 — that would be speculative plumbing for a shape M5 hasn't
determined yet (violates `CLAUDE.md`'s "don't design for hypothetical future requirements").
M5 will need to pass `today`/`tomorrow` `StoredPlan` summaries into this function; that's a
signature change to make then, not now.

**Orchestrator loop and `CoreApp` live in `app.rs`, not a separate `orchestrator.rs`.** §2's
repository layout table lists `orchestrator.rs` as its own file, but `lib.rs`'s own M1-era doc
comment (written before M3, describing the intended M3 shape) already grouped the orchestrator
into `app.rs` alongside `CoreApp`/`CoreAppBuilder`, and `send_message` — the orchestrator loop
itself — is explicitly part of `CoreApp`'s public surface per §7.1's code block. Splitting it
into a second file would just move private helper functions (`to_llm_messages`,
`serialize_tool_calls`) across a file boundary for no encapsulation benefit, since they only
serve `send_message`. Kept as one file; if `app.rs` grows unwieldy once M5 adds plan methods,
splitting becomes a mechanical refactor, not a design decision.

**`chat_model_override`/`--mock` builds a synthetic single-provider `ModelsConfig`** (one
`ProviderConfig { id: "mock", ... }` bound to all three roles) rather than requiring
`settings.models()` to already hold a config naming that provider. This keeps the override a
pure bypass of real provider construction, matching its doc comment ("bypasses real provider
construction entirely") and §7.1's own description ("tests & CLI `--mock`") — a test or
`--mock` run shouldn't need onboarding to have run first.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, and `cargo test --workspace` all pass — 117 unit tests in `lifeagent-core` (up from
the M2 baseline; includes the new `plugin`/`tools`/`core_tools`/`context`/`app`/`usage` suites)
plus 3 orchestrator integration tests in `crates/core/tests/orchestrator.rs` against a tiny
test plugin (tool call → reply; invalid args → model retries; iteration cap at 6), satisfying
§14 M3's "done when." `lifeagent-llm`'s 46 tests are unchanged from M2.

## 2026-09-24 — B1 resolved: goal-weight protein anchor, lower factors, macro-feasibility floor

`docs/BLOCKERS.md`'s B1: the macro split could produce zero or negative `carbs_g`, because
`protein_g = w × factor` (anchored to *current* body weight, uncapped) plus the `fat_g ≥ 0.6·w`
floor could consume the entire kcal budget for a heavy user, and the spec's own `if carbs_g <
100` repair branch was a no-op whenever the fat floor was already binding (it recomputes the
same floor value). Full mechanism explained to the user before deciding; worked example: female,
150 kg, 155 cm, 60 y, Lose → kcal floored to 2010; old formula gave protein 300 g (1200 kcal) +
fat 90 g (810 kcal) = 2010 = the whole budget → carbs 0.

Two decisions were needed (both marked **DECIDE** in `docs/BLOCKERS.md`); resolved by the user:

1. **Protein anchor:** switched from `w × (2.0 Lose / 1.8 Gain / 1.6 Maintain)` to
   `anchor_w × (1.8 Lose / 1.6 Gain / 1.2 Maintain)`, where `anchor_w =
   target_weight_kg.unwrap_or(w)` — the user's explicit choice, combining the spec's option (b)
   "goal-weight anchor" with lower per-kg factors than any of the three originally proposed
   options. Rationale (user-stated intent, as understood from the choice): protein needs should
   scale toward the body being built *toward*, not the one being left, and the lower factors
   reduce how much headroom the fix needs from `fat_g`'s floor in the first place. A floor
   `protein_g ≥ 1.2·w` (current weight) is layered on top so protein for the *actual* body eating
   the diet never falls unreasonably low just because a much smaller goal weight was set — e.g.
   a 150 kg user with a 65 kg target now gets `protein_g = max(65×1.8, 1.2×150) = max(117, 180) =
   180`, so the current-weight floor is what actually binds in that case, not the far-off goal
   weight.
2. **Infeasible case:** if even after (1) the budget can't fit `protein_g` + the `0.6·w` fat
   floor + a 100 g carb minimum, **raise `kcal`** to whatever those three require and set
   `floor_applied = true`, rather than silently accepting protein below the safety floor. This
   reuses the existing `floor_applied` boolean (previously only set by the absolute
   `max(BMR, 1500/1200)` floor) rather than adding a second flag to `NutritionTargets` — the UI
   only ever needs to show one "adjusted for safety" note, not attribute which floor fired.

Also resolved without needing a decision (simplest compliant choice, not one of the two
**DECIDE** items):

- **`round_to_10` → `round_up_to_10`** everywhere it's used (both the original absolute floor
  and the new macro-feasibility floor): rounding *down* to the nearest 10 could push `kcal`
  below the very floor it was just clamped to (e.g. a floor of 2007.75 rounds down to 2000,
  violating `kcal ≥ floor`); rounding up preserves the invariant exactly, at the cost of at most
  9 kcal of overshoot, negligible against the ±8% tolerance used elsewhere in the plan validator
  (§8.10).
- `effective_direction` (the existing "goal already reached → treat as Maintain" conversion,
  §8.6) now explicitly drives the macro split too, not just `delta` — the spec's original
  pseudocode only named it in the `delta` computation, but the same conversion has to apply to
  which protein factor and anchor logic is used, or a user who reached their goal would still
  get Lose/Gain-level protein under a Maintain-level calorie budget.

Amended `docs/PHASE1_SPEC.md` §8.6 (formula) and §13 (added a goal-weight-anchor case, the
originally-broken heavy-weight case now asserting `carbs_g > 0`, and a case that exercises the
new macro-feasibility `floor_applied` path to the required targets-engine tests).
`docs/BLOCKERS.md`'s B1 entry is removed as resolved; B2, B3, B4 remain open. No code changes
yet — `lifeagent-plugin-diet`'s `targets.rs` is still an M4 stub, so this is a spec-only
resolution; M4's implementation will follow this corrected formula directly, tests included.

## 2026-09-24 — B2 resolved: `carbs_g` is total carbohydrate, fiber included

`docs/BLOCKERS.md`'s B2: the per-100g sanity check treated fiber as an addend separate from
carbs (`protein + carbs + fat + fiber ≤ 100`) and double-charged it in the energy formula
(`e = 4·protein + 4·carbs + 9·fat + 2·fiber`), while food composition tables report fiber
*inside* total carbohydrate. Wheat bran (protein 15.5, carbs 64.5, fat 4.3, fiber 42.8) sums to
127 under the old rule → hard rejection → `PluginError::Other("无法估算该食物营养")`, for a food
whose numbers are simply correct under the standard convention. §9.2/§9.3's own prompts never
told the model which convention to use either, so nothing was actually broken by a wrong model
answer — the validator was rejecting a right one.

`docs/BLOCKERS.md` flagged this as needing no real judgment call (unlike B1's two **DECIDE**
items), so resolved directly with its recommended fix rather than asking:

- **`carbs_g` = total carbohydrate, fiber included.** This is what food composition tables
  report, so it's what an LLM asked for "nutrition values... consistent with standard food
  composition tables" (§9.3's own prompt) will produce by default — the fix aligns the
  validator with the model's natural output rather than the other way around.
- Sum rule → `protein + carbs + fat ≤ 100` (fiber no longer a separate addend; it's already
  inside `carbs`).
- Energy rule → `e = 4·protein + 4·(carbs − fiber) + 9·fat + 2·fiber` (the non-fiber part of
  carbs at 4 kcal/g, fiber itself at 2 kcal/g — so fiber contributes once, not twice).
- Added validity rule `fiber ≤ carbs` (fiber can't exceed the total it's part of).
- **Stated the convention explicitly in the §9.3 prompt** ("`carbs_g` is total carbohydrate,
  fiber included... Do not subtract fiber from it") — per `BLOCKERS.md`'s own note, this half
  matters more than the validator change: fixing only the validator while leaving the prompt
  silent would leave the model's choice a coin flip again as soon as it happened to pick the
  other convention.

Checked, no change needed: §8.6's `fiber_g = 14·kcal/1000` plan target and §8.10's "fiber < 70%
of target" warning both compare a fiber value against a separately-computed fiber value —
neither depends on whether fiber is also counted inside `carbs`, so both already read correctly
under the new convention.

Amended `docs/PHASE1_SPEC.md` §8.3 (validator rules), §9.3 (prompt), and §13 (added the wheat
bran pass case and a `fiber > carbs` rejection case to the required nutrition-sanity-check
tests). `docs/BLOCKERS.md`'s B2 entry removed as resolved; B3, B4 remain open. No code changes
yet — `lifeagent-plugin-diet`'s sanity-check function is still an M4 stub; M4's implementation
follows this corrected rule directly, tests included.

## 2026-09-24 — B3 resolved: catch-up anchors to the logical day, not "today's calendar date"

`docs/BLOCKERS.md`'s B3: §7.13's "past `plan_time`" check compared against "today's calendar
date at `plan_time`," evaluated as of the moment of the check — but between midnight and
`day_boundary` the calendar date has already advanced while the logical day hasn't, so that
phrase silently means "tomorrow's calendar date" during exactly that window. Worked case (also
acceptance scenario §15.9): relaunch at 01:00 after the app was down through the previous
evening's `plan_time` (21:00). `logical_today` = the previous calendar day (the 23rd, since
01:00 < the 04:00 boundary); `tomorrow` = the 24th, with no plan yet. The old rule computed
"today's calendar date" as the 24th (the actual calendar date *right now*, at relaunch) and
checked `01:00 ≥ (24th, 21:00)` — false — so catch-up silently never fired, even though
`plan_time` had already passed hours earlier on the 23rd. Explained in full to the user before
deciding, since the fix's shape (anchor to `plan_time` *within* `logical_today`, giving `due_at`
= `logical_today`'s own calendar date at `plan_time`) left two things genuinely undefined.

Both marked **DECIDE** in `docs/BLOCKERS.md`; the user chose both recommended options:

1. **`plan_time < day_boundary` is forbidden**, not supported. The clean anchoring rule (`due_at`'s
   calendar date = `logical_today`) only holds when `plan_time ≥ day_boundary`; the opposite case
   needs a second rule (`due_at`'s calendar date = `logical_today + 1`) for a `plan_time` value
   (small hours of the morning, generating *tomorrow's* meal plan) that isn't meaningful UX
   anyway. Added as a settings-validation rule (§7.2): `plan_time ≥ day_boundary`, rejected
   otherwise — one anchoring rule, one branch to test, matching B1 and B2's own "simplest
   compliant alternative" pattern rather than adding a second code path for an edge case with no
   real use.
2. **The "gap" case is left alone, not backfilled.** Catch-up only ever asks "does `tomorrow`
   have a plan?" — it does not also ask "does `today`?" So after an outage long enough that even
   `today` has no plan (not just one missed overnight cycle), nothing retroactively fills it in;
   the user sees the Today page's existing `today_plan: None` empty state and clicks 立即生成
   manually. This was already the design's implicit behavior ("catch-up only ever targets
   tomorrow"), now made explicit and intentional rather than an unnoticed gap. Revisit only if
   real usage shows multi-day outages are common enough that this friction matters.

Amended `docs/PHASE1_SPEC.md` §7.13 (corrected anchoring rule, explicit statement of both
decisions), §7.2 (added the `plan_time ≥ day_boundary` validation rule the scheduler now relies
on), and §13 (added the 01:00-relaunch case, a validation-rejection test, and a test confirming
today's own missing plan is *not* backfilled, to the required scheduler tests).
`docs/BLOCKERS.md`'s B3 entry removed as resolved; only B4 remains open. No code changes yet —
`scheduler.rs` is still an M5 stub; M5's implementation follows this corrected rule directly,
tests included.

## 2026-09-24 — B1/B2 follow-up after review

A review pass over the B1–B3 resolutions found issues. The user decided on each:

- **B1: protein safety floor lowered from `1.2·w` to `0.8·w` (RDA level).** The `1.2·w`
  floor was not something the user asked for. It was added while interpreting the answer to the
  infeasibility question, and in practice it cancelled the goal-weight anchor for the heavy
  users it was meant to help: 150 kg with a 65 kg target gave `max(1.8×65, 1.2×150) = 180 g`.
  With `0.8·w` the same case gives `max(117, 120) = 120 g`. The floor still binds here, but it
  adds 3 g over the goal-weight value instead of 63 g. In general it only binds when the target
  is far below current weight (for Lose, a target under ~44% of current weight; 65/150 is 43%).
  §8.6 amended. The example in the B1 entry above is superseded by this one.
- **B1: default target weight for Lose/Gain goals saved without one (§7.4).** The target stays
  optional in the UI, but when it's left empty, saving the goal stores latest weight − 1 kg
  (Lose) or + 1 kg (Gain), clamped to the 30–300 kg validation range, without asking the user.
  It is **stored once at save time, not recomputed**: a 70 kg user saving "Lose" with no target
  gets `target_weight_kg = 69`, and after reaching 69 kg the existing §8.6 "goal reached →
  Maintain" rule applies and the deficit stops until they set a new goal. That's the user's
  explicit choice over recomputing (target always trailing current weight by 1 kg, never
  reached). If no weight has been logged at save time, the target stays unset and §8.6's
  `unwrap_or(w)` fallback applies. Clamping keeps the stored value valid under §7.4's own
  30–300 kg rule. For example, a 30 kg user saving Lose gets target 30, which is immediately
  "reached" (Maintain). That's correct, since they can't go below the minimum anyway. Code:
  `GoalsService` has no access to weight, so the default belongs wherever goal saving is
  wired with the event store (`CoreApp`'s save-goal path, M6/M7). §7.4, §8.6 comment and §13
  amended.
- **B1: a Lose goal can still end up with a small surplus, accepted.** A 1 kg default barely
  moves the protein anchor off current weight, so the macro-feasibility raise can still push a
  Lose user's kcal slightly above TDEE when the fat floor, protein and carb minimum don't fit.
  Example: female, 100 kg, 150 cm, 80 y, sedentary, Lose → kcal 1660 against TDEE ≈ 1652.
  The user chose to accept this rather than cap kcal at TDEE for Lose: it's rare, the surplus is
  tens of kcal, and `floor_applied` is set, so the UI still shows the safety note.
- **B2: `diet_update_food` (§8.5) does not state the carbs convention. Deferred, known gap.**
  §9.3's estimator prompt now says `carbs_g` includes fiber, but the `diet_update_food` tool
  description (user-supplied per-100 g values, often read off a package label) does not.
  Chinese labels (GB 28050) and the China Food Composition Table include fiber in 碳水化合物,
  so zh-CN users are unaffected. EU-style labels exclude fiber, so a user copying one would
  store carbs understated by the fiber amount. User-sourced values bypass the §8.3 sanity check
  (`source='user'` always wins), so nothing catches this. Fix when it matters: add one line to
  the tool description stating the convention.
- **B3: no change.** The two implementation notes from the review stay unrecorded for now at
  the user's request.

## 2026-09-24 — M4: closed. Diet plugin logging

§14 M4: food table, resolver, sanity check, parser, tools, intake, targets engine, context
snippet, today card, plan-vs-actual. Done when diet unit tests pass and an integration test
logs a meal via the orchestrator and `today_view` shows correct totals
(`crates/plugin-diet/tests/logging.rs`). Before starting, a check against the spec found
several places where M4 depended on things from M5 or left them undefined. The user approved
these resolutions:

**Core pieces pulled into M4:**

- **Plan types and `PlanStore` (read + insert).** Plan-vs-actual (§8.11), the context snippet's
  "今日计划中尚未吃的餐" line (§8.7) and `TodayView`'s plan fields (§7.14) all read plans, which
  §14 otherwise leaves to M5. `plan.rs` now has `PlanContent`, `StoredPlan`, `PlanStatus`,
  `PlanTrigger` and `PlanStore { current(date), insert(...) }` (version = max + 1, status
  proposed). Generation, revision and accept stay M5 and build on `insert`. Before M5 writes
  any plans, those M4 features read an empty table: no adherence, no snippet line, `None` plans.
  Two additions beyond §7.12/§7.6: `StoredPlan.instruction` (the table already stores it, and
  M5's revision test needs to read it back) and `PluginContext.plans` (the diet plugin needs
  plan access).
- **`WeightPoint`** (named in §7.14, never defined) is `{ kg, logical_date, occurred_at }`,
  from the latest `body.weight_logged` event.
- **`today_view` works before models are configured.** This reverses the M3 simplification
  noted above. When no model is configured, `CoreApp` now holds a placeholder `LlmRouter` with no
  providers (every call fails with `LlmError::Config`), plus a `models_configured` flag. So
  building a `PluginContext` can't fail anymore, and `send_message` checks the flag first
  (now before storing the user message). **Open for M7:** the router is built once, in
  `CoreAppBuilder::build`, so models saved during onboarding in a running desktop app won't
  take effect until something rebuilds it. The `save_models` command will need a reload path.
- **The event dispatcher (§7.5)**, planned for M3 but missed. `EventStore` now publishes each
  committed change as an `EventChange` (`Appended`/`Deleted`) on an internal channel instead of
  emitting `DataChanged` itself. A task started by `CoreAppBuilder::build` delivers appended
  events to every plugin's `on_event` (errors logged) and *then* emits `DataChanged`, matching
  §7.5's order. **Deviation:** deletes are not passed to `on_event`, only turned into
  `DataChanged`. `Event` has no marker, so a plugin would see a deletion as a new event. A
  plugin that needs deletes later should get a separate hook. Side effect: an `EventStore`
  used without a `CoreApp` (in tests) no longer emits `DataChanged`.
- `CoreError::Plugin(String)` for plugin hook failures surfaced by core (`today_card`,
  `context_snippet`). The M3 context builder was misusing `Validation` for this.
- A `test-support` cargo feature exposes `lifeagent_core::test_support` (a `PluginContext` on
  a temp DB, with a `MockChatModel` and a settable `FixedClock`) to `lifeagent-plugin-diet`'s
  tests through its dev-dependency. It's never enabled in a real build.

**Diet plugin decisions:**

- `normalize` (§8.2) strips trailing ASCII and CJK sentence punctuation but **not closing
  brackets**, because the spec's own example names like "牛肉(熟)" end in one.
- Targets (§8.6): no active goal counts as Maintain on current weight. When the
  macro-feasibility floor raises kcal, carbs are recomputed from the rounded-up kcal
  (≥ 100 g, up to ~2 g more) rather than fixed at 100. That keeps protein + fat + carbs equal
  to kcal exactly. Outputs are rounded: kcal-type fields to 1, grams to 0.1.
- Resolver: a name repeated within one meal is estimated once. A newly estimated food counts
  as used. Inserts never overwrite an existing food, so user-corrected values always win
  (§16). §8.3's `PluginError::Other("无法估算该食物营养")` reaches the model as `"internal
  error"`, because §7.11 only passes `InvalidArgs`/`NotFound` messages through. That follows the
  spec as written. Worth revisiting if users see unhelpful replies when an estimate fails.
- Parser: KNOWN_FOODS is rendered as a JSON array. `meal_type` accepts both JSON `null` and the
  string `"null"` (the prompt's own notation). Validation failures (no items, over 20, grams
  outside 1–2000) are `InvalidArgs`, so the conversation model can relay them to the user.
- `diet_update_food` checks user values against the hard §8.3 rules (rejecting impossible
  ones) but doesn't apply the kcal correction, since user values win. A missing `fiber_g`
  keeps the food's current fiber.
- The context snippet shows **full** entry ids. §8.7's example abbreviates them, but the model
  needs the full id to call `core_delete_entry`. Each meal is described by its item names
  joined with "、".
- Snack's zh-CN label is "加餐".
- Still not implemented: B1's default target weight at goal save (§7.4). It belongs in
  `CoreApp`'s save-goal path, which comes with M6/M7.

Verification: fmt, clippy (`-D warnings`, all targets) and all 232 workspace tests pass. That's
core 125 unit + 3 orchestrator integration, llm 46, and diet 56 unit + 2 logging integration.
The diet unit tests cover every §13 item for M4: targets (hand-computed example, the B1
heavy-weight case asserting carbs > 0, the macro-feasibility floor, rate cap, 1200 floor,
goal reached → maintain), the sanity check (including wheat bran and fiber > carbs), the
resolver (name hit, alias hit, miss → estimate → cached with no second LLM call), and parser
validation and default meal type by time.

## 2026-09-24 — B4 resolved: daily backup via `VACUUM INTO`, dated files, keep 7

`docs/BLOCKERS.md`'s B4: §12 had the scheduler copy `lifeagent.db` once a day. The database runs
in WAL mode, where recent commits live in the `-wal` file until a checkpoint, so a plain copy
of the main file is stale or torn. The same sentence also contradicted itself: one fixed name,
`lifeagent.db.bak`, but "keep last 7 dated copies".

`BLOCKERS.md` flagged this as needing no judgment call, so it was resolved directly with its
fix, plus the details it left open:

- **`VACUUM INTO`** instead of a file copy. It writes a consistent snapshot, WAL contents
  included, in one statement through `sqlx`. The target path is bound as a parameter
  (`VACUUM INTO ?`, which SQLite accepts) rather than spliced into the SQL. sqlx 0.9 rejects
  dynamically built SQL anyway.
- **Dated files, 7 kept:** `{data_dir}/backups/lifeagent-YYYY-MM-DD.db`, named by logical
  date; older ones beyond the newest 7 are deleted.
- **Whether a backup is due comes from the file on disk**, not an in-memory flag: each tick
  takes one if today's file doesn't exist yet. That gives "first tick after the day boundary"
  exactly, survives restarts, and never takes two the same day.
- **Write to `.tmp`, then rename.** `VACUUM INTO` fails if the target exists, so a crash
  mid-write that left a partial file under the final name would block every later attempt.
  A leftover `.tmp` is removed before each run.
- **Failures** log a warning and retry on the next tick. They don't emit `PlanFailed`, which is
  about plans and drives user notifications (§10.1).
- **Backups run before onboarding and without models configured.** §7.13 said the scheduler
  "skip[s] entirely" in that state. That now applies only to plan generation, since a backup
  needs no model. §7.13 was amended to say so.

Amended `docs/PHASE1_SPEC.md` §12, §7.13 and §13 (backup tests: a row still in the WAL is in
the backup, one backup per logical day, retention of 7, a leftover `.tmp` doesn't block).
`docs/BLOCKERS.md` now has no open blockers. No code changes yet: the backup is part of the
scheduler, which is M5.
