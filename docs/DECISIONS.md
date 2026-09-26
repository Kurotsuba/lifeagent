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

## 2026-09-24 — M5: closed. Planning, scheduler, backup

§14 M5: plan service (generate, retry, store, revise, accept), diet plan input and
finalize/validate, plan core tools, scheduler, daily DB backup. Done when planning and
scheduler integration tests pass (`crates/plugin-diet/tests/planning.rs`).

**Decided by the user before starting:**

- **`{user_language}` (§9.4) is Simplified Chinese, with i18n kept open.** No language setting
  exists, so `lifeagent_core::i18n::Language` has one variant, `ZhCn`, and the planner reads
  `USER_LANGUAGE.prompt_name()`. A second language is a new variant plus a setting. §9.4 amended.
- **`AppEvent::PlanReady` carries `trigger`**, so the desktop can notify only for scheduled
  plans (§10.1) without a later breaking change. §7.5 amended.
- **The `plan_time ≥ day_boundary` check (§7.2, from B3) is enforced in both setters.**
  Changing `day_boundary` later can break it just as changing `plan_time` can. Neither setter
  validated anything before.

**Other decisions:**

- **One `PlanService` per app.** `CoreApp`, the scheduler and the plan core tools (through
  `ToolRegistry`, which now takes the service as a parameter) share one instance, so §7.12's
  "one generation at a time" lock covers chat, the shell and the scheduler alike.
- **Planner prompt.** Sent as one user message built from three constants: header,
  per-section, revision (§9.4's loop and conditional). User-supplied text (profile, notes,
  brief, instruction) is substituted last in each part, so braces in it aren't read as
  placeholders. §9.4's "You create tomorrow's plan" is kept verbatim even for other dates; the
  `Date:` line says which. The retry appends the model's output re-serialized from the parsed
  JSON, since `complete_json` returns the parsed value, not the raw text.
- **Violation and warning text is zh-CN.** Unmet violations end up in `warnings` shown to the
  user (§7.12 step 6), so they're written for the user. The retry request around them stays
  English like the rest of the prompts. A missing section reads "计划缺少“diet”部分".
- **Revising needs an existing plan:** `CoreError::NotFound` otherwise, and an empty
  instruction is a validation error.
- **The context builder's "Plans" section (§7.9)** now shows today's and tomorrow's plans.
  Core doesn't know section schemas, so it renders any section with a `meals` array shaped like
  the diet section's (`meal_type`, `title`, `items[{name, grams}]`) and skips others.
- **Plan core tools return that compact form, not the full plan JSON.** Tool results are cut
  at 4,000 characters (§7.10), and a full plan can exceed that, which would hand the model
  truncated JSON. `core_revise_plan` defaults its date like `core_get_plan` (tomorrow if it
  has a plan, else today); §7.8 gave no default. `NothingToPlan` reaches the model as a
  `NotFound` message ("complete the profile and log a weight first") instead of "internal
  error".
- **Scheduler.** It checks `CoreApp`'s `models_configured` flag rather than
  `settings.models()`: with models saved after startup (the open M7 reload issue), the
  setting would say yes while the router can't call anything, and every tick would fail
  three times. Attempts are counted per target date, failures only, in memory (§7.13).
  `PlanFailed` comes only from the scheduler; manual and chat generation return errors to
  their caller.
- **Backup** as resolved in B4 (`VACUUM INTO ?` with a bound path, dated files, keep 7,
  `.tmp` then rename).
- **Diet plan input (§8.9):** the meal rule in the brief ("exactly 3 main meals plus
  0–(meals_per_day − 2) snacks") is derived from the validator's own limit (total ≤
  meals_per_day + 1), so the planner is told what will be checked. Targets are passed as
  the five macro values, without BMR/TDEE/delta.
- **Diet finalize (§8.10):** planned items go through the same resolver as logged meals, so
  they count toward `use_count`, and with it the KNOWN_FOODS ranking. That's accepted, since
  planned foods are likely to be eaten. Allergy and dislike checks match against both the
  planned and the canonical name. Matching "in both directions" (§8.10) can over-flag short
  names ("油" against an allergy to "花生油"). For allergies, erring toward a false alarm is
  the intended side.
- Added `PluginContext::today()` (logical date), now used by core tools, the context builder
  and the diet plugin.

Verification: fmt, clippy (`-D warnings`, all targets) and all 276 workspace tests pass. That's
core 149 unit + 3 orchestrator, llm 46, and diet 70 unit + 2 logging + 6 planning/scheduler
integration. The planning tests cover the §13 required cases:
- a kcal violation is retried, the second request names it, and the valid plan is stored
  as version 1;
- a revision creates version 2 with trigger `revision` and the instruction stored;
- scheduler catch-up at 01:00 after plan_time generates exactly once, doesn't backfill
  today, and takes the backup;
- not due before plan_time;
- skipped before onboarding;
- `PlanFailed` plus a 15-minute retry, at most 3 attempts.

Unit tests cover every §8.10 violation and warning rule, the "is it time" rule across
plan_time/day_boundary combinations, the settings check, and the backup (a row still in the
WAL, one per day, retention, leftover `.tmp`).

## 2026-09-25 — M6: closed. CLI wired to `CoreApp`

§14 M6: all §11 subcommands wired to `CoreApp`; `lifeagent-cli` registers the diet plugin as
its composition root. Done when, with a real API key, `say` logs a meal and `plan generate`
produces a valid plan (manual check, noted here).

**Bug found and fixed before it shipped: `models set-key` couldn't run in the order §11 lists
it.** `CoreAppBuilder::build` builds a real `LlmRouter` as soon as `settings.models()` has a
provider, which fetches that provider's key from the secret store immediately and fails if it
isn't there yet — precisely the gap `set-key` exists to close. Since the CLI built one
`CoreApp` up front for every subcommand, running `models set-json` (which stores the provider)
and then `models set-key` for it (§11's own listed order) made `set-key` itself fail before it
ever ran, while trying to build the app. Fixed by special-casing `Command::Models{action:
SetKey}` in `main` to run before `CoreApp` is built at all, going straight to
`KeyringSecretStore` — it needs no data dir, DB or router, only the OS credential store. This
also makes `set-key` order-independent with `set-json`, not just fixed for the listed order.

**Other decisions:**

- **Data dir default**, per §3: `directories::ProjectDirs::from("", "", "LifeAgent").data_dir()`,
  overridable with `--data-dir`. No default existed anywhere yet; `crates/cli/src/paths.rs`.
- **`--mock`'s "`MockChatModel` that echoes"** (§11) is the real `lifeagent_llm::mock::
  MockChatModel`, not a hand-rolled stand-in: one echo closure is queued right before each
  `chat`/`say` turn, since the model always answers without tool calls, so
  `CoreApp::send_message`'s loop makes exactly one call per turn. `--mock` is not wired into
  plan generation — the planner role expects structured JSON an echo could never satisfy, and
  §11 scopes `--mock` to smoke-testing plumbing, not the planner.
- **Output format: pretty JSON for every subcommand except `chat`/`say`.** §11 only names JSON
  explicitly for `today`; JSON everywhere else keeps the developer harness uniform and
  scriptable, and doesn't contradict anything the spec says about the other subcommands.
  `chat`/`say` print the reply text plus one bullet per activity, matching §11's own wording
  ("prints reply and activities").
- **`plan generate`/`show`/`revise` all default `--date` to tomorrow** (today's logical date +
  1) when omitted. §11 doesn't specify a default; tomorrow is what a user actually wants from
  each of these day to day, and matches the core tools' own default in the no-existing-plan
  case (§7.8).
- **`CoreApp::plugin_context` made `pub`** (was private, M3). `foods list` and `targets` need
  a `PluginContext` to call into the diet plugin's own `FoodTable` and `intake::targets_for` —
  `core` still never depends on `lifeagent-plugin-diet` (the type is core's own); this just
  lets a composition root reach plugin-specific reads it has no other way to build, since
  `PluginContext`'s other fields (`db`, `clock`, `llm`) aren't otherwise exposed off `CoreApp`.
- **`foods list`** needed a way to list full food rows, not just names; `known_names` (§8.4,
  for the parser prompt) only returns names. Added `FoodTable::list` alongside it, same
  ordering (most used first).
- **`Food` now derives `Serialize`** (plugin-diet), so `foods list` can print it directly;
  `Macros` and `NutritionSource` already did.
- **`targets`** builds a `NutritionTargets` the same way the diet plugin's own `today_card`
  does (`intake::targets_for`, profile + latest weight + active goal, defaulting to Maintain
  with no goal), erroring with a clear message if the profile or a weight is missing yet,
  rather than printing a bare `null`.
- **`models set-json`/`profile set-json` call `Utc::now()` directly**, not through a `Clock`.
  `CoreApp` exposes no clock accessor (deliberately — `CLAUDE.md`'s "always get now from the
  injected `Clock`" is about testable core/plugin logic); the CLI is a leaf composition root
  with no time-dependent tests of its own, and the two throwaway example harnesses this
  milestone replaces did the same.
- **Added `rpassword`** (workspace dependency) for `models set-key`'s hidden prompt — §11 says
  the key is "read from stdin, hidden"; hand-rolling terminal echo suppression across Windows
  and Unix for one prompt isn't worth it. It requires a real controlling terminal (like `ssh`,
  `git credential` prompts, etc.); it cannot be driven by piped stdin, so it wasn't exercised
  by the automated verification below.
- **Deleted the two throwaway example harnesses** (`crates/core/examples/chat.rs`,
  `crates/plugin-diet/examples/try.rs`, both added just before this milestone): their own doc
  comments said they were superseded by the M6 CLI, and everything they covered (chat, diet
  logging, planning, the profile/goal seed) is now reachable through real subcommands.

Verification: fmt, clippy (`-D warnings`, all targets) and all 277 workspace tests pass (one
more than M5: `FoodTable::list`'s new unit test). Every subcommand was smoke-tested by hand
against a scratch `--data-dir` with `--mock`: `say`/`chat` echo and print activities; `today`
renders a `TodayView` with a null diet target before a profile exists; `profile set-json` then
`show` round-trips; `plan generate`/`revise` surface `CoreError::NothingToPlan`/`NotFound` as
clean errors before a profile+weight or an existing plan exist; `plan show` prints `null`
without one; `foods list` prints `[]` empty; `targets` errors with a clear message before a
profile+weight exist; `models set-json` then `set-key` (§11's listed order) both succeed, the
latter without needing `CoreApp` to build first (see the bug above).

**Verified by the user with a real API key, `chat` mode, 2026-09-25.** This sandbox had no
API key and no working OS credential store to do it here (see the WSL keyring workaround
below); the user ran it themselves against a live Anthropic model. In one turn, "我今天
80.2kg，我的目标体重是75kg。我今天摄入了200g鸡小胸，半个番茄罐头，150g西蓝花，100g干意大利
面,半个洋葱" produced three tool calls in a single round — `core_log_weight` (80.2 kg),
`core_remember` (the stated goal, as a note — see the observation below), and the diet
plugin's meal-logging tool (611 kcal, items resolved and totaled) — confirming the
orchestrator's multi-tool-call loop and the diet plugin's parser/resolver/validator work
end to end against a real model, not just mocks. "生成今天的计划" (the user asked for
*today's* plan, not the usual tomorrow default) correctly resolved to today's date and
returned a full plan — breakfast/lunch/dinner/snack, each with named dishes and gram-level
items — per its activity line: "已生成 2026-09-25 的计划（第 1 版）". §14 M6's "done when" is
satisfied: a real key logs a meal via chat and `plan generate` returns a valid plan.

**Observation from this check, not a blocker for M6:** there is no core or diet tool for
setting a `WeightGoal` from chat. Asked to record a goal weight, the model fell back to
`core_remember`, and its reply ("目标体重 75kg 已保存") reads as if a structured goal was
saved, when only a free-text memory note was. `GoalsService::get_active_weight_goal` — what
the deterministic targets engine (`targets::compute`, §8.6) actually reads — was untouched,
so the kcal/macro targets shown right after reflect Maintain at the logged weight, not a
trajectory toward the stated 75 kg goal. This matches §7.8 and §11 as written: Phase 1 gives
goal-setting no chat tool and the CLI no command for it, by design — the spec's own onboarding
flow (§15) sets the goal through a structured form, not chat. Left as-is here; worth a look if
a later milestone wants chat to be able to set or change the goal too, since right now it
*appears* to work from the reply text alone.

**Setting up a working OS keyring in WSL for this check**, since the default Linux
`keyring` backend needs a D-Bus Secret Service that WSL doesn't start on its own (`sudo apt
install gnome-keyring dbus-x11`, then `export $(dbus-launch)` and
`eval "$(printf '' | gnome-keyring-daemon --unlock --components=secrets)"` per shell). Even
then, `models set-key` failed with `secret store error: Couldn't access platform storage: SS
error: result not returned from SS API` — WSL's `gnome-keyring-daemon` creates only a
transient `session` collection with no `default` alias, and the `keyring` crate's Linux
backend (`zbus-secret-service-keyring-store`) specifically requires one; this is a documented
upstream limitation (github.com/open-source-cooperative/keyring-rs/issues/133), not a
lifeagent bug. Worked around, for manual testing only, by aliasing `default` to the existing
`session` collection over D-Bus directly:
`gdbus call --session -d org.freedesktop.secrets -o /org/freedesktop/secrets -m
org.freedesktop.Secret.Service.SetAlias "default" "objectpath
'/org/freedesktop/secrets/collection/session'"`. That collection is in-memory and doesn't
survive the daemon exiting, so this needs redoing per shell session — fine for one-off
verification, not something the app or `lifeagent-cli` needs to know or do anything about,
since the real target platform (Windows Credential Manager) has none of these issues.

## 2026-09-25 — M7.1 step 1: `CommandError` mapping + `CoreError::NotConfigured`

`docs/PHASE1_SPEC.md` §10.2 gives `CommandError { code, message }` with codes `not_configured`,
`auth`, `llm`, `invalid_input`, `not_found`, `internal`, and says `message` is "already in
zh-CN." Two things needed resolving before `apps/desktop/src-tauri/src/error.rs` could implement
`From<CoreError> for CommandError`.

**"Models not configured" needed its own `CoreError` variant.** `CoreApp::require_models`
(M3/M4) returned `CoreError::Validation("models are not configured yet")` — the only way to
recognize this specific case at the command boundary would have been matching on that message
string, which breaks silently if the text ever changes. Added `CoreError::NotConfigured` (no
payload) instead; `require_models` returns it directly, and it maps to `CommandError`'s
`not_configured` code with no string matching. The one existing test asserting
`Err(CoreError::Validation(_))` (`app.rs`) now asserts `Err(CoreError::NotConfigured)`.
`PluginError::from(CoreError)` (§7.7, M3) already has a catch-all arm, so the new variant falls
through to `PluginError::Other` there unchanged — a tool call can't run without models anyway.

**`invalid_input` gets a generic zh-CN message, not each `CoreError::Validation`'s own text.**
`CoreError::Validation` carries free-form English detail from a dozen call sites across
`profile.rs`, `goals.rs`, `memory.rs`, `settings.rs`, `plan.rs`, etc. (e.g. "height_cm must be
within 120..=230, got 119.9"). Translating each one properly needs either a structured
sub-code per validation rule or a message-keyed lookup table — real work, and exactly what §14
M8 already lists as its own milestone item ("Error messages in zh-CN for every `CommandError`").
For M7.1, every `CoreError::Validation` maps to one static zh-CN message
("输入内容有误，请检查后重试。"); the original English detail is kept at `tracing::debug!`, not
dropped, so it's still available for development. Revisit in M8 with per-rule messages once the
UI forms exist to make that detail actionable to a user (which field, which bound).

Also mapped: `LlmError::Auth` → `auth` (the variant already exists, so no HTTP-status sniffing
was needed); `LlmError::Config` and `CoreError::NotConfigured` both → `not_configured` (a router
built with no provider, and a provider missing its key, are both "finish configuring this"
states from the user's point of view); every other `CoreError`/`LlmError` variant → `internal`/
`llm` respectively, logged at `error!` (no personal data in any of them) since their detail
can't reach the user through a single generic message anyway.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass (287 tests: +9 for `error.rs`'s new mapping tests,
+0 net elsewhere since the one changed assertion is a rename, not an addition). `CommandError`
itself is `#[allow(dead_code)]` for now — nothing constructs it yet outside its own tests, since
`commands.rs` (the rest of M7.1) is what will actually call `.into()` on a `CoreError`. Remove
the allow once that lands.

## 2026-09-25 — M7 cross-cutting infra step 2: composition root; two real bugs found by smoke-testing it

`docs/PHASE1_SPEC.md` §10.1: build `CoreApp` (with `SystemClock`, `KeyringSecretStore`, the diet
plugin) in `setup()`, manage it as Tauri state, start the scheduler. Added
`apps/desktop/src-tauri/src/paths.rs` (mirrors `lifeagent-cli`'s `default_data_dir`) and wired
`lib.rs`'s `run()` accordingly. Smoke-tested against this machine's real data dir (from M6's own
manual verification — a real, working `models` config, saved before this session) rather than a
throwaway one, specifically to exercise a first launch under real conditions. That surfaced two
bugs, both fixed here rather than filed, since neither needed a judgment call:

**Bug 1: a stored `models` config that can't build failed `CoreAppBuilder::build()` outright.**
This machine's keyring session had already expired (see the M6 entry's WSL keyring workaround —
it doesn't survive a shell restart), so `LlmRouter::new` failed with `no API key set for
provider "anthropic"` even though a valid config was on disk. `build_router` (M3) only had two
cases, "config present, build it" and "no config, use the placeholder" — a config present but
unusable propagated its `LlmError` straight out of `build()`, which would crash the whole
desktop app before it could even reach a window, let alone Settings, to fix it. Added a third
outcome: a build failure on a *stored* config logs a warning and falls back to the same
no-providers placeholder as "never configured," so `models_configured` is `false` and the app
still starts. This is the direct, code-level counterpart to the M4 entry's "**Open for M7**"
note about `models_configured` existing so read paths don't have to assume a working router.

**Bug 2: `start_scheduler()` panicked when called after `tauri::async_runtime::block_on`
returned.** `CoreApp::start_scheduler` (M5) calls `tokio::spawn` directly, which needs an active
Tokio runtime context (a `Handle`) to find its reactor. Building `CoreApp` inside
`tauri::async_runtime::block_on(...)` provides that context for the duration of the `block_on`
call itself, but `setup()`'s own thread has none once `block_on` returns — calling
`start_scheduler()` there panicked with "there is no reactor running, must be called from the
context of a Tokio 1.x runtime." Fixed by moving the `start_scheduler()` call inside the same
`block_on` future as `CoreAppBuilder::build()`, so both `tokio::spawn` call sites (the event
dispatcher `build()` starts internally, and the scheduler loop) run while that context is live.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass (288 tests: +1 in `lifeagent-core` for Bug 1's
fallback case). Manually ran the real `lifeagent-desktop` binary (`GDK_BACKEND=x11`, no `--mock`,
this machine's actual data dir, `RUST_LOG=info`) for 8 seconds: no panic, the fallback warning
logged as expected, process still running when the timeout signaled it — confirms `setup()`
completes and the scheduler's loop is alive under real conditions, not just against a fresh
temp dir. No change to the user's stored `models` config or keyring state was made or needed.

## 2026-09-25 — M7 cross-cutting infra step 3: `AppEvent` → webview forwarding

`docs/PHASE1_SPEC.md` §10.1: forward `AppEvent`s as Tauri events `data-changed`, `plan-ready`,
`plan-failed`. The spec names the three event channels but not their JSON payload shape, so one
was picked directly from each `AppEvent` variant's own fields (`apps/desktop/src-tauri/src/
events.rs`):

- `data-changed`: `{ event_type, logical_date }`
- `plan-ready`: `{ date, version, trigger }`
- `plan-failed`: `{ date, error }`

`tauri::async_runtime::spawn(...)` is used for the forwarding task itself, not raw
`tokio::spawn` — unlike `tokio::spawn` (Bug 2 in the step-2 entry above), Tauri's own wrapper
spawns onto its globally-held runtime handle rather than reading a thread-local "current"
context, so it works correctly when called from `setup()`'s plain thread *after*
`tauri::async_runtime::block_on` has already returned (confirmed by the same manual 8-second run
as step 2: no panic, forwarder task alive).

**The event → payload mapping is a separate pure function (`payload_for`), not inlined in the
spawned loop.** The loop itself (subscribe, match, call Tauri's `emit`) needs a live `AppHandle`
and isn't practically unit-testable without Tauri's `test` feature and its `MockRuntime` — a real
webview is also not available in this Linux/WSL2 sandbox to verify against (see the M7_SCOPE.md
environment caveat). Pulling the actual mapping logic out into `payload_for(&AppEvent) ->
(&'static str, Value)` means the only real "logic" here — which channel name and which JSON keys
each variant produces — has direct unit test coverage (3 new tests in
`lifeagent-desktop`) instead of relying on "it compiles" for a change that's otherwise opaque to
automated verification.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass (291 tests: +3 for `payload_for`). Manually ran the
real binary again (same conditions as step 2): no panic, forwarder task starts cleanly. Full
end-to-end confirmation that an emitted event actually reaches a webview is deferred to M7.6
(needs a real window and at least one M7.1 command to trigger a state change from), same as the
tray/notification/autostart pieces already flagged as Windows-only manual verification.

## 2026-09-25 — M7 cross-cutting infra step 4: plan notifications; click-to-focus deferred

`docs/PHASE1_SPEC.md` §10.1: on `PlanReady` (trigger `schedule`) and `PlanFailed`, show a
Windows notification if `notifications_enabled`, titled "明日计划已生成"/"明日计划生成失败",
body = first sentence of the summary / a short error; clicking focuses the main window on
Today. Implemented in `apps/desktop/src-tauri/src/notify.rs` as its own subscriber task
(`core_app.subscribe()` again — a second, independent receiver on the same broadcast channel
events.rs already reads from), gated on `settings.notifications_enabled()`.

**Clicking to focus the window is not implemented.** `tauri-plugin-notification` 2.4's desktop
backend (`src/desktop.rs`) wraps `notify_rust`, whose builder exposes only
title/body/icon/sound — no activation/click callback. Reaching real click-through on Windows
needs the platform's own toast API (`winrt` `ToastNotificationManager`, handling
`ToastActivated` outside this plugin entirely), which is out of scope for this cross-cutting
step and, being Windows-only, isn't something this Linux/WSL2 sandbox could verify even if
implemented. Left as a known gap; revisit when M7.5's window-focus plumbing (tray's 打开, single
instance) exists to hang it off of, or in M8.

**`PlanFailed`'s notification body is a fixed zh-CN string, not `AppEvent::PlanFailed.error`
itself.** That field is `CoreError::to_string()` from the scheduler (`scheduler.rs`), i.e.
English, developer-facing text ("LLM error: authentication failed", etc.) — not something to
put in a user's toast. Same call as `error.rs`'s `CommandError` mapping (M7.1 step 1): a
generic zh-CN message now, the underlying detail already reaches a developer via the
scheduler's own `tracing::warn!`, and per-cause zh-CN text is M8 work.

**`PlanReady` only notifies for `trigger: Schedule`.** A manual or revision generation is
something the user just did themselves in the UI (§10.3's 立即生成/重新生成 buttons) and already
sees the result on screen; a toast for that would be redundant. This matches the cross-cutting
plan already written down before starting M7 (`docs/M7_SCOPE.md`).

The event→title/date mapping (`notification_target`) and the summary-truncation helper
(`first_sentence`) are pure functions with their own unit tests, for the same reason as step 3's
`payload_for`: the surrounding task needs a live `AppHandle` and a real OS notification backend
neither exist in this sandbox to verify against.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass (298 tests: +7 in `lifeagent-desktop`). Manually ran
the real binary again (same conditions as steps 2–3): no panic, all three background tasks
(scheduler, event forwarder, notification listener) start cleanly. Actually seeing a toast fire
needs a real `PlanReady`/`PlanFailed` event, which needs at least one M7.1 command (or a real
plan generation) to trigger — deferred to M7.6 like the rest of this sandbox's Windows-only
manual-verification items.

## 2026-09-25 — M7.1: `commands.rs` — all §10.2 commands, router reload, and a real ACL bug found

`docs/PHASE1_SPEC.md` §10.2: every command in its table, each a thin wrapper around an existing
or small new `CoreApp` method. Several things had to be resolved or built beyond "wire it up"
before the table's commands could actually work.

**Router reload — closing M4's "Open for M7" item.** `CoreApp` built its `LlmRouter` once, in
`CoreAppBuilder::build`, and had no way to rebuild it afterward. That made `save_models`/
`set_api_key` pointless on an already-running app: onboarding's step 3 saves a config and a key,
then immediately calls `test_model` — which would still hit the stale (usually unconfigured)
router built at startup, exactly the gap M4's entry flagged. Fixed in `crates/core/src/app.rs`:
- `CoreAppInner`'s `router: Arc<LlmRouter>` + `models_configured: bool` fields became one
  `router_state: std::sync::RwLock<Arc<RouterState>>` (a small struct pairing the two, swapped as
  one unit so a reader never sees a router paired with the wrong flag).
- `std::sync::RwLock`, not `tokio::sync::RwLock`: every hold is just an `Arc` clone with no
  `.await` inside it, so a blocking lock is simpler and correct — no need to make
  `plugin_context()`/`models_configured()`/`require_models()` async, which would have rippled
  into every call site (`scheduler.rs`, `lifeagent-cli`, the orchestrator loop).
- The non-override half of `CoreAppBuilder::build_router` (settings lookup → build → fallback to
  unconfigured on failure, from the step-2 entry above) was factored into a free function,
  `build_router_state`, so the new `pub async fn CoreApp::reload_models(&self)` can reuse it
  without needing a `CoreAppBuilder` (which only ever exists at startup) or its test/CLI-only
  `chat_model_override`.
- `models_configured()` changed from `pub(crate)` to `pub` (needed by `get_app_state`).
- `save_models` and `set_api_key` both call `reload_models()` after writing, unconditionally —
  it's a harmless no-op rebuild if nothing's actually changed for the router, and there's no
  reason to make the caller reason about which of the two onboarding might call first.

**A real Tauri ACL bug, found by testing commands through actual IPC rather than as plain Rust
calls.** Every other test in this workspace calls service methods directly; that proves
`CoreApp`'s logic is correct but not that a `#[tauri::command]` is reachable from a real webview
at all. Added `apps/desktop/src-tauri/tests/commands.rs`, using `tauri::test`'s mock runtime
(`tauri = { features = ["test"] }` as a **dev**-dependency, unioned with the main dependency's
`tray-icon` feature for test builds only) to send real `InvokeRequest`s through the real
`invoke_handler` and get real JSON back. Every single command it tried came back
`"<command> not allowed. Plugin not found"` — Tauri 2's permission system denies every app-level
command by default, and `capabilities/default.json` (from M0's scaffold) only granted
`core:default`/`notification:default`/`autostart:default`, never anything for the app's own
commands. **This means the real production app was equally broken**: the frontend could not
have called a single command in `commands.rs`, onboarding included, until this was fixed — not a
test-only artifact.

Fixed with the mechanism `tauri-build` provides for exactly this (`tauri_build::AppManifest::
commands(&[...])`, which autogenerates an `allow-<command>`/`deny-<command>` permission pair per
name, in kebab-case, e.g. `allow-get-app-state` for `get_app_state`):
- `build.rs` now lists every command name in a `COMMANDS` const and passes it through
  `Attributes::new().app_manifest(AppManifest::new().commands(COMMANDS))`. A doc comment on the
  const says to keep it in sync with `commands.rs` — there's no way to derive one from the other
  automatically, and a command left out here is silently unreachable (denied), not a compile
  error.
- `capabilities/default.json` grants each generated `allow-<command>` permission.
- The test itself needed `tauri::generate_context!()` (the project's real `tauri.conf.json` +
  `capabilities/`), not `tauri::test::mock_context`'s bare-bones one — permission enforcement is
  identical regardless of runtime, so a mock context with no capabilities proves nothing (every
  command would fail the same "Plugin not found" way for a different, uninteresting reason: no
  ACL manifest registered *at all*, vs. one that's registered but doesn't grant the command).
- The request's `url` must be `"tauri://localhost"`, not `"http://tauri.localhost"` (the doc
  example's "alternatively" option) — the latter resolves to `Origin::Remote`, which
  `capabilities/default.json`'s default `ExecutionContext::Local` doesn't match, so every command
  was still denied even after the permissions existed, just with a different, more specific
  error naming the capability that almost-but-didn't match.

**`get_history`'s `limit` needed a new store method, not a post-filter.** Filtering
`ConversationStore::recent(limit)`'s result down to non-tool messages *after* the query would
make `limit` mean "however many visible messages happen to survive whatever the raw row count
was," shrinking unpredictably whenever tool messages were interleaved. Added
`ConversationStore::recent_visible(limit)`, filtering `role != 'tool'` inside the same
ROWID-windowed subquery `recent` already uses, so `limit` means what the command's caller expects
it to mean.

**`save_settings` needed one atomic setter, not two.** `Settings::set_plan_time` and
`set_day_boundary` (§7.2, B3) each validate against the *other's currently-stored* value — fine
for changing one at a time, but a caller changing both together (exactly what a Settings form
does) hits an order dependency: moving both several hours earlier (e.g. 21:00/04:00 →
03:00/02:00) is a valid final pair, but calling `set_plan_time` first is rejected against the
still-04:00 stored boundary. Added `Settings::set_plan_time_and_day_boundary(plan_time,
day_boundary, now)`, validating the new pair directly against each other, with no dependency on
whatever was stored before. `save_settings` uses this instead of the two single-field setters.

**`SecretStore` gained a default-implemented `has()`.** `has_api_key` needs presence, not the key
itself — `has(&self) -> Result<bool, CoreError> { Ok(self.get(id)?.is_some()) }` as a trait
default means neither `KeyringSecretStore` nor `MemorySecretStore` needed changing, and the key
value never has to flow through a caller that only wanted a yes/no.

**`preview_targets`'s and settings' JSON boundaries.** `ChatTurn` and `MemoryNote` gained
`Serialize` (needed to cross the command boundary; neither had it before since `send_message`'s
and `memory().list()`'s only prior callers were Rust code). `RoleUsage` (new, in `usage.rs`
alongside a new `summary(db, since)` query function) mirrors `llm_usage`'s role/token columns;
`CoreApp::usage_summary()` computes "current calendar month" from `Clock::now()`'s UTC month —
a simplification worth revisiting only if a non-UTC user actually notices the boundary landing a
few hours off their own midnight. `SettingsDto`/`AppState`/`TestModelResult` are desktop-local
DTOs (not added to `core`'s public surface), since they're UI-shaped groupings with no other
caller, unlike `Profile`/`WeightGoal`/`StoredPlan`/etc., which are reused directly.

**`test_model` bypasses the orchestrator entirely** — one `RoleRequest` with a fixed "Reply with
OK." message, no history, no tools, nothing persisted — since it's a connectivity check, not a
conversation turn. Its `error` field reuses `CommandError`'s own `LlmError` mapping (step 1) for
the message text, rather than duplicating that logic.

**`autostart`'s OS-level registration is explicitly out of scope here.** `save_settings` persists
the `autostart` boolean; actually calling `tauri_plugin_autostart`'s enable/disable needs an
`AppHandle`-level autostart manager, which belongs with M7.5's tray work (per `docs/M7_SCOPE.md`),
not this step.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass — 312 tests total (up from 297 before this step):
+10 in `crates/core` (`reload_models` ×2, `usage::summary` ×3, `recent_visible` ×2,
`set_plan_time_and_day_boundary` ×2, `has` ×1), +5 real-IPC tests in
`apps/desktop/src-tauri/tests/commands.rs`. Manually ran the real `lifeagent-desktop` binary
again against this machine's actual data dir: no panic, ran the full timeout window.

## 2026-09-25 — M7.2: Onboarding UI; a real minifier bug found and fixed

`docs/PHASE1_SPEC.md` §10.3: three-step blocking onboarding (基本信息 / 目标 / 模型), gated on
`get_app_state().onboarding_complete`. Built the whole frontend layer for it — `App.tsx` routing,
`api/types.ts` + `api/commands.ts` (typed `invoke()` wrappers), `onboarding/` (wizard container
+ three step components), and the zh-CN strings it needs, per CLAUDE.md's thin-view-layer rule
(no business logic beyond what the components need to hold as they collect input).

**A real production bug, again found by actually building rather than trusting `tsc`.**
`npm run build` (used by `beforeBuildCommand`, i.e. every real release build) failed with
esbuild's minifier erroring "Transforming destructuring to the configured target environment...
is not supported yet" on ordinary code like `const [preview, setPreview] = useState(null)` — the
first real destructuring this project's frontend has ever had, since the M0 placeholder page used
none. Bisected by toggling `minify: false` (what `TAURI_ENV_DEBUG` selects): the build succeeded
without minification, isolating the bug to Vite 8's default bundler (`rolldown`, evidenced by the
stack trace) combined with the `safari13` target the M0 scaffold's `vite.config.ts` carried over
from Tauri's generic project template. Fixed by dropping `safari13` from the build target — this
project targets Windows only (CLAUDE.md's own first line), so macOS/Safari compatibility was
never actually a requirement here, unlike a template meant to cover every Tauri platform.
Verified: `npm run build` (real, minified) now produces a working bundle, `600KB → 240KB`
minified confirming minification is actually running, not just not-crashing.

**Layout bug caught by screenshotting the running app, not just reading the code.** The
onboarding container centered its content vertically (`justify-center`) inside the fixed
1100×720 window. Step 1's form is taller than that, so with no scroll, the step indicator and the
下一步/上一步 buttons were pushed off-screen entirely — the user would have had no way to
proceed past onboarding at all. Fixed by restructuring to a flex column: a scrollable content
area above a fixed footer holding the navigation buttons, so the buttons stay reachable
regardless of how tall a given step's form is.

**Verification without a way to click the real window.** This sandbox has no `xdotool`/
`ydotool`/equivalent installed, and installing one needs a password this session doesn't have —
so the golden path (fill in three steps, finish) could not be driven through the actual UI.
Compensated with: (1) screenshotting the running app at each step by temporarily forcing the
wizard's initial `step` state and `sameModelForAll` default, confirming all three steps and both
the shared-model and per-role-model layouts render correctly with the right zh-CN labels, then
reverting those temporary edits exactly; (2) extending `tests/commands.rs` with real-IPC tests
for every onboarding command that didn't already have one (`save_weight_goal`/`get_weight_goal`
round-trip, `complete_onboarding` flipping `get_app_state`, `preview_targets` against a valid
profile, and `test_model` resolving `{ ok: false, error: Some(...) }` rather than rejecting when
nothing is configured — the case onboarding's own connection test hits before anything is saved).
`test_model` is deliberately not exercised against a real provider in any automated test
(CLAUDE.md: never call a real LLM in tests); that stays a manual, real-network check, same as
M6's own real-API-key verification.

**`test_model` is called for all three roles on every 测试连接 click, not deduped by
provider+model.** When `sameModelForAll` is on (the default), the three calls are identical
requests; the redundancy costs one connectivity check's worth of extra latency in exchange for
not having to reason about which (role, model) pairs are actually distinct. §10.3's own
suggested role defaults (opus/sonnet/haiku for planner/conversation/parser) mean a user who
unchecks "same model for all" is exactly the case where testing all three actually matters.

**`ModelsConfig`'s `ProviderKind::OpenAiCompatible` serializes as `"open_ai_compatible"`, not
`"openai_compatible"`.** Verified directly (a throwaway `serde_json::to_string` check), not
guessed from the natural English spelling — serde's snake_case rename splits at every capital,
so "OpenAiCompatible" splits into "Open"/"Ai"/"Compatible". Documented inline in `api/types.ts`
so nobody re-guesses it wrong later.

**Everything is persisted only at the very end (完成), except the model config being tested,
which is saved as part of 测试连接 itself.** `save_profile`/`log_weight`/`save_weight_goal`/
`complete_onboarding` all fire from `handleFinish`, since none of the earlier steps need their
data durable yet — `preview_targets` and `test_model` both work against in-memory draft state or
a just-saved candidate config, not anything persisted early. A "tested successfully" flag is
invalidated (`modelSignature`, a JSON snapshot of every model-related field) whenever the model
step's fields change after a successful test, so editing a model name after testing correctly
re-disables 完成 until 测试连接 is run again against the new values.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass — 316 tests (+4 real-IPC tests). `npm run build`
(`tsc -b && vite build`) passes clean, strict mode, no unused locals/params. Screenshotted all
three onboarding steps (both model-step layouts) against the running app via `npm run tauri dev`;
confirmed the layout fix by comparing before/after screenshots.

## 2026-09-26 — M7.3: Today + Chat pages

`docs/PHASE1_SPEC.md` §10.3: main shell (sidebar + 今日/对话/设置 tabs) and the Today and Chat
pages. Full breakdown in `docs/M7_SCOPE.md`'s M7.3 entry; the decisions worth recording here:

**Activity chips are matched to a rendered message by content equality, not by index or id.**
`get_history` doesn't reconstruct `activities` for past turns (M7.1's own decision), so `Chat`
keeps the last live `send_message` result's `{content, activities}` separately and only shows
the chips on a rendered assistant message when that message's `content` equals the live reply's
`content` *and* it's the last message in the list. This is simple and correct for the normal
case (one new assistant reply at a time) and degrades safely (chips just don't show) if two
consecutive replies were ever byte-identical, which never happens in practice for this app's
LLM-generated prose.

**No browser `confirm()` for meal deletion.** §10.3 asks for "delete button per meal with
confirm" without specifying the mechanism; a native `confirm()` popup would break the app's own
styling and dark-mode support. Used a two-step in-place button instead (tap once to arm a
red 确认删除/取消 pair, tap again to actually delete) — same pattern as GitHub's own inline
delete confirmations, no modal/dialog component needed.

**Tomorrow's no-plan-yet message reads `plan_time` from `get_settings` live**, rather than
hardcoding §10.3's own example "21:00" — that string is the *default* `plan_time`, not a
constant, and a user who changes it in Settings (M7.4) should see the real value here.

**Kcal ring and macro bars use fixed, non-cycled hues** (sky for the ring; indigo/amber/violet
for protein/carbs/fat, always in that order) per the dataviz skill's categorical-color rule,
with the number always shown as text alongside the color so nothing is color-only. Skipped the
skill's palette validator script (`scripts/validate_palette.js`) for this one, since these are
small in-app meters reusing the app's own already-established Tailwind neutral/emerald/red
system, not a new standalone palette being shipped — worth revisiting with the validator if
Settings (M7.4) or later work introduces more chart-like surfaces.

**Verification without click automation, again.** Same sandbox gap as M7.2 (no `xdotool`/
equivalent, no way to install one without a password). Unlike M7.2, this machine's actual data
dir (`~/.local/share/lifeagent`, from earlier M7.1/M7.2 manual runs) already has a real profile,
a logged weight, and prior chat history in it — so rather than needing to click through
onboarding, `App`'s `screen`/tab state was temporarily forced (`useState` initial value edited
directly, the onboarding-gate `useEffect` temporarily no-op'd) to land straight on each page,
screenshotted with `GDK_BACKEND=x11 npm run tauri dev` + `import -window <id>` (M0's own
recipe), then the edit was reverted and confirmed clean with `git diff --stat`. This exercised
real backend data end-to-end (computed nutrition targets, real conversation history rendering,
correct button disabled-states) without ever calling `send_message`/`generate_plan` against the
real configured provider, which this check had no reason to trigger. A real click-through
(actually sending a message, generating/accepting a plan) still needs a real environment with
input automation or a human at the keyboard — same caveat M7.2 recorded, now also covering M7.3.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass (no Rust changed — M7.1 already exposed every
command this milestone needed). `npm run build` (`tsc -b && vite build`) passes clean, strict
mode. Screenshotted Today (kcal ring, macro bars, empty meal list, no-plan-yet state) and Chat
(real prior conversation, correctly disabled send button) against the real running app.

## 2026-09-26 — M7.4: Settings page

`docs/PHASE1_SPEC.md` §10.3's Settings section. Full breakdown in `docs/M7_SCOPE.md`'s M7.4
entry; the decisions worth recording here:

**A dedicated `ModelDraft` type for Settings, not a reused `OnboardingDraft`.** Onboarding's
`StepModel` always shows a blank API key field and gates a "finish" action on a fresh successful
test — neither applies once a key is already stored. Rather than bend `OnboardingDraft`/
`StepModel` to cover both a blank-slate wizard and an edit-existing-config settings page, Settings
got its own small `ModelDraft` (`settings/ModelSection.tsx`) with a `newApiKey` field that means
"replace with this" when non-empty and "keep what's stored" when blank, plus a `has_api_key`-
driven status hint instead of ever trying to display or pre-fill a real key.

**Plan time and general toggles share one save, not two.** §10.3 lists 计划 and 通用 as separate
bullets, but `SettingsDto`/`save_settings` (M7.1) already persist all four fields — plan time,
day boundary, notifications, autostart — as one atomic write with no partial-update variant.
Two separately-saved forms would only create a false impression that saving one leaves the other
untouched, when in fact either save round-trips (and re-persists) all four. Built as one
`GeneralSection` instead.

**`ConfirmDeleteButton` extracted to `components/`.** M7.3's `MealList` already had a two-step
(tap-to-arm) delete confirmation; `MemorySection` needed the exact same behavior for memory
notes. Pulled it out to `components/ConfirmDeleteButton.tsx` and switched `MealList` over to it
rather than duplicating the same ~25 lines a second time — a legitimate shared component (used
identically in two places now), not a premature abstraction for a single call site.

**Screenshot verification without working scroll, again a sandbox gap.** Programmatically
setting a scrollable container's `scrollTop` from a `useEffect` (both immediately and via a
repeating `setInterval`) had no visible effect in this environment's WebKitGTK webview, even
though Vite's HMR log confirmed the edits were applied each time (`hmr update
/src/settings/Settings.tsx`) and the *previous* two milestones' state-forcing technique (editing
a `useState` initial value) worked fine in the same session. Root cause not chased down (likely
some interaction between this WebKitGTK build, WSLg, and imperative scroll — not worth solving
for a one-off manual check when a working alternative existed). Worked around by temporarily
applying a CSS `transform: scale(0.25)` (with a matching `width: 400%`, since `transform` is a
paint-only operation that doesn't affect layout/overflow) to Settings' content wrapper, which
fit the entire page — all four sections — into one screenshot without needing to scroll at all.
Reverted afterward; confirmed clean with `grep -rn "TEMP" apps/desktop/ui/src`.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass (no Rust changed). `npm run build` (`tsc -b && vite
build`) passes clean, strict mode. Screenshotted the full Settings page (scaled to fit) against
the real running app with this machine's real, already-configured data — every section showed
correct real data: profile/goal fields, a goal preview matching Today's own kcal ring exactly,
model config, a non-empty 本月用量 table from real prior API usage, plan/general settings, and
one real memory note.

## 2026-09-26 — M7.5: Shell chrome (tray, single instance, autostart, quick-entry)

`docs/PHASE1_SPEC.md` §10.1/§10.3's remaining desktop-shell pieces. Full breakdown in
`docs/M7_SCOPE.md`'s M7.5 entry; the decisions worth recording here:

**Quick-entry routes by window label, not a URL/hash.** The spec calls for the quick-entry
popup to have "its own tiny route," which suggested a client-side router at first read. This
app has none (`App.tsx` is state-driven, no `react-router`), and adding one for a single second
screen would be the kind of dependency CLAUDE.md's "keep dependencies minimal" rule argues
against. `@tauri-apps/api/window`'s `getCurrentWindow()` already resolves synchronously to
metadata Tauri injects at page load (no IPC round-trip needed), so `main.tsx` just picks
`<QuickEntry />` vs `<App />` based on `getCurrentWindow().label === "quick-entry"`. Both windows
load the exact same `index.html` (`WebviewUrl::App("index.html".into())`, same as the main
window) — nothing about the served page itself differs, only what mounts.

**Quick-entry gets its own capabilities file, scoped to just `allow-send-message`.** The main
window's `capabilities/default.json` grants every command (profile, goals, models, memory,
usage, plan actions, ...) — appropriate for the full app shell, wildly excessive for a popup
whose only job is "type a message, send it." A second capability file
(`capabilities/quick-entry.json`, `"windows": ["quick-entry"]`) keeps it to `core:default` +
`allow-send-message`, per §12's least-privilege principle already established for `default.json`
itself (see M7.1's entry on `capabilities/default.json` needing explicit grants at all).

**`generate_plan`'s tray action calls `CoreApp` directly, not through a synthetic IPC call.**
The 生成明日计划 menu item's click handler already has the real `CoreApp` (cloned into the tray's
`on_menu_event` closure at `tray::build` time) and runs on the backend, not in the webview — so
there's no reason to round-trip through `commands::generate_plan` and its `CommandError`
mapping, which exists specifically for the boundary a real frontend `invoke()` call crosses.
Made `commands::resolve_plan_date` `pub(crate)` instead of duplicating its "no date -> tomorrow"
logic, so both paths pick the same date without copy-pasting the one-line `succ_opt()` call.

**Autostart sync is deliberately untested by automated tests, unlike almost everything else in
this codebase.** `tauri_plugin_autostart`'s `AutoLaunchManager::enable`/`disable` write real
OS-level state (a Windows registry run key, an XDG `.desktop` file, a macOS launch agent) —
calling either for real inside `cargo test` would leave side effects on whatever machine runs
the suite, CI included, which is a materially different risk than this project's existing
"never call a real LLM in tests" rule (CLAUDE.md) is even about, but the same *spirit*: don't let
an automated test run touch anything real outside its own temp dir. `apps/desktop/src-tauri/
tests/commands.rs`'s mock builder doesn't register the autostart plugin either (confirmed by
reading `setup()` — it never calls `.plugin(tauri_plugin_autostart::init(...))`), so an IPC test
that invoked `save_settings` today would panic on `state() called before manage()` for
`AutoLaunchManager`; deliberately did not add such a test rather than register the plugin in the
mock builder just to make one panic-free while still executing a real `enable`/`disable` in the
test process. `save_settings` joins several other commands (`get_models`/`save_models`,
`get_memory`/`delete_memory`, `get_usage_summary`, the plan commands) that `tests/commands.rs`'s
mock builder already doesn't register at all — this is consistent with, not a new gap beyond,
that existing pattern.

**Tauri 2 APIs were verified against the actual installed crate sources before writing any
code**, not from memory or general Tauri familiarity — `~/.cargo/registry/src/.../tauri-2.11.6`
and `tauri-plugin-autostart-2.5.1` were grepped directly for `MenuItem::with_id`,
`TrayIconBuilder`'s builder methods, `WindowEvent::CloseRequested`'s `CloseRequestApi::
prevent_close`, `WebviewWindowBuilder`'s window-option methods, `WebviewUrl::App`'s `PathBuf`
shape, and `ManagerExt::autolaunch`'s exact return type, before either was used. Everything
compiled clean on the first attempt as a result.

**Verification without click automation, a third time.** Same sandbox gap as M7.2–M7.4 (no
`xdotool`/equivalent). Tray menu clicks in particular are a harder case than the webview-content
state-forcing trick used for Today/Chat/Settings, since a tray menu is an OS-native widget with
no React state to force — there is no way to exercise 打开/快速记录/生成明日计划/退出 by editing
source in this sandbox. Compensated by: (1) confirming via `xwininfo -root -tree` that a real
tray-icon proxy window (`"tray-icon tray app <pid>-1"`) exists after startup, proving
`TrayIconBuilder::build` actually succeeded rather than erroring silently; (2) temporarily making
`tray::open_quick_entry` `pub` and calling it once from `setup()` to screenshot the quick-entry
window's real rendered content, then reverting (confirmed via `grep -rn "TEMP"`); (3) reading
Tauri's own source for the parts that can't be exercised at all here (close-to-tray's
`prevent_close`, the single-instance callback, the autostart sync) rather than trusting a mental
model of how they behave. Real click-through verification (actually closing the main window and
confirming the process survives in the tray, launching a second instance, sending a message
through quick-entry end-to-end) needs a real Windows environment — the same caveat M7.2's
onboarding screenshots and M7.3/M7.4's page checks already carry, now covering M7.5 too, and
overlapping with M7.6's own full acceptance pass.

Verification: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` all pass, no new failures. `npm run build` (`tsc -b && vite
build`) passes clean, strict mode. Ran the real app; no panic or logged error from
`tray::build`, the single-instance plugin, or the startup autostart sync; confirmed the tray
icon's proxy window and the quick-entry window's correct rendering as described above.

## 2026-09-26 — M7 review fixes

A review of all M7 work (everything checkable without a real Windows environment) found seven
issues; all fixed. The decisions worth recording:

**Quick-entry needs `core:window:allow-hide`.** `core:default`'s window set is read-only
(`allow-is-visible`, `allow-title`, ...), so the popup's own `getCurrentWindow().hide()` (Esc,
and after a reply) was denied by the ACL and swallowed by `void` — an undecorated,
taskbar-less window with no way to dismiss it but Alt+F4. Added the one permission; a real-IPC
test (`quick_entry_window_can_hide_itself_but_not_reach_data_commands`) was confirmed to fail
without it ("window.hide not allowed") and pass with it.

**`send_message` is retry-safe instead of the Chat page deduplicating.** The user message is
stored before the first model call, so a failed turn leaves it behind; resending the same text
used to store it twice. Now, if the latest user message has no final (tool-call-free) assistant
reply after it and `text` equals it, `send_message` resumes that turn rather than appending
again (`ConversationStore::unfinished_user_message`). Done in core rather than as a new
`retry_message` command so the §10.2 table stays as specified and quick-entry/CLI get the same
behaviour. Any tool results the failed turn already stored stay in the history window, so the
model sees e.g. a meal it already logged instead of logging it again. Chat now resyncs via
`get_history` on failure instead of dropping its optimistic bubble (which could drop the wrong
message if an event-triggered refetch had already replaced the list).

**Tray 生成明日计划 always toasts its result**, ignoring `notifications_enabled`: it's the
direct response to the user's own click (with the main window usually hidden), not an
unsolicited notification. On failure, the body is the `CommandError`'s zh-CN message, so
"not configured" and "complete your profile first" are distinguishable.

**Desktop logging (§12)** goes to `data_dir/logs/lifeagent.YYYY-MM-DD.log` (daily, 14 kept) plus
stdout, INFO by default, `LIFEAGENT_LOG` overriding. It uses the rolling appender as a blocking
writer, not `non_blocking`: Tauri's exit path never unwinds `run()`, so a `WorkerGuard` would
never flush the last buffered lines. If the data dir can't be resolved, logging falls back to
stdout and `setup()` still fails as before.

**Settings shows the §12 "what is sent to cloud models" paragraph** at the top of 模型.
**Settings sections show their load error** instead of 加载中… forever (shared
`settings/LoadState.tsx`), and 模型 falls back to onboarding's Anthropic defaults when no config is
stored instead of never rendering. **Commands take "now" from `CoreApp::clock()`** (made `pub`)
rather than `Utc::now()`.

Verification: fmt/clippy/`cargo test --workspace` clean (319 tests, +3); `npm run build` clean.
Ran the real binary against a fresh throwaway data dir (`XDG_DATA_HOME`, so no models and no LLM
calls): the log file is created and written (checked with `LIFEAGENT_LOG=debug`), with no ANSI
codes and no first-run appender error.

## 2026-09-26 — M7 review: smaller fixes

The review's remaining smaller items, plus one more serious problem found while fixing them:

**The desktop IPC tests were writing to the real OS keyring.** `tests/commands.rs`'s
`core_app()` never overrode `CoreAppBuilder`'s default `KeyringSecretStore`, so
`set_api_key_then_has_api_key_round_trips_with_camel_case_args` stored `sk-test` under
`LifeAgent / provider:anthropic` in the developer's real credential store on every
`cargo test --workspace` — overwriting a real Anthropic key configured under that same provider
ID. The harness now always uses `MemorySecretStore`, like every other `CoreAppBuilder` in the
workspace's tests already did (checked all of them). A key overwritten this way has to be
re-entered (Settings → 模型, or `lifeagent-cli models set-key anthropic`).

**Settings → 模型 got onboarding's guards.** Save/测试 are disabled (with a hint) unless provider
ID, base URL and model name(s) are filled and there's a key to use — stored for this provider ID
(re-checked via `has_api_key` whenever the ID changes, since keys are stored per ID) or newly
typed. Toggling "same model for all" carries the current value(s) across so a blank model is
never saved for every role. Changing provider kind applies the same prefill as onboarding
(`prefillFor`, now exported from `StepModel.tsx`).

**Quick-entry**: the post-reply hide timer is cancelled by typing or a new submit (it used to
hide the window mid-sentence), hiding resets the reply/error so a reopened window is clean, and
the input is refocused whenever the reused window regains focus (`autoFocus` only fires on the
first open).

**Onboarding 完成 retry** no longer logs the same weight twice when a later step
(`save_weight_goal`) failed on the first attempt.

**Not changed — `PlanFailed` notifications.** The review suspected a failed manual 立即生成
would also toast. It can't: only the scheduler publishes `PlanFailed`; a manual failure is
returned to its caller. Documented on `notify::notification_target`.

Also: `tauri-plugin-single-instance` is registered first, per its docs; the weight dialog says
"enter a weight > 0" instead of "required" for invalid input; stale comments fixed (`error.rs`'s
leftover `allow(dead_code)`, `commands.rs`/`lib.rs`/`App.tsx`/`api/commands.ts` module docs,
`M7_SCOPE.md`'s already-resolved open decision). Five new real-IPC tests cover `get_history`, the
plan commands' error codes, `get_settings`, `get_models`/`save_models` (including the live
router reload) and the memory/usage commands, with `send_message` driven by `MockChatModel`.
`save_settings` stays untested over IPC on purpose (real OS autostart, see M7.5).

Verification: fmt/clippy/`cargo test --workspace` clean (324 tests); `npm run build` clean.

## 2026-09-26 — M7.6 step 1 (Windows onboarding) fixes

Found during the first real Windows acceptance run, onboarding step:

**Anthropic `effort` is dropped for models that reject it.** With onboarding's suggested
per-role models, 测试连接 failed with `400 "This model does not support the effort
parameter."`: §6.3 pairs the parser role with `effort: low`, but §10.3 suggests
`claude-haiku-4-5` for that role, and Haiku 4.5 rejects `output_config.effort`. It passed with
`claude-sonnet-5` for all roles only because Sonnet 5 accepts it. The provider now omits
`output_config` for a denylist of model-ID prefixes (`claude-haiku*`, `claude-3*`, Sonnet ≤ 4.5,
Opus ≤ 4.1) rather than an allowlist, so newer models keep getting the per-role effort.
Done in the provider rather than the router because it's an Anthropic-API fact, like "no
`temperature`". Haiku therefore runs the parser at its own default (no thinking), which is
what "low" was for anyway.

**Autostart sync no longer errors on every launch.** Startup reconciliation called
`disable()` whenever the setting was off, and on Windows disabling a never-registered entry
fails with "file not found" (os error 2), logged at ERROR on every launch. `sync_autostart`
now checks `is_enabled()` first and only changes a registration that differs; startup reuses
the same function instead of its own copy.

**Onboarding 测试连接 shows per-role latency and names the failing role**, e.g.
"连接成功（解析 320ms · 规划 910ms · 对话 450ms）" / "连接失败：解析：…". The failure message also
lost a stray "Error: " prefix (an `Error` object was being stringified).

**LLM calls are now logged at INFO (§12).** The Windows run's log file was empty after a
successful onboarding: nothing in `llm`/`core` logged at INFO, so §12's "log each LLM call at
INFO with role, model, tokens, latency, ok" was never implemented. `LlmRouter::complete` now
emits one `llm call` line with exactly those fields (plus provider ID) — no content — next to
the existing `UsageSink` record, so it covers every caller (orchestrator, planner, parser,
`test_model`).

## 2026-09-26 — M7.6 steps 2–4 (Windows chat) fixes

Steps 2–4 stored the right data (one breakfast, one lunch, the correction soft-deleted and
re-logged), but two replies/parses were wrong:

**The system prompt is built once per turn, not per tool-loop iteration.** After
`diet_log_meal` succeeded, the model replied "看到这份早餐已经记录过了" — "already recorded".
`send_message` rebuilt the system prompt on every iteration, so the second call's "Today's
status" already listed the meal the first call had just logged, and the model read the user's
message as a repeat. §7.11's pseudocode doesn't say which; building it once means the status
describes the day as it was when the user spoke, and tool results carry the updated totals
(they already return `day_total`/`remaining`). It also keeps the prompt prefix stable across
iterations. Regression test: `system_prompt_is_not_rebuilt_after_a_tool_call_within_a_turn`
(fails without the fix).

**The meal parser prompt (§9.2) now gives a concrete decomposition example.** Haiku 4.5 returned
"牛肉拉面(熟)" as a single 300 g item despite "split composite dishes" — on a fresh database
KNOWN_FOODS is empty, so nothing anchored it. §8.4 already names this exact example
("牛肉拉面" → noodles, beef, broth, vegetables); it's now in the prompt itself, plus a
stir-fry one, and "a dish name is not an item by itself". Once a whole-dish food exists in the
table, §8.4 deliberately keeps reusing it, so a database that already learned "牛肉拉面(熟)" has
to be reset (or the food deleted) to re-test this.

## 2026-09-26 — M7.6 queued fixes (found during the Windows acceptance run)

All three are prompt/context changes; none touch validation or the numbers engine. Prompt
texts changed in code and in the spec's verbatim copies (§8.5, §9.1–§9.3, §9.4 planner header).

**Corrections keep unchanged items' grams.** Re-logging after "卤蛋其实是两个" re-parsed the whole
meal from text, so Haiku re-guessed every portion (beef 80→50 g, broth 400→300 g) and lunch
*fell* 662→617 kcal after adding an egg. The system prompt's correction rule and the
`diet_log_meal` description now tell the conversation model to describe the corrected meal item
by item, carrying the earlier tool result's names and grams for everything the user didn't
correct — the one exception to "pass the user's original wording". The parser prompt gained
"use grams exactly as stated" so those carried-over grams stick. Rejected: a dedicated
`diet_correct_meal` tool editing the stored items directly — more exact, but new tool surface
and schema beyond §8.5 for what the model already does correctly when told.

**Food names say dry/raw vs cooked when it matters.** "燕麦 40g" (a dry portion) was resolved as
cooked oats at 68 kcal/100 g — 27 kcal instead of ~150. The parser must now name the state the
grams refer to ("燕麦片(干)" vs "燕麦粥", "大米(生)" vs "熟米饭"), and the food-estimate prompt
uses the state the name gives (干/生/熟) instead of defaulting to "as normally eaten".

**Plan numbers come from the app, not the model.** Plan summaries stated "约2160千卡、蛋白质135克"
(the targets, not the computed 2024/2135 kcal), and the step-7 chat reply repeated it — against
CLAUDE.md's "numbers shown to the user are computed in Rust". The planner prompt now forbids
nutrition figures in `summary`, and only lets it reference recent adherence when the brief has
data (v1 had claimed "和前几天不重样" with no history). `render_plan_compact` — the plan text in
the system prompt and in `core_get_plan`/`core_revise_plan` results — now carries the
validator-computed `total.kcal` per meal and a "day total (computed by the app)" line, so the
conversation model has real numbers to quote. Core still knows no section schema: it reads
`total.kcal` only when present.

Not changed: revision latency (85 s on Opus 5 at `high` effort). It's the planner's quality
lever per §6.3; revisit with real usage data, not from one run.
