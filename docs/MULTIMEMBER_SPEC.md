# Multimember plugin — specification

Status: approved design, not yet implemented (2026-10-04). Decisions behind it are recorded in
`docs/DECISIONS.md`, "Multimember plugin".

This document follows the conventions of `docs/PHASE1_SPEC.md`: it is the source of truth for
the plugin's behavior, `CLAUDE.md` wins for conventions, and milestones (§10) are implemented in
order, each closed only when its "done when" criteria hold.

## 1. Purpose and scope

The owner of an account cooks for other people (a partner, flatmates, adult children). The
`multimember` plugin lets the owner describe those people and who eats which meal, so the daily
diet plan becomes **one set of shared dishes** that:

- meets the owner's own nutrition targets, exactly as today;
- is safe for everyone at the table (no attendee's allergens) and respects everyone's dietary
  pattern and dislikes;
- comes with a portion size for each other attendee that keeps that person at their
  **maintenance** energy, computed in Rust.

### In scope

- Profiles for other household members (up to 7 besides the owner).
- Weekly default attendance per member and meal, plus one-off overrides per date.
- Per-member dislikes.
- A computed `multimember` plan section, and diet plan integration (constraints, servings,
  per-member portion totals).
- Tools, API commands and a web UI tab to manage the above.

### Out of scope

- Other members using the app, logging in, or chatting. **Only the owner interacts with the
  app**, and the chat always speaks as the owner.
- Logging meals, weights or anything else for other members. There are no events, goals,
  progress or history for them.
- Goals for other members (lose / gain). Every other member is planned for **maintenance and a
  balanced, healthy meal**.
- Members under 16 or over 70. Nutrition for children and elderly people needs different
  references, which this app doesn't have.
- Member-only dishes or extras (e.g. an extra bowl of rice for one person). Everyone at a meal
  eats the same dishes; only portion size differs (§6.4).
- Meals the owner doesn't eat. The diet plan always has the owner's three main meals; others
  can only join those.

## 2. Terms

| Term | Meaning |
|---|---|
| Owner | The account holder. Their profile, goals, weight and preferences stay in core, unchanged. |
| Member | Another person in the household, stored by this plugin. Never the owner. |
| Attendee / guest | A member who eats a given main meal on a given date. |
| Main meal | `breakfast`, `lunch` or `dinner`. Snacks are always the owner's alone. |
| Maintenance estimate | A member's daily kcal and macros from the diet targets engine with no goal (Maintain). Labelled 目安 / estimate in the UI. |
| Scale | A guest's portion as a multiple of the owner's portion of the same meal. |

## 3. Architecture

- New crate `crates/plugin-multimember` (`lifeagent-plugin-multimember`), plugin id
  `multimember`, so tools are `multimember_*` and tables `multimember_*`. Depends on
  `lifeagent-core` and `lifeagent-llm` only.
- **The core schema does not change.** Owner data stays in `profile`, `goals`, `events` and
  `preferences`. Training and stock tables don't change.
- The plugin and diet never depend on each other. They share data only through the
  `multimember` plan section (§6), the way diet already reads `training`.
- The nutrition maths stays in diet: multimember passes guest profiles in its section, and diet
  turns them into maintenance estimates with its own `targets::compute`.
- Registered in both composition roots (`cli`, `server`). `optional() = true`,
  `enabled_by_default() = false`. While it is off, the app behaves exactly as it does today.

### 3.1 Core addition: computed plan sections

Every plan section today goes through the planner LLM. The `multimember` section is a pure
function of stored data, so core gains one optional hook:

```rust
/// A section built without the LLM. Called in the plugin's planning stage, before
/// `plan_input`. When it returns `Some`, the section is added to the draft and to the stored
/// plan as is, and `plan_input`/`finalize_plan_section` are not called for this plugin.
async fn computed_plan_section(
    &self,
    _ctx: &PluginContext,
    _date: NaiveDate,
    _prior: &PlanDraft,
) -> Result<Option<PlanSection>, PluginError> {
    Ok(None)
}
```

Rules:
- Computed sections go into `draft` before the stage's LLM call, so a later stage sees them like
  any other finalized section. Their `warnings` join the plan's warnings. A computed section
  must not return `violations` (there is nothing to retry); core treats any as warnings.
- A stage whose plugins all returned computed sections makes no LLM call.
- If only computed sections exist (no LLM stage ran), planning still fails with
  `NothingToPlan`, as it does today when no section has input.
- Revisions recompute the section from current data; the revision instruction doesn't reach it.

### 3.2 Planning stages

| Plugin | `plan_after` (new in **bold**) |
|---|---|
| training | — |
| multimember | — |
| diet | `training`, **`multimember`** |
| stock | `training`, **`multimember`** |

Stock must name `multimember` too. Otherwise, with training off, stock would plan one stage
before diet and its shopping list couldn't follow diet's servings.

## 4. Data model (plugin migrations)

Each migration is one statement (core rule for plugin migrations). Conventions as in core: TEXT
UUID v7 ids from Rust, RFC 3339 UTC instants, ISO dates, JSON as TEXT, 0/1 flags as BIGINT.
There is **no owner row**: the owner eats every main meal by definition (§1, out of scope).

```sql
-- v1
CREATE TABLE multimember_members (
  id               TEXT PRIMARY KEY,
  seq              BIGINT GENERATED ALWAYS AS IDENTITY,
  display_name     TEXT NOT NULL,
  sex              TEXT NOT NULL,             -- core Sex: 'male' | 'female'
  birth_date       TEXT NOT NULL,             -- YYYY-MM-DD
  height_cm        DOUBLE PRECISION NOT NULL,
  weight_kg        DOUBLE PRECISION NOT NULL, -- a profile field, edited by hand; never logged
  activity_level   TEXT NOT NULL,             -- core ActivityLevel
  dietary_pattern  TEXT NOT NULL,             -- core DietaryPattern
  allergies_json   TEXT NOT NULL,             -- JSON array of strings
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  archived_at      TEXT                       -- set: hidden from planning and lists
)

-- v2
CREATE TABLE multimember_dislikes (
  id           TEXT PRIMARY KEY,
  seq          BIGINT GENERATED ALWAYS AS IDENTITY,
  member_id    TEXT NOT NULL REFERENCES multimember_members(id),
  text         TEXT NOT NULL,
  valid_from   TEXT,                          -- logical date, inclusive; NULL = open
  valid_until  TEXT,                          -- logical date, inclusive; NULL = open
  created_at   TEXT NOT NULL,
  deleted_at   TEXT
)

-- v3
CREATE TABLE multimember_attendance_defaults (
  member_id  TEXT NOT NULL REFERENCES multimember_members(id),
  weekday    BIGINT NOT NULL,                 -- 0 = Monday .. 6 = Sunday
  meal_type  TEXT NOT NULL,                   -- 'breakfast' | 'lunch' | 'dinner'
  present    BIGINT NOT NULL,
  PRIMARY KEY (member_id, weekday, meal_type)
)

-- v4
CREATE TABLE multimember_attendance_overrides (
  member_id  TEXT NOT NULL REFERENCES multimember_members(id),
  plan_date  TEXT NOT NULL,                   -- logical date
  meal_type  TEXT NOT NULL,
  present    BIGINT NOT NULL,
  note       TEXT,                            -- e.g. "飲み会"
  updated_at TEXT NOT NULL,
  PRIMARY KEY (member_id, plan_date, meal_type)
)
```

Notes:
- Members are archived, not deleted, so a stored plan's `servings` (§6.3) can still show a name.
  Dislikes are soft-deleted like core preferences.
- Medical notes are deliberately not collected for members.
- A member with no default row for a (weekday, meal) is **absent**. `multimember_add_member`
  writes defaults for the new member: present at **dinner on all 7 days**, absent otherwise,
  unless the call says otherwise.
- Attendance for a date resolves as: override if one exists, else the default, else absent.

## 5. Validation

`MemberProfile` validation, in the plugin. Failures are `CoreError::InvalidInput` with a
`ValidationRule`, so they reach the UI as data and the LLM as a fixable tool error.

| Field | Rule | ValidationRule |
|---|---|---|
| `display_name` | trimmed, 1–40 chars, unique among non-archived members (case-insensitive) | new `MemberNameInvalid`, new `MemberNameTaken` |
| `birth_date` | age (core `age_years`, on today's logical date) within **16..=70** | new `MemberAgeOutOfRange { got, birth_date }` |
| `height_cm` | 120–230 (same as the owner) | existing `HeightOutOfRange` |
| `weight_kg` | 30–300 (same as `core_log_weight`) | existing `WeightOutOfRange` |
| `allergies` | ≤ 20 entries, each trimmed 1–40 chars | new `MemberAllergiesInvalid` |
| members | at most **7** non-archived members | new `TooManyMembers { max }` |
| dislike `text` | trimmed 1–100 chars; `valid_from ≤ valid_until` when both set | reuse the core preference rules |
| attendance | `meal_type` is a main meal; `weekday` 0..=6; override date not in the past | new `AttendanceInvalid` |

Each new rule needs an `ErrorDetail` variant in `crates/api/src/error.rs` and strings in all
three locale tables.

Age is checked again at plan time (§6.1): a member who has since turned 71 is left out of plans
with a warning, never an error, and stays editable so the owner can archive them.

The owner's own profile bounds (age 16–100) don't change.

## 6. Planning

### 6.1 The `multimember` section (computed, no LLM)

`computed_plan_section(ctx, date, _)`:

1. Load non-archived members. Leave out any whose age on `date` is outside 16..=70 and add the
   warning "<name> は年齢範囲（16〜70歳）外のため献立から外しました" (worded via the plan warnings
   path; see §9 for locale).
2. Resolve attendance for each remaining member and each main meal on `date`.
3. Return `None` when no member attends any meal, so diet plans exactly as for a single person.
   Otherwise return:

```json
{
  "meals": [
    { "meal_type": "dinner", "guest_ids": ["0192…", "0192…"] }
  ],
  "guests": [
    { "member_id": "0192…", "name": "Aki",
      "sex": "female", "age": 34, "height_cm": 160, "weight_kg": 52, "activity_level": "light",
      "dietary_pattern": "omnivore",
      "allergies": ["えび"], "dislikes": ["セロリ"] }
  ]
}
```

`meals` lists only main meals with at least one guest. `guests` lists each attending member
once. `dislikes` are those valid on `date`. Both use `snake_case` per the serialization rule.

### 6.2 Diet: plan input

When the draft has a `multimember` section, diet's `plan_input`:

1. Computes each guest's **maintenance estimate**: `targets::compute(profile, weight_kg, age,
   None)`, with a `Profile` built from the guest's fields (`meals_per_day` 3, no medical notes).
   No exercise credit for guests.
2. Computes each guest's **scale** (§6.4) from that estimate and the owner's day kcal target
   (with exercise, as already used for the owner).
3. Adds to the brief, per shared meal:
   - "Shared dinner with: Aki, Ken. Cook for ≈ 3.1 portions; `items` stay the owner's portion."
   - Allergies of everyone at that meal (owner's included), as a **hard** constraint.
   - Dislikes of everyone at that meal, to avoid.
   - The strictest dietary pattern at the table: vegan > vegetarian > pescatarian > omnivore.
   - "The others aim to maintain their weight: keep shared meals balanced (protein, vegetables,
     a moderate amount of fat and salt)."
4. The output schema the LLM fills **does not change**. `items` remain the owner's portion, so
   the owner's targets, the existing validator and plan-vs-actual are unaffected. The LLM never
   outputs servings or scales.

### 6.3 Diet: finalize

`finalize_plan_section` reads the `multimember` section from `prior` and, after the existing
steps (§8.10 of the Phase 1 spec):

- **Violations** (retry), per shared meal: an item matches any attendee's allergy (same matching
  rule as the owner's allergies); the meal breaks the table's strictest dietary pattern, judged
  only by allergy-style name matching against a small built-in term list per pattern (meat and
  fish terms for vegetarian, plus egg and dairy terms for vegan, meat terms for pescatarian).
- **Warnings**: an item matches an attendee's dislike; a guest's scale was clamped (§6.4).
- Attaches to each shared meal:

```json
"servings": [
  { "member_id": "0192…", "name": "Aki", "scale": 0.92,
    "total": { "kcal": 610, "protein_g": 38, "carbs_g": 70, "fat_g": 18, "fiber_g": 7 } }
],
"portions": 3.1
```

  `total` is the meal total × `scale`, rounded like other totals. `portions` is
  `1 + Σ scale`, to one decimal. Both are computed in Rust.
- Attaches at section level `"guest_estimates": { "<member_id>": { "kcal": …, "protein_g": …,
  … } }`, the maintenance estimates, for display.
- `name` is copied in so stored plans stay readable after a member is renamed or archived.

When there is no `multimember` section, none of these fields appear and the section is
byte-for-byte what it is today.

### 6.4 Scale

```
scale(guest) = clamp(guest_maintenance_kcal / owner_day_target_kcal, 0.5, 2.0)
```

rounded to 2 decimals. It's the same for every meal the guest shares that day. Reasoning: the
owner's main meals plus snacks add up to the owner's target, so a guest who shares all three
main meals gets close to their maintenance kcal. They get a little less, because snacks aren't
shared; that is accepted and noted in the UI copy ("目安"). A clamp adds a warning naming the
guest.

Macros are not balanced per guest: a guest gets the owner's macro ratio (for example high
protein while the owner is in a deficit). This is reported, not validated. Composition differs
between people only once member-only extras exist, which is out of scope (§1).

### 6.5 Stock

Stock's brief gains one line when a `multimember` section is in its draft: "Shopping amounts
must cover every portion of shared meals (the diet brief gives the portion count)." Its
validator doesn't change. Stock and diet are planned in the same LLM call, which sees both
briefs.

## 7. Tools

All take and return `snake_case` JSON. The chat always speaks as the owner. These tools are how
the owner tells the assistant about the others ("my partner is out for dinner on Friday").

| Tool | Arguments | Effect |
|---|---|---|
| `multimember_list_members` | — | Members with profile fields, default attendance and current dislikes. |
| `multimember_add_member` | profile fields; optional `attendance: [{weekday, meal_type, present}]` | Adds a member (§4 default attendance unless given). |
| `multimember_update_member` | `member` (id or exact name), any profile fields | Partial update, revalidated. |
| `multimember_archive_member` | `member` | Archives. |
| `multimember_set_attendance` | `member`; either `weekday` + `meal_type` + `present` (default) or `date` + `meal_type` + `present` + optional `note` (override) | Upserts one row. |
| `multimember_clear_attendance_override` | `member`, `date`, `meal_type` | Removes an override. |
| `multimember_add_dislike` | `member`, `text`, optional `valid_from`, `valid_until` | Adds a dislike. |
| `multimember_remove_dislike` | `id` | Soft-deletes. |

Tool descriptions must say: these never log food or weight for anyone, and members can't have
goals.

`member` resolves by id first, then by case-insensitive exact display name among non-archived
members. Anything else is `NotFound`, listing the known names so the model can ask.

## 8. Context and Today

- `context_snippet(date)`: one line per member ("Aki (34, female): dinner daily; allergies: えび;
  dislikes: セロリ") plus today's and tomorrow's attendance, so the assistant can answer and
  update without a lookup. No weight or height in the snippet (DEBUG logging only, per
  `CLAUDE.md`).
- `today_card(date)`: who eats which meal today.
- Today and Nutrition show, on each shared meal: "一緒に: Aki, Ken · 3.1人前", and on expand, each
  guest's portion kcal and macros (labelled 目安). No rings or progress bars for guests.

## 9. API, server and UI

**Commands** (`crates/api/src/ops.rs`, routes in `crates/server`): `list_members`,
`add_member`, `update_member`, `archive_member`, `set_attendance_default`,
`set_attendance_override`, `clear_attendance_override`, `add_member_dislike`,
`remove_member_dislike`, `get_attendance(from, to)`. Errors leave Rust as `ErrorDetail`.

**Web UI** (`apps/web`): a new **Household** tab (家族 / Household / 家庭), shown while the plugin is
on, after Training:
- Member list with an add/edit form (all §5 fields, inline validation messages from
  `errors`), archive button.
- Weekly attendance grid: rows = members, columns = 7 days × 3 main meals, toggles.
- Next 7 days: overrides on top of the grid, with a note field.
- Per-member dislikes, same control as the owner's preferences.
- Shared-meal display on Today and Nutrition (§8).

Settings → Features gains the multimember switch, like training and stock. Every new string goes
into `ja.ts`, `en.ts` and `zh-CN.ts`. Plan warnings stay in the language the planner already
uses for warnings.

## 10. Milestones

### MM1 — Members and attendance storage
Crate, manifest, migrations v1–v4, store module, validation (§5) and `ValidationRule` /
`ErrorDetail` variants with translations. Registered in `cli` and `server`, optional, off.
**Done when:** migrations apply on a fresh and an existing tenant schema; unit tests cover every
§5 rule (including ages 15, 16, 70 and 71 on the boundary day, via the injected `Clock`) and
attendance resolution (override > default > absent); fmt, clippy and tests pass.

### MM2 — Tools and context
§7 tools, `context_snippet`, `today_card`.
**Done when:** orchestrator tests with `MockChatModel` add a member, set an override and add a
dislike through tool calls; unknown `member` returns `NotFound` with the names; the snippet has
no weight or height.

### MM3 — Computed sections in core
§3.1 hook and stage handling.
**Done when:** core tests show a computed section in the draft of a later stage, no LLM call
for an all-computed stage, `NothingToPlan` with only computed sections, and unchanged results
for plugins that don't implement the hook.

### MM4 — Diet and stock integration
§6.1–6.5: the section, diet brief and finalize, scale, stock brief line, `plan_after` changes.
**Done when:** tests in `crates/api/tests/` (the crate that may depend on several plugins)
cover a shared dinner end to end with the mock planner: guest allergen → violation and retry;
dislike → warning; scale computed and clamped; `servings`, `portions` and `guest_estimates`
present and computed in Rust; no `multimember` section → diet output identical to before; a
71-year-old member left out with a warning.

### MM5 — API and web UI
§9 commands, routes, Household tab, shared-meal display, feature switch, three locales.
**Done when:** server route tests for each command; `tsc --noEmit` passes; manually: add two
members, set attendance, generate tomorrow's plan, see shared meals with portions on Today.

## 11. Testing notes

- Never call a real LLM; use `MockChatModel`. Time from the injected `Clock`.
- The targets engine is reused, not re-tested; tests for guests check that diet builds the right
  `Profile` and passes `goal = None`.
- Privacy: a test asserts that member weight and height don't appear in INFO-level log output
  from planning.
