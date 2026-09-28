# Design: per-serving dishes (chain restaurant menus)

Status: **proposed 2026-09-27**. Written against branch `phase1` after `7e30b9b`. Extends
`docs/NUTRITION_SOURCES.md` (it lifts that document's "per-serving data" non-goal) and
`docs/PHASE1_SPEC.md` §8.1, §8.2, §8.4, §8.5 and §9.2. Where they disagree, this document
describes the new behavior.

## 1. Problem and goal

Every number the app computes today comes from **per-100 g food rows**. The parser splits a
meal into generic foods with gram weights, and each food resolves to a per-100 g row.

Chain restaurants (KFC, 松屋, McDonald's, …) publish nutrition **per menu item** (per serving),
often without the item's weight or its ingredient breakdown. That data can't enter today's
model:

- The importer's `basis_g` column scales values to per 100 g, which needs a weight the chain
  doesn't publish.
- Even with the weight, the parser splits "松屋の牛めし並盛" into 熟米饭 + 牛肉 + 洋葱, so a
  `牛めし（並盛）` row would never be looked up.

Goal: parsing gets two levels:

```
sentence ──parser──▶ dishes
                       │
                       ├─ dish has a per-serving record ──▶ nutrition = per_serving × servings
                       │                                    (one meal item: the dish)
                       │
                       └─ no record ──▶ the dish's ingredients (grams)
                                          └─▶ today's per-100 g resolver (chain + AI estimate)
                                              (flat meal items, exactly as today)
```

### Decisions taken (2026-09-27, with the user)

| # | Question | Decision |
|---|---|---|
| D1 | Shape of a meal whose dish had **no** record | **Flat ingredients, as today.** The dish name isn't kept. Only a matched dish becomes a dish item. |
| D2 | Matching a dish name to large chain menus | **Candidates + LLM pick.** Rust narrows the brand's **locally imported** dish rows to candidates, and one small parser-model call picks one of them or answers none. A pick is saved as an alias. |
| D3 | A branded dish with no data (e.g. a KFC item not imported) | **Decompose to ingredients.** Dishes are never AI-estimated per serving. |
| D4 | Plans | **Unchanged.** The planner keeps proposing foods × grams, and the plan validator keeps using the per-100 g resolver. |

### Non-goals

- Fetching menus or nutrition from the internet, or from the model's own knowledge. Dish data
  comes only from local imports and manual entry (§5.3, §7). A remote source could be added
  later through the source interface (`docs/NUTRITION_SOURCES.md` §4.4), but it's out of
  scope here.
- Per-serving LLM estimates (D3). The `llm_estimate` source holds per-100 g foods only.
- Dish items in plans (D4).
- User recipes, meaning a saved dish defined as a list of foods ("我的便当 = 米饭 200 g + …").
  §11 notes how they'd fit later.
- Modifiers on a matched dish ("牛めし without onions"). A modifier either maps to its own
  menu row (toppings such as 生卵 are usually their own row) or is ignored. §11 has the
  details.
- Rewriting history. Logged meals keep their snapshot, as they do today.

## 2. Concepts

| Term | Meaning |
|---|---|
| **Food** | Unchanged: a per-100 g row in `diet_foods`. |
| **Dish** | New: a per-serving row in `diet_dishes`. It has an optional **brand**, a **name** that includes its size (`牛めし（並盛）`), an optional `serving_g`, and macros **per serving**. |
| **Brand** | The chain or maker (`松屋`, `KFC`). An empty brand means a brandless dish, e.g. from the user's office canteen dataset. The brand is part of a dish's identity: `(brand_key, name_key)`. |
| **Servings** | How many servings were eaten (`0.25`–`10`; `半份` = 0.5). |
| **Dish item** | A logged meal item that came from a dish: `macros = per_serving × servings`. |
| **Food item** | A logged meal item that came from a food: `macros = per_100g × grams / 100`. This is today's `MealItem`. |

Both levels live in the **same source chain** (`docs/NUTRITION_SOURCES.md` §2): a dataset
source can hold foods, dishes, or both. `manual` can hold both. `llm_estimate` holds foods
only (D3). Priority, enable/disable, delete and replace work the same for both levels. Source
management, ordering and the Settings list therefore keep their current behavior; they only
show one more count.

### Precedence

1. **Dish before food.** When a parsed dish matches a dish record, its ingredients are never
   looked up. This follows the requested flow.
2. **Within dishes, chain priority.** The first source in priority order that knows
   `(brand_key, name_key)` wins, with the same shadowing rules as foods.
3. **A branded dish needs its brand.** A dish without a brand in the parse matches **only
   brandless dishes**. Without this rule, "我做了牛丼" (homemade) would match `松屋 牛丼`
   whenever only one brand sells a 牛丼. The parser infers a brand only when the user names
   it or the item is unmistakably a chain product ("麦辣鸡腿堡" → 麦当劳), using KNOWN_BRANDS
   (§5.1).

## 3. Storage — diet plugin migration 3

The migration adds new tables only. Existing tables and rows are unchanged, so no rebuild is
needed. It's registered in `DietPlugin::manifest()` as version 3 (`crates/plugin-diet/src/lib.rs`).

```sql
CREATE TABLE diet_dishes (
  id          TEXT PRIMARY KEY,                 -- UUID v7
  source_id   TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  brand       TEXT NOT NULL DEFAULT '',         -- display; '' = brandless
  brand_key   TEXT NOT NULL DEFAULT '',         -- normalize(brand)
  name        TEXT NOT NULL,                    -- includes size: '牛めし（並盛）'
  name_key    TEXT NOT NULL,
  serving_g   REAL,                             -- NULL when the chain doesn't publish it
  kcal        REAL NOT NULL,                    -- all macros PER SERVING
  protein_g   REAL NOT NULL,
  carbs_g     REAL NOT NULL,                    -- total carbohydrate, fiber included (B2)
  fat_g       REAL NOT NULL,
  fiber_g     REAL NOT NULL DEFAULT 0,
  use_count   INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (source_id, brand_key, name_key)
);
CREATE INDEX diet_dishes_brand_name ON diet_dishes(brand_key, name_key);

-- Aliases carried by dataset rows (import `aliases` column). Scoped to source and brand.
CREATE TABLE diet_dish_source_aliases (
  source_id  TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  brand_key  TEXT NOT NULL,
  alias_key  TEXT NOT NULL,
  dish_id    TEXT NOT NULL REFERENCES diet_dishes(id) ON DELETE CASCADE,
  PRIMARY KEY (source_id, brand_key, alias_key)
);

-- Global redirects, written by the LLM pick step (§5.3) and, later, by the user.
-- Keyed by name, not id, so they survive a dataset being replaced (same idea as
-- diet_name_aliases).
CREATE TABLE diet_dish_name_aliases (
  brand_key        TEXT NOT NULL,
  alias_key        TEXT NOT NULL,      -- normalize("<parsed name> <parsed size>")
  target_name_key  TEXT NOT NULL,
  origin           TEXT NOT NULL,      -- 'pick' | 'user'
  created_at       TEXT NOT NULL,
  PRIMARY KEY (brand_key, alias_key)
);
```

Why a separate table instead of a `basis` column on `diet_foods`:

- The unit is different, and so is the key: a dish's key includes the brand. Mixing both
  would put `WHERE basis = …` on every food query and make one row type mean two things.
- It keeps the typed split in Rust (`Food.per_100g` vs `Dish.per_serving`), so no code can
  scale a per-serving value by grams by mistake.
- The per-100 g path, including every existing query, test and the migration-2 layout, stays
  untouched.

`food_count` in `SourceInfo` is joined by a `dish_count`, filled by the same list query.

## 4. Types and the source interface

### 4.1 Rust types (`crates/plugin-diet/src/model.rs`, `sources.rs`)

```rust
pub struct Dish {
    pub id: String,
    pub brand: String,            // '' = brandless
    pub brand_key: String,
    pub name: String,
    pub name_key: String,
    pub serving_g: Option<f64>,
    pub per_serving: Macros,      // same struct, documented unit differs
    pub source_id: String,
    pub source_kind: SourceKind,  // never LlmEstimate
    pub source_name: String,
    pub use_count: i64,
}
```

`Macros`'s doc comment is updated so it no longer says "per 100 g for a food"; it now reads
"per 100 g (Food), per serving (Dish), or for a portion, meal or day". `Macros::for_grams`
stays. A new `Macros::times(f64)` covers servings.

### 4.2 `FoodSource` gains dish methods with default "miss" bodies

```rust
#[async_trait]
pub trait FoodSource: Send + Sync {
    // … existing: info, lookup, record_use …

    /// Exact match on (brand_key, name_key), then this source's dish aliases. `Ok(None)` = miss.
    async fn lookup_dish(&self, _brand_key: &str, _key: &str) -> Result<Option<Dish>, PluginError> { Ok(None) }

    /// All dishes of one brand in this source, for the candidate step (§5.3).
    async fn brand_dishes(&self, _brand_key: &str) -> Result<Vec<Dish>, PluginError> { Ok(vec![]) }

    async fn record_dish_use(&self, _dish: &Dish) -> Result<(), PluginError> { Ok(()) }
}
```

Default bodies mean a later `RemoteSource` doesn't have to support dishes. `TableSource`
implements all three against `diet_dishes`. For `llm_estimate` it can simply return the
defaults, since that source never has dish rows.

`SourceChain` gains:

- `lookup_dish(brand, name) -> Option<Dish>`. It applies the user/pick redirect in
  `diet_dish_name_aliases` first, then walks the chain. This mirrors `lookup`, including the
  dangling-alias fall-through.
- `brand_candidates(brand_key) -> Vec<Dish>`. The winning copy per `name_key` across enabled
  sources, in priority order.
- `known_brands(limit) -> Vec<String>`. Distinct display brands of enabled sources, ordered by
  summed `use_count`, then by name.
- `known_dishes(limit) -> Vec<String>`. Display names of **brandless** dishes (`brand_key =
  ''`) in enabled sources, for the parser's `{known_dishes}` (§5.1). Same shape as
  `sources::known_names`: one name per `name_key` (the copy from the highest-priority enabled
  source), ordered by `use_count` summed across sources, then by name. Branded dishes are
  excluded because the parser reaches them through `{known_brands}` and the pick step (§5.3).
  Name redirects in `diet_dish_name_aliases` aren't listed, the same as `known_names` ignores
  `diet_name_aliases`.

## 5. Parsing and resolution (replaces the §8.4 parse → resolve step)

### 5.1 Stage 1: the parser (one model call, as today)

The `MEAL_PARSER` prompt (§9.2) changes its output from flat items to dishes. **Every dish
carries its ingredient breakdown**, so the fallback (D1/D3) needs no second call:

```json
{"meal_type": "lunch|…|null",
 "dishes": [
   {"name": "牛めし", "brand": "松屋", "size": "並盛", "servings": 1, "grams": null,
    "components": [{"name": "熟米饭", "grams": 250}, {"name": "牛肉(炖)", "grams": 85},
                   {"name": "洋葱(炖)", "grams": 30}]},
   {"name": "鸡蛋", "brand": null, "size": null, "servings": 2, "grams": null,
    "components": [{"name": "鸡蛋", "grams": 100}]}
 ]}
```

Prompt rule changes (the existing rules about oil, raw/cooked states and drinks move into the
`components` wording unchanged):

- One entry per thing the user ate as a unit: a menu item, a home dish, or a single food. A
  single food is a dish whose only component is itself.
- `brand`: only if the user names the shop or the item is unmistakably that chain's product.
  Use the exact spelling from `KNOWN_BRANDS` when it's the same brand. Otherwise `null`.
- `size`: the size or variant as the user said it (並盛, 大盛, M, 2ピース), else `null`.
- A brandless item that is one of `KNOWN_DISHES`: copy that name exactly (it already includes
  any size, e.g. `A定食（大）`) and set `size` to `null`. `normalize` doesn't equate `A定食（大）`
  with `A定食 大`, so splitting the size out would miss the stage 2 match.
- `servings`: the count of that item (default 1). `grams`: only if the user states the weight
  of the whole item.
- `components`: what the item consists of, with grams **for everything eaten** (servings
  already applied). The existing composite-dish splitting rules apply here.
- Placeholders: `{known_foods}` (unchanged), new `{known_brands}` (`SourceChain::known_brands(50)`,
  §4.2), and new `{known_dishes}` (`SourceChain::known_dishes(100)`, §4.2: the most-used
  **brandless** dish names, so canteen datasets can match exactly).

Validation (`parser::validate_dishes`, which replaces `validate_items`; failures are
`InvalidArgs` as today):

| Rule | Limit |
|---|---|
| dishes per meal | 1–20 |
| `servings` | 0.25–10 |
| `grams` (when set) | 1–3000 |
| components per dish | 1–20, each with today's item rules (name non-empty, 1–2000 g) |
| flattened components per meal | ≤ 40 |

### 5.2 Stage 2: exact dish match (DB only)

For each dish, `SourceChain::lookup_dish(brand_key, key)` runs twice at most: first with
`key = normalize("{name} {size}")` when `size` is set, then with `normalize(name)`. Per §2
rule 3, a `null` brand searches only `brand_key = ''`.

### 5.3 Stage 3: candidates + LLM pick (D2) — only for branded exact misses

A dish goes to this stage when it has a brand that is in `known_brands` and stage 2 missed.
Brandless misses and unknown brands skip straight to stage 4, which costs no extra call.

**All menu data is local.** A brand's "menu" here means the dish rows already stored in the
local SQLite database, i.e. rows the user imported (§7.1) or entered by hand (§7.4), in
enabled sources. No stage fetches anything from the internet, and the model is never asked to
recall or look up a chain's menu or nutrition. The pick call (step 2) only chooses among the
local rows Rust gives it: its output is an index into that list or `null`, and it can't add an
item or a number. A brand with no local rows isn't in `known_brands`, so its dishes never
reach this stage.

1. **Candidates (Rust, local DB).** `brand_candidates(brand_key)` returns the brand's locally stored dish rows. If it has
   more than 200 rows, keep the top 200 by a character-bigram Dice score against
   `"{name} {size}"` (ties broken by `use_count`). This needs no FTS table or new dependency;
   menus are hundreds of rows, which is trivial in memory.
2. **Pick (one call per meal, not per dish).** New prompt `DISH_PICK`, role Parser, JSON.
   Input: each pending dish (the user's wording, `name`, `size`, `servings`) with its numbered
   candidate list (display names only). Output:
   `{"picks": [{"dish": 0, "candidate": 12}, {"dish": 1, "candidate": null}]}`. The prompt
   says: choose only a candidate that is the same item; choose the regular size (並/M/レギュラー)
   when no size was stated; otherwise `null`. **Size words may be in another language or
   generic** (小/中/大碗, 小/中/大份, small/medium/large, 普通): map them to the chain's
   equivalent size, where 中/medium/普通 means the regular size and 小/大 mean the next size
   down/up that exists on the menu. A stated size that has no reasonable equivalent is `null`.
3. **Checks.** An index out of range, or a dish missing from the output, counts as `null`.
   There's no re-ask: a wrong-shaped answer just means decomposition.
4. **Memory.** Each pick writes `diet_dish_name_aliases(brand_key, normalize("{name} {size}"),
   target_name_key, 'pick')`. The same wording then hits in stage 2 next time, with no model
   call. A `null` is **not** cached (§11 Q1).

Pick prompts include dish names, so they're logged at DEBUG only, like the other prompts.

### 5.4 Stage 4: fallback to ingredients

All dishes still unmatched contribute their `components` to **one** `resolve_many` call,
which is the existing per-100 g chain plus AI estimate, unchanged. Their meal items are today's
flat food items (D1).

One extra case: a matched dish with `grams` set but no `servings` given.

- If `serving_g` is known: `servings = grams / serving_g`.
- If `serving_g` is unknown: the grams can't be converted, so the dish **falls back to its
  components** (logged at DEBUG). That's the honest option: the numbers stay computed from
  data the app actually has.

### 5.5 Building the meal (`tools::log_meal`)

Items keep the parser's dish order. A matched dish becomes one dish item; an unmatched dish
becomes its components' food items, in place. Then `record_dish_use` runs on each matched
dish and `record_use` on each food, as today. `MealLogged.total` is still the sum of item
macros, computed in Rust.

Calls per logged meal: 1 (parser) + 0–1 (pick) + one estimate per unknown food (as today).

## 6. Meal item record (event payload, backward compatible)

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind { Food, Dish }           // #[serde(default)] → Food

pub struct MealItem {
    pub name: String,                       // food name, or dish name incl. size
    #[serde(default)] pub kind: ItemKind,
    #[serde(default)] pub brand: Option<String>,     // dish items only
    pub grams: Option<f64>,                 // food: always; dish: serving_g × servings if known
    #[serde(default)] pub servings: Option<f64>,     // dish items only
    pub food_id: Option<String>,            // food items (old events: always present)
    #[serde(default)] pub dish_id: Option<String>,   // dish items
    pub source: SourceKind,
    #[serde(default)] pub source_id: Option<String>,
    #[serde(default)] pub source_name: Option<String>,
    pub macros: Macros,                     // for this item
}
```

Old `diet.meal_logged` payloads (with `grams: number` and `food_id: string`) deserialize into
the `Option`s and default to `kind = Food`. This needs a test (§10).

`MealItem::to_json` adds `kind`, `brand` and `servings`; `grams` may be `null`. `estimated` is
still `source == LlmEstimate`, so it's always false for dish items. A new
`MealItem::label()` gives the one-line text used by `intake::context_snippet`, the desktop
and the CLI:

- food: `熟米饭 150g` (as today)
- dish: `松屋 牛めし（並盛）×1`

The correction flow in `diet_log_meal`'s description (re-log item by item) gets one example for
dishes: `"松屋 牛めし（並盛）1份、味噌汁1份"`, so a re-log hits stage 2 directly.

## 7. Import, export and manual entry

### 7.1 File format (`import.rs`)

CSV gets two new columns; JSON gets a new array.

| Column | Accepted headers | Meaning |
|---|---|---|
| `basis` | `basis`, `基准` | `100g` (default; also `100 g`, `每100g`) or `serving` (also `份`, `每份`). |
| `brand` | `brand`, `品牌`, `店铺`, `ブランド` | Dish rows only. On a `100g` row: dropped with a warning. |

Behavior of the existing columns on a `serving` row:

- `basis_g` / `每份克数` becomes `serving_g`, stored and **not** used to scale. It's optional.
- `name` includes the size, as the chain lists it. The same name under different brands is
  allowed.
- `aliases` are scoped to the brand in `diet_dish_source_aliases`.

A file may mix food and dish rows. JSON envelope: `"foods": [...]` (unchanged) and a new
`"dishes": [...]`, with rows `{brand, name, serving_g, kcal|kj, protein_g, carbs_g, fat_g,
fiber_g, aliases}`. A bare JSON array of rows follows the per-row `basis` rule, like CSV.

### 7.2 Dish row validation

| Check | Outcome |
|---|---|
| Missing or non-numeric required field; negative values; `fiber_g > carbs_g` | row **error** |
| `kcal > 5000` per serving | row **error** (sanity bound) |
| `serving_g` set and (`P + C + F > serving_g` or `kcal > 9 × serving_g`) | row **error** |
| `serving_g` ≤ 0 | row **error** |
| Same `(brand_key, name_key)` earlier in the file | row **error** (the first is kept) |
| Energy vs macros off by > max(20, 15 %) (same formula as foods) | **warning** only |

The `Tr`/`—` markers, kJ conversion, BOM, limits and the 200-issue cap all apply unchanged.

### 7.3 Preview, report, export

- `ImportPreview` gains `valid_food_rows`, `valid_dish_rows` and `brands: Vec<String>`
  (distinct, max 20). `valid_rows` stays as their sum. `sample` rows gain `basis`, `brand`
  and `serving_g`. `shadowed_by_manual` and `replaces_estimates` count **foods only**; dishes
  get their own `dish_shadowed_by_manual`. Nothing counts dishes against estimates, since
  estimates are never dishes.
- `ImportReport` gains `imported_dishes`.
- **Replace** deletes and reinserts both levels, and carries `use_count` over by
  `(brand_key, name_key)` for dishes.
- **Export** writes both levels. CSV gets the `basis`, `brand` and `serving_g` columns (the
  latter written as `basis_g` for compatibility); JSON uses `foods` and `dishes`. Round trips
  stay exact.

### 7.4 Manual dishes: new tool `diet_update_dish`

Symmetric with `diet_update_food`. It's for "the receipt says 牛めし並 is 733 kcal":

```
diet_update_dish { dish_name, brand?, kcal, protein_g, carbs_g, fat_g, fiber_g?, serving_g? }
```

It upserts into `manual` (after the pick/user redirect, like `diet_update_food`) and validates
with §7.2's hard rules. Its result note says only future meals are affected.

`diet_add_alias` stays food-only in v1 (§11 Q3); the pick step covers dish aliases.

## 8. Surfaces

### 8.1 CLI (`crates/cli/src/main.rs`)

- `foods list [--dishes] [--brand B]`: `--dishes` lists dishes instead of foods. The columns
  are brand, name, serving_g and per-serving macros.
- `foods lookup <name> [--brand B]`: also shows dish matches (winner first, then shadowed
  copies).
- `foods sources`: adds a `dishes` count column.
- `foods import` / `export`: the preview prints the food/dish split and the brand list.

### 8.2 Desktop (Tauri + UI)

- `list_foods` gains `kind: "food" | "dish"` and `brand?`, and returns `Food[] | Dish[]`.
  `commands.rs`, `api/commands.ts` and `api/types.ts` add the `Dish` type,
  `SourceInfo.dish_count`, the new preview/report fields, and the `MealItem` changes (§6).
- `today/MealList.tsx`: a dish item renders `label` data: brand in muted text, name, `×1`.
  The "估" badge logic is unchanged. `i18n/zh-CN.ts` gets `servingsShort: (n) => \`×${n}\``.
- `settings/FoodDataSection.tsx`: the source rows show `1,532 种食物 · 210 道菜品`, and
  sources with no dishes keep the current text. The import dialog shows the food/dish count
  tiles and brands, and the sample table gains a 基准 (basis) column (每100g / 每份). The help
  popover documents `basis`, `brand` and "`basis_g` means serving weight on `serving` rows".
- `TomorrowPlanCard.tsx`: no change (D4).

## 9. Affected scope

| Area | File(s) | Change |
|---|---|---|
| Schema | `plugin-diet/src/sources.rs` (or a new `dishes.rs`), `lib.rs` | Migration 3; register version 3 |
| Model | `plugin-diet/src/model.rs` | `Dish`, `ItemKind`, `MealItem` fields, `Macros::times`, `MealItem::label` |
| Source chain | `plugin-diet/src/sources.rs` | `FoodSource` dish methods, `TableSource` impl, `SourceChain::{lookup_dish, brand_candidates, known_brands, known_dishes}`, `dish_count`, `upsert_manual_dish`, delete/list include dishes |
| Parser | `plugin-diet/src/parser.rs`, `prompts.rs` | New output shape, `{known_brands}` / `{known_dishes}`, `validate_dishes` |
| Dish matching | **new** `plugin-diet/src/dish_resolver.rs` | Stages 2–3, bigram ranking, `DISH_PICK` prompt, alias write |
| Food resolver | `plugin-diet/src/resolver.rs` | **None.** It's still called with the flattened components. |
| Logging tool | `plugin-diet/src/tools.rs` | `log_meal` assembly (§5.5); new `diet_update_dish`; description examples |
| Intake | `plugin-diet/src/intake.rs` | `context_snippet` uses `MealItem::label`; totals are unchanged |
| Plan | `plugin-diet/src/validate.rs`, `plan.rs`, `core/src/plan.rs` | **Compile-only**: `MealItem` construction sets `kind: Food`, `grams: Some(..)` |
| Import/export | `plugin-diet/src/import.rs` | `basis`/`brand` columns, `dishes` array, dish validation, preview/report/replace/export |
| CLI | `crates/cli/src/main.rs` | §8.1 |
| Desktop Rust | `apps/desktop/src-tauri/src/commands.rs`, `tests/commands.rs` | `list_foods` kind, new fields pass through |
| Desktop UI | `api/types.ts`, `api/commands.ts`, `today/MealList.tsx`, `settings/FoodDataSection.tsx`, `i18n/zh-CN.ts` | §8.2 |
| Docs | `PHASE1_SPEC.md` §8.4/§9.2 pointers, `NUTRITION_SOURCES.md` non-goal line, `DECISIONS.md` | Point to this doc; record D1–D4 |
| **Not affected** | `core` (orchestrator, storage, scheduler), `llm`, targets engine, plan generation, Tomorrow card, event store schema | — |

## 10. Tests (all offline, `MockChatModel`)

- **Migration 3** on a migration-2 DB with foods: nothing changes in existing rows. Deleting
  a dataset cascades through `diet_dishes` and `diet_dish_source_aliases`, and
  `PRAGMA foreign_key_check` is empty.
- **Old events:** a Phase 1 and a migration-2-era `diet.meal_logged` payload both still
  deserialize, with `kind = Food` and `estimated` unchanged.
- **Parser:** the new output shape parses; `validate_dishes` covers every limit in §5.1; the
  prompt contains KNOWN_BRANDS and KNOWN_DISHES.
- **`known_dishes`:** lists brandless dishes only; a `name_key` present in two sources appears
  once, spelled as in the higher-priority one; a disabled source's dishes are absent; ordering
  is by summed `use_count`, then name; `limit` is honored.
- **Known dish hit:** a canteen dataset with `A定食（大）`; the mock parser returns that name
  with `size: null` → exact stage 2 hit, no pick or estimate call.
- **Exact dish hit:** "松屋 牛めし 並盛 ×2" with the dish in a dataset gives one dish item,
  `macros = 2 × per_serving`, no pick call and no estimate call (assert the request count).
- **Brand rule:** a brandless "牛丼" with only `松屋 牛丼` in the data never hits it; it
  decomposes.
- **Pick:** a branded miss sends one pick call listing the candidates. Picking candidate k
  logs dish k and writes the alias, so a second identical log makes no pick call. `null`, an
  out-of-range index or a missing entry all decompose. Two branded misses in one meal share
  **one** call. The pick prompt's candidates are exactly the brand's local rows in enabled
  sources, so a disabled source's dishes aren't listed. A brand with no local rows makes no
  pick call.
- **Size mapping:** "中碗松屋牛肉饭" with `松屋 牛めし（並盛）` in a dataset: the pick prompt
  contains the size-mapping rule and the user's `size: "中碗"`. A mocked pick of the 並盛
  candidate logs that dish and writes the alias `(松屋, "牛肉饭 中碗")`, so the same wording
  hits in stage 2 next time with no pick call.
- **Candidate ranking:** a brand with more than 200 rows is cut to 200, and the exact-ish
  match is included.
- **Fallback:** a matched dish with `grams` set and no `serving_g` decomposes. With
  `serving_g`, `servings = grams / serving_g`.
- **Mixed meal:** a matched dish plus an unmatched home dish gives the dish item followed by
  that dish's flat components, in parser order, and the total is their sum.
- **Priority:** the same dish in `manual` and a dataset → manual wins. After the dataset is
  disabled, the lower copy wins.
- **Import (pure, table-driven):** `basis` synonyms; `brand` on a `100g` row gives a warning;
  `serving_g` isn't used to scale; each §7.2 rule; the same name under two brands is fine,
  twice under one brand is an error; mixed files; the JSON `dishes` array.
- **Replace/export:** dish `use_count` carries over; the export → import round trip is
  identical for both levels.
- **`diet_update_dish`:** creates a manual dish that shadows the dataset copy, and rejects
  impossible values.
- **Desktop:** a `list_foods` round trip with `kind: "dish"`; a preview carries the split
  counts.

## 11. Open questions / later

- **Q1. Negative cache for picks.** A branded dish that isn't on the menu (e.g. an old limited
  item) costs one pick call every time it's logged. A `none` could be cached until any source
  holding that brand changes (compare `diet_food_sources.updated_at`). This is deferred as
  rare and cheap.
- **Q2. Brand aliases.** "McDonald's" / "麦当劳" / "マック" rely on the parser mapping them to
  KNOWN_BRANDS. If that proves unreliable, add a `brand_aliases` field to the import envelope
  and a `diet_brand_aliases` table.
- **Q3. User dish aliases through chat.** Extend `diet_add_alias` with an optional `brand` to
  write `diet_dish_name_aliases(origin='user')`. Not needed while the pick step learns
  aliases.
- **Q4. Modifiers on a matched dish** ("ご飯少なめ", "no mayo"). v1 ignores them for the
  numbers; the user's wording is still in `raw_text`. Later, the parser could emit
  `adjustments: [{name, grams: ±}]` resolved per 100 g and added to the dish item.
- **Q5. Plans with dishes (D4 later).** The plan item becomes `{name, grams} | {brand, name,
  servings}`, the validator resolves dishes with stage 2 only (no pick), and the planner
  brief lists the user's most-used dishes.
- **Q6. User recipes.** A dish whose values are derived from foods (`diet_dish_components`)
  fits the same `diet_dishes` identity. It would be the natural home for "keep the dish name
  when decomposed" if D1 is revisited.

## 12. Implementation stages

Each stage ends with `cargo fmt`, `clippy -D warnings`, `cargo test --workspace` green, and a
`DECISIONS.md` entry.

1. **Schema + types, no visible change.** Migration 3, `Dish`, the `FoodSource` dish methods,
   `SourceChain` dish lookup, and the `MealItem` changes with old-event tests. Callers set
   `kind: Food`.
2. **Parsing.** The new parser shape and prompt, `dish_resolver` (exact match + pick), `log_meal`
   assembly, `MealItem::label`, and the context snippet. `diet_update_dish`.
3. **Import/export + CLI.** The §7 format, validation, preview and replace, plus the §8.1
   commands.
4. **Desktop.** §8.2.
