# Design: pluggable nutrition data sources and batch import

Status: **implemented 2026-09-26** (stages 1–3 of §10; the two stages marked optional there —
§9.3's candidate-matching step and a first `RemoteSource` — were not built). Written 2026-09-26
against the Phase 1 code on branch `phase1` (after `8e7d0c9`). Extends `docs/PHASE1_SPEC.md` §5,
§8.2, §8.3, §8.5, §10.2, §10.3 and §11. Where this document and the spec disagree, this document
describes the new behavior. Deviations made while implementing it are recorded in
`docs/DECISIONS.md`'s 2026-09-26 entry.

## 1. Goal

Today every food lives in one table, `diet_foods`. Its values come either from the user
(`diet_update_food`, `source = 'user'`) or from a parser-model estimate made on a lookup miss
(`source = 'llm_estimate'`). The user has no way to bring in a trusted dataset, such as a food
composition table, their own spreadsheet, or a brand's nutrition labels.

This design adds:

1. **A swappable source interface.** Nutrition data comes from an ordered chain of
   sources, and the first source that knows a food wins.
2. **Fixed ends of the chain.** The first source is always the user's manual entries. The LLM
   estimate is always the last resort, used only when no other source knows the food.
3. **Batch import.** CSV or JSON files become new dataset sources in the chain. Import works
   from both the desktop UI and the CLI.

```
lookup "熟米饭"
  │
  ├─ 1. 手动录入 (manual)          ── fixed first; diet_update_food writes here
  ├─ 2. 中国食物成分表 (dataset)   ─┐
  ├─ 3. my_snacks.csv (dataset)    ─┤ imported; user reorders, enables, disables, deletes
  ├─ …                             ─┘
  ├─ N. AI 估算缓存 (llm_estimate) ── fixed last; earlier estimates
  └─    miss everywhere → estimate with the parser model (§8.2/§8.3), store in N
```

### Non-goals (for this design)

- Remote sources (USDA FoodData Central API, Open Food Facts, barcode lookup). The interface
  allows them (§4.4), but none is built here.
- Fuzzy or semantic name matching. Matching stays exact on normalized names and aliases.
  §9 describes an optional later stage for this, because it is the main limit of large
  datasets.
- Importing through chat. Files never pass through the LLM (cost, privacy, and the rule that
  numbers never come from the model).
- Per-serving data. The stored basis is always per 100 g of the edible portion. The importer
  can convert other bases (§6.2). *(Lifted by `docs/DISH_NUTRITION.md`: per-serving dishes.)*
- Changing past meals. Logged meals keep a snapshot of their values (already true today:
  `MealItem.macros` is stored in the event payload). Import, reorder or delete never rewrites
  history.

## 2. Concepts

| Term | Meaning |
|---|---|
| **Source** | A named provider of per-100 g nutrition rows. It has a kind, a priority and an enabled flag. |
| **Kind** | `manual` (exactly one, id `manual`), `dataset` (zero or more, created by import), `llm_estimate` (exactly one, id `llm_estimate`). Later: `remote`. |
| **Chain** | Enabled sources in priority order. Resolution walks it and stops at the first hit. |
| **Shadowing** | A food present in several sources: only the highest-priority copy is used. Lower copies stay stored and come back if the higher one is deleted or disabled. |
| **User alias** | A global name redirect added by the user (`diet_add_alias`): `卤蛋 → 卤鸡蛋`. Applied before the chain walk (§5 step 2). |
| **Source alias** | An alternative name carried by a dataset row (a CSV `aliases` column). Scoped to that source and removed with it. |

Rules:

- `manual` is always first, can't be disabled, deleted or reordered, and can't be
  imported into. It holds exactly what the user entered by hand.
- `llm_estimate` is always last and can't be deleted or reordered. It can't be disabled
  either in v1 (see open question Q1).
- Datasets sit between the two, in an order the user sets.
- Stored `priority` values are internal (§3.1: manual 0, datasets 1..n, estimate 1000000).
  Every surface shows a 1-based position instead: manual is 1, the first dataset is 2, and
  the CLI's `source move --to POS` counts among datasets only (§8.1).

## 3. Storage

Diet plugin **migration 2**, added to `DietPlugin::manifest()` next to `foods::MIGRATION_1`.

### 3.1 New and changed tables

```sql
CREATE TABLE diet_food_sources (
  id           TEXT PRIMARY KEY,      -- 'manual', 'llm_estimate', or UUID v7 for datasets
  kind         TEXT NOT NULL,         -- 'manual' | 'dataset' | 'llm_estimate'
  name         TEXT NOT NULL,         -- display name; datasets default to the file stem
  priority     INTEGER NOT NULL,      -- manual 0, datasets 1..n (dense), llm_estimate 1000000
  enabled      INTEGER NOT NULL DEFAULT 1,
  origin_json  TEXT NOT NULL DEFAULT '{}',  -- datasets: {file_name, format, imported_at, rows}
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

-- Replaces the Phase 1 diet_foods (the unique key changes, so SQLite needs a rebuild).
CREATE TABLE diet_foods (
  id          TEXT PRIMARY KEY,
  source_id   TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL,
  kcal        REAL NOT NULL,
  protein_g   REAL NOT NULL,
  carbs_g     REAL NOT NULL,          -- total carbohydrate, fiber included (B2)
  fat_g       REAL NOT NULL,
  fiber_g     REAL NOT NULL DEFAULT 0,
  use_count   INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (source_id, name_key)
);
CREATE INDEX diet_foods_name_key ON diet_foods(name_key);

-- Source-scoped aliases (dataset rows). Replaces the json_each scan over aliases_json, which
-- doesn't scale to datasets with tens of thousands of rows.
CREATE TABLE diet_food_aliases (
  source_id  TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  alias_key  TEXT NOT NULL,
  food_id    TEXT NOT NULL REFERENCES diet_foods(id) ON DELETE CASCADE,
  PRIMARY KEY (source_id, alias_key)
);

-- Global user aliases: a name redirect applied before the chain walk.
CREATE TABLE diet_name_aliases (
  alias_key   TEXT PRIMARY KEY,
  target_key  TEXT NOT NULL,          -- a normalized name, not a food id: survives re-imports
  created_at  TEXT NOT NULL
);
```

The `source` column (`'user'`/`'llm_estimate'`) goes away because provenance now comes from
`source_id` → `diet_food_sources.kind`. `aliases_json` goes away too: its entries were all
added by the user through `diet_add_alias`, so they move to `diet_name_aliases`.

Foreign keys already work: `crates/core/src/db.rs` enables `foreign_keys(true)`. So deleting a
dataset source cascades to its foods and aliases.

### 3.2 Migration of existing data (inside migration 2, one transaction)

`apply_migrations` runs each migration inside a transaction (`crates/core/src/plugin.rs`), and
`PRAGMA foreign_keys` can't be changed inside a transaction, so foreign keys stay **on** for
the whole rebuild. The steps are ordered so that this is safe. In particular, no table may
reference `diet_foods` until the rebuilt table has its final name. Otherwise the reference
would point at the table being dropped, or be rewritten by the rename.

1. `CREATE TABLE diet_food_sources (…)` and insert the two built-in rows: `manual`
   ("手动录入", priority 0) and `llm_estimate` ("AI 估算", priority 1000000).
2. `CREATE TABLE diet_foods_v2 (…)`, with the §3.1 `diet_foods` columns and constraints.
   `INSERT INTO diet_foods_v2 SELECT …` from `diet_foods`, mapping `source = 'user'` →
   `source_id = 'manual'` and `'llm_estimate'` → `'llm_estimate'`. Keep ids, so past
   `MealItem.food_id` values stay valid.
3. `CREATE TABLE diet_name_aliases (…)`. It has no foreign keys, so it can be created at
   any point.
   `INSERT INTO diet_name_aliases SELECT j.value, f.name_key, f.updated_at FROM diet_foods f,
   json_each(f.aliases_json) j`. Phase 1 stores alias entries already normalized, and
   `add_alias` keeps them unique across foods, so the primary key can't conflict.
4. `DROP TABLE diet_foods;` Nothing references the old table, so the implicit delete passes
   the FK checks.
5. `ALTER TABLE diet_foods_v2 RENAME TO diet_foods;`
6. `CREATE INDEX diet_foods_name_key ON diet_foods(name_key);`
7. `CREATE TABLE diet_food_aliases (…)`. Its FK now names the final `diet_foods`. It starts
   empty because Phase 1 has no source-scoped aliases.

Two implementation notes:

- **Timestamps.** Migration SQL is a static string, so the built-in rows' timestamps come from
  `strftime('%Y-%m-%dT%H:%M:%S+00:00','now')`. That is the same shape `Clock::now().to_rfc3339()`
  writes elsewhere (`+00:00`, not `Z`). This is a deliberate exception to the "time from
  `Clock`" rule: it happens once at migration time and no test asserts on it. Record it in
  `DECISIONS.md`.
- **Multi-statement migration.** `apply_migrations` runs `migration.sql` with one
  `sqlx::query(..).execute`. sqlx's SQLite driver runs every statement in the string, but add
  a test that migrates a Phase 1 database with foods and aliases and checks the result. If
  that ever proves wrong, split migration 2 into versions 2..k. Each still runs in its own
  transaction.

## 4. The source interface

New module `crates/plugin-diet/src/sources.rs`. `foods.rs` keeps `normalize`, the §8.3 checks
and the row-level SQL helpers.

### 4.1 Types

```rust
/// Provenance kind. Serialized names keep Phase 1's values so old event payloads still parse:
/// Manual is "user", as `NutritionSource::User` was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    #[serde(rename = "user")]
    Manual,
    #[serde(rename = "dataset")]
    Dataset,
    #[serde(rename = "llm_estimate")]
    LlmEstimate,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub kind: SourceKind,
    pub name: String,
    pub priority: i64,
    pub enabled: bool,
    pub food_count: i64,         // filled by list queries
    pub origin: Option<DatasetOrigin>,
}

pub struct Food {                 // existing struct, changed fields
    pub id: String,
    pub name: String,
    pub name_key: String,
    pub per_100g: Macros,
    pub source_id: String,
    pub source_kind: SourceKind,  // replaces `source: NutritionSource`
    pub source_name: String,
    pub use_count: i64,
}
```

`NutritionSource` is renamed to `SourceKind` and gains `Dataset`. Its serde names stay the same,
so existing `diet.meal_logged` payloads (`"source": "user"` / `"llm_estimate"`) deserialize
unchanged.

`Food.aliases` (Phase 1: the food's normalized alias keys, read from `aliases_json`) is
removed. Aliases are no longer a property of one row: user aliases are global redirects, and
source aliases live in `diet_food_aliases`. Callers change as follows:

- The `diet_add_alias` result becomes `{ food_id, name, alias }`, echoing the alias that was
  added instead of the full list.
- `foods list` output drops the field. `foods lookup` (§8.1) shows the user aliases pointing
  at the name and the source aliases of each copy.

### 4.2 The trait

```rust
#[async_trait]
pub trait FoodSource: Send + Sync {
    fn info(&self) -> &SourceInfo;

    /// Exact match on a normalized key, by name first and then by this source's own aliases.
    /// Never calls a model. `Ok(None)` is a miss.
    async fn lookup(&self, key: &str) -> Result<Option<Food>, PluginError>;

    /// Called on the food that won a resolution. DB-backed sources bump `use_count`.
    async fn record_use(&self, _food: &Food) -> Result<(), PluginError> { Ok(()) }
}
```

Implementations:

| Impl | Backs | Notes |
|---|---|---|
| `TableSource { info, db }` | `manual`, every `dataset`, and the `llm_estimate` cache | One indexed query per lookup: `WHERE source_id = ? AND name_key = ?`, then `diet_food_aliases`. |
| *(later)* `RemoteSource` | e.g. an HTTP food database | Would cache its hits as rows under its own `source_id`, so it's fast and offline after the first lookup. Not built in this design. |

While `TableSource` is the only implementation, the chain walk costs up to two queries per
source for each name. An allowed optimization: when every enabled source is a
`TableSource`, `SourceChain::lookup` may run one query over `diet_foods` joined to the
enabled `diet_food_sources` (and `diet_food_aliases`), ordered by `priority`, then name
match before alias match, `LIMIT 1`. It must return exactly what the walk would, and the
chain-priority tests (§11) apply to both. The trait still exists for later non-table
sources.

The LLM estimator is **not** a `FoodSource`. It produces new data instead of looking data up,
and it runs as the chain's fallback step (§5). Its earlier results are ordinary rows in the
`llm_estimate` `TableSource`.

### 4.3 The chain

```rust
pub struct SourceChain { sources: Vec<Box<dyn FoodSource>> }

impl SourceChain {
    /// Enabled sources in priority order, built from diet_food_sources.
    pub async fn load(ctx: &PluginContext) -> Result<Self, PluginError>;

    /// User alias redirect, then the first hit in priority order.
    pub async fn lookup(&self, name: &str) -> Result<Option<Food>, PluginError>;

    /// Every copy of a food across all sources, winners first (for `foods lookup` / debugging).
    /// Includes copies in disabled sources, flagged as such, after the enabled ones.
    pub async fn lookup_all(&self, name: &str) -> Result<Vec<Food>, PluginError>;
}
```

Within one source, a name match beats an alias match. Across sources, priority comes first:
an alias hit in a higher source beats an exact name hit in a lower one. This follows from
"first source that knows a food wins": a dataset that lists `米饭` as an alias of `熟米饭`
knows that food.

`load` runs per resolution call. That costs one small query, and it means reorder, enable or
import takes effect immediately with no cache to invalidate.

### 4.4 Adding a new kind of source later

1. Add a `SourceKind` variant and a `kind` string.
2. Implement `FoodSource`.
3. Add a branch to the factory in `SourceChain::load` that maps a `diet_food_sources` row of
   that kind to the new impl. Kind-specific configuration goes in `origin_json`, and secrets
   go in `SecretStore`, never in the table.

The source code, the ordering and the UI don't change.

## 5. Resolution (replaces §8.2 step 1–2)

`resolver::resolve_many(ctx, names)`, same signature and same concurrency limit (4):

1. `chain = SourceChain::load(ctx)`.
2. For each name: `key = normalize(name)`. If `diet_name_aliases` has `key`, walk the chain
   for its `target_key` first. Only one hop is followed, and redirect chains are never
   created (§7.3).
3. If there was no redirect, or the redirect's target is found in no enabled source (a
   **dangling** alias, e.g. its dataset was deleted or disabled), walk the chain for `key`
   itself and take the first hit. A dangling alias is kept, not deleted: it works again when
   its target comes back.
4. Misses left after the whole chain (including the `llm_estimate` cache) are estimated by the
   parser model, exactly as today: `FOOD_ESTIMATE` prompt, `check_nutrition`, one re-ask, then
   `无法估算该食物营养`. The result is inserted into source `llm_estimate` under the name
   the user wrote (`name`/`key`), never under a dangling alias's `target_key`. The estimate
   is for what the user asked about, and the target key has no display name to store.
5. `record_use` on each winner.

`resolve_many` has two callers, and both get this behavior: meal logging (§8.4) and plan
validation (`validate.rs`, §8.10). So a plan that names a food no source knows also creates
an `llm_estimate` row, as it does today.

This gives two properties:

- **Datasets back up the estimate.** A food the model estimated earlier and a dataset that
  arrives later: the dataset has the higher priority, so it wins from then on with no
  migration. The old estimate stays shadowed in the cache.
- **Manual always wins.** `diet_update_food` writes to `manual`, so the user's correction
  shadows every dataset and every estimate.

`MealItem` (event payload) gains two fields, `#[serde(default)]` for old events:

```rust
pub source_id: Option<String>,
pub source_name: Option<String>,   // snapshot, so a later rename or delete doesn't change history
```

`MealItem::to_json`'s `estimated` stays `source == LlmEstimate`, so the Today page's "估" badge
behaves as before. `to_json` also gains `source_name` (`null` for old events) so the UI can
show it as a tooltip.

`known_names` (the parser's `KNOWN_FOODS`, max 150) becomes: names of winning foods across
enabled sources, grouped by `name_key`, ordered by `SUM(use_count)` and then by name. Imported
rows start at `use_count = 0`, so a new dataset doesn't flood the prompt. Its names appear
there once they are used (see §9 for the matching limit this leaves).

## 6. Batch import

New module `crates/plugin-diet/src/import.rs`. It has two parts, so preview and commit share
one code path:

```rust
/// Pure: bytes in, rows and issues out. No DB, no clock, no model. Fully unit tested.
pub fn parse(file_name: &str, bytes: &[u8]) -> Result<ParsedImport, FoodDataError>;

/// Checks against the current DB (shadowing counts) without writing.
pub async fn preview(ctx: &PluginContext, parsed: &ParsedImport) -> Result<ImportPreview, FoodDataError>;

/// Writes in one transaction: a new dataset source, or replaces an existing dataset's rows.
pub async fn commit(ctx: &PluginContext, parsed: ParsedImport, target: ImportTarget)
    -> Result<ImportReport, FoodDataError>;

pub enum ImportTarget {
    New { name: String },                 // appended as the lowest-priority dataset (just above llm_estimate)
    Replace { source_id: String },        // keeps id, name and priority; swaps all rows atomically
}
```

User-facing failures are a dedicated `thiserror` enum. Its `Display` is the final zh-CN
message, because the generic `CoreError` → `CommandError` mapping in the desktop
(`error.rs`) replaces messages with fixed text such as `未找到对应的数据。`, and would lose
it:

```rust
#[derive(Debug, thiserror::Error)]
pub enum FoodDataError {
    #[error("{0}")]
    Invalid(String),       // user-caused: bad file, limits, zero valid rows, rename length, …
    #[error("{0}")]
    NotFound(String),      // unknown source id, expired import token
    #[error(transparent)]
    Plugin(#[from] PluginError),   // db and other internal failures
}
```

`import.rs` and the §7.1 management functions return `FoodDataError`. The CLI prints
`Display`. §8.2 covers the desktop mapping.

### 6.1 File formats

**Encoding:** UTF-8 only, with a leading BOM tolerated (Excel adds one). Anything else fails
with `文件不是 UTF-8 编码，请用 UTF-8 另存后重试` (save as UTF-8 and retry).

**Format detection:** from the `.csv` / `.json` extension. Otherwise from the first
non-whitespace byte: `{` or `[` means JSON, anything else means CSV.

**CSV:** comma-separated with a header row, parsed with the `csv` crate. This is a new
dependency. It's justified because quoted fields with commas, newlines and escaped quotes are
exactly the edge cases a hand-rolled splitter gets wrong. Columns can be in any order, and
unknown columns are ignored with one warning listing them.

| Column | Required | Accepted header names | Meaning |
|---|---|---|---|
| `name` | yes | `name`, `名称`, `食物` | Display name. Normalized with `normalize()` for the key. |
| `kcal` | one of `kcal`/`kj` | `kcal`, `energy_kcal`, `能量`, `热量`, `能量(kcal)` | Energy per basis. |
| `kj` | one of `kcal`/`kj` | `kj`, `energy_kj`, `能量(kj)` | Converted as `kcal = kj / 4.184`. If both are given, `kcal` wins, with a warning when they disagree by more than 2 %. |
| `protein_g` | yes | `protein_g`, `protein`, `蛋白质` | |
| `carbs_g` | yes | `carbs_g`, `carbohydrate`, `碳水化合物`, `碳水` | **Total** carbohydrate, fiber included (B2). |
| `fat_g` | yes | `fat_g`, `fat`, `脂肪` | |
| `fiber_g` | no (0) | `fiber_g`, `fiber`, `膳食纤维`, `纤维` | |
| `basis_g` | no (100) | `basis_g`, `per_g`, `每份克数` | Values are per this many grams. Scaled to per 100 g on import. |
| `aliases` | no | `aliases`, `别名` | `\|`-separated (or `｜`). |

Header matching is case-insensitive, trimmed, and ignores ASCII/full-width parentheses. Empty
cells in optional columns take their default. An empty cell in a required column makes the
row an error.

**Energy unit.** A bare `能量` or `热量` header is read as kcal. The importer doesn't try to
detect kJ values in a kcal column. Checking the unit is the user's responsibility: rename the
column to `kj` / `能量(kJ)` when the file is in kJ. The in-app help (§8.3) and the CLI
`foods import --help` both say so. The preview's energy warning (§6.2) and the sample table
make a wrong unit easy to spot, but they don't block the import.

**Non-numeric markers.** Food composition tables use markers instead of numbers:

| Cell (trimmed, case-insensitive) | Meaning | Outcome |
|---|---|---|
| `Tr`, `微量` | trace | `0`, with one file-level warning counting the affected rows |
| `—`, `–`, `-`, `…`, `...`, `N/A`, `NA` | not measured | optional column: its default. Required column: row **error** |

Other non-numeric text is an error in any column. Numbers may use a full-width digit or
decimal point (`１２．５`), which is converted. Thousands separators are not accepted.

**JSON:** either a bare array of rows, or an envelope:

```json
{
  "format": "lifeagent.foods.v1",
  "name": "中国食物成分表 第6版",
  "foods": [
    { "name": "熟米饭", "kcal": 116, "protein_g": 2.6, "carbs_g": 25.9, "fat_g": 0.3,
      "fiber_g": 0.3, "aliases": ["米饭", "白米饭"] },
    { "name": "燕麦片", "kj": 1413, "protein_g": 10.1, "carbs_g": 77.4, "fat_g": 0.2,
      "fiber_g": 6.0, "basis_g": 100 }
  ]
}
```

Row keys are the canonical column names above (English only). `aliases` is an array.
Unknown keys are ignored. The envelope's `name` is the default source name. Otherwise the
default is the file stem.

### 6.2 Row validation

Each row is scaled to per 100 g (`basis_g`), then checked:

| Check | Outcome |
|---|---|
| Missing or non-numeric required field (after the §6.1 marker rules); `basis_g` ≤ 0 | row **error** |
| `validate_per_100g` hard rules (all ≥ 0, fiber ≤ carbs, P+C+F ≤ 100, kcal ≤ 900) | row **error** |
| Empty name after `normalize`; name > 100 chars | row **error** |
| Same `name_key` as an earlier row in the file | row **error** (the first occurrence is kept) |
| kcal differs from the §8.3 energy `4·protein + 4·(carbs − fiber) + 9·fat + 2·fiber` by > max(20, 15 %) | **warning** only. Imported values aren't corrected: composition tables can use food-specific energy factors, and user data wins as in §16. |
| Alias equal to another row's name or alias in the same file; > 20 aliases | alias dropped, **warning** |
| Alias equal to the row's own name | alias dropped silently |
| Unknown columns or keys | one file-level **warning** |

Rows with errors are skipped. The rest import. File limits: ≤ 20 MB, ≤ 50 000 data rows, and
at least one valid row, otherwise the import fails as a whole. Issue lists are capped at 200
entries, and the report gives the full count.

Messages are zh-CN strings built in Rust, like `validate.rs`'s plan violations. So the CLI and
the desktop show the same text, and `CommandError.message` stays zh-CN. Each issue carries its
1-based line number (the CSV line, or the JSON array index + 1) and the row's name when there
is one.

### 6.3 Preview and report

```rust
pub struct ImportPreview {
    pub format: ImportFormat,           // csv | json
    pub suggested_name: String,
    pub total_rows: usize,
    pub valid_rows: usize,
    pub errors: Vec<RowIssue>,          // capped at 200
    pub error_count: usize,
    pub warnings: Vec<RowIssue>,        // capped at 200
    pub warning_count: usize,
    pub shadowed_by_manual: usize,      // valid rows whose name the user has set by hand (they won't take effect while manual has them)
    pub replaces_estimates: usize,      // valid rows that will override an earlier AI estimate
    pub sample: Vec<ImportRowView>,     // first 20 valid rows, values per 100 g, rounded for display
}

pub struct RowIssue { pub line: usize, pub name: Option<String>, pub message: String }

pub struct ImportReport {
    pub source: SourceInfo,
    pub imported: usize,
    pub skipped: usize,
}
```

### 6.4 Commit

One transaction:

- **New:** insert a `diet_food_sources` row. Its priority is the max dataset priority + 1,
  which puts it last among datasets, and it can be reordered afterwards. Insert every valid
  row into `diet_foods` and its aliases into `diet_food_aliases`.
- **Replace:** keep the source row but update `origin_json` and `updated_at`. Delete its foods
  (aliases cascade), then insert the new rows. `use_count` carries over by `name_key`, so
  frequently used foods stay in `KNOWN_FOODS`.

Inserts are batched, a few hundred rows per statement, well under SQLite's
bound-parameter limit. At 50 000 rows this stays within a couple of seconds.

After commit, log at INFO the source id, row counts and duration. Food names and file
paths are logged at DEBUG only (§12 logging rule).

Import doesn't append to the event store. It isn't a life record. It also doesn't emit
`data-changed`, since no Today number changes. The Settings section refetches its own list.

## 7. Managing sources

### 7.1 Operations (in `sources.rs`)

| Function | Rules |
|---|---|
| `list_sources(ctx) -> Vec<SourceInfo>` | All sources in priority order, with `food_count`. |
| `reorder_datasets(ctx, ids: Vec<String>)` | Must be exactly the current dataset ids. Renumbers them 1..n. `manual` and `llm_estimate` are rejected. |
| `set_enabled(ctx, id, bool)` | Datasets only. |
| `rename_source(ctx, id, name)` | Datasets only. 1–60 chars. |
| `delete_source(ctx, id)` | Datasets only. Cascades to its rows. Past meals are unaffected (snapshots). |
| `list_foods(ctx, source_id?, query?, limit)` | Browsing. `query` is a `name_key LIKE %q%` substring match. |
| `export(ctx, source_id, format) -> Vec<u8>` | Same CSV/JSON formats as import, so an export can be imported again. Includes the source's own aliases. Any source can be exported, including `manual` and `llm_estimate`. |

**Export limits (v1).** An export is data portability, not a full backup:

- User aliases (`diet_name_aliases`) aren't exported, because they are global rather than
  per-source.
- A `manual` export can only be re-imported as a **dataset** (Q2), so the restored rows sit
  below `manual`, not in it. Moving that dataset to the top of the datasets gives nearly the
  same effect.

A full backup is the SQLite file itself.

### 7.2 Manual source writes

- `diet_update_food` always **upserts into `manual`**: it creates or updates the row
  `(manual, key)`, where `key` is resolved through `diet_name_aliases` first, as in today's
  alias behavior. It no longer modifies whatever row happens to match. So a correction never
  edits an imported dataset or the estimate cache. It shadows them.
- When the call omits `fiber_g`, the fiber value comes from the food's current **chain
  winner** (whichever source that is, dataset included), else `0`. This is today's
  "keep the current value" rule, applied through the chain.
- Its tool description and result text stay as they are ("affects future entries only").

### 7.3 User aliases

`diet_add_alias(food_name, alias)`:

- `food_name` must resolve through the chain (any source). Otherwise `NotFound`.
- The redirect is stored as `alias_key → target_key`, where `target_key` is the resolved
  food's `name_key` (not its id). The alias then keeps working after the target's dataset is
  re-imported, or when a manual entry later shadows it.
- The call is rejected when any of these holds:
  - `alias_key` already resolves to a *different* food (same rule as today);
  - `target_key` is itself an alias key (no `X → alias`);
  - `alias_key` is already the `target_key` of another alias (no `Y → X` followed by `X → Z`).

  The last two rules together prevent chains and cycles. The call is idempotent when the
  alias already points at the same key.
- `food_name` may itself be a user alias. Resolution follows the redirect, so the new alias
  points at the final key, never at the alias.

## 8. Surfaces

### 8.1 CLI (`lifeagent-cli foods …`)

```
foods list [--source ID] [--query Q] [--limit N]     # existing command, gains filters
foods lookup <name>                                  # winner first, then shadowed copies, with source names
foods sources                                        # table: priority, id, kind, name, enabled, foods
foods import <file> [--name NAME] [--replace ID] [--dry-run] [--yes]
foods export --source ID [--format csv|json] [--out FILE]   # stdout by default
foods source move <ID> --to <POS>                   # POS is 1-based among datasets
foods source enable <ID> | disable <ID>
foods source rename <ID> <NAME>
foods source delete <ID> [--yes]
```

`foods import` prints the preview: counts, the first 20 errors and warnings, and the sample.
With `--dry-run` it stops there. Otherwise it asks `Import N foods? [y/N]` unless `--yes` is
given, then prints the report. Errors print as `line 12 (熟米饭): …`. The command exits
non-zero if nothing could be imported.

### 8.2 Tauri commands (`apps/desktop/src-tauri/src/commands.rs`)

These are thin wrappers calling `lifeagent_plugin_diet::{sources, import}` with
`state.plugin_context()`, as the CLI does (`crates/cli/src/main.rs`, `foods list`).

**Error mapping.** Add `impl From<FoodDataError> for CommandError` in `error.rs`, and use it
only for these commands:

| `FoodDataError` | `code` | `message` |
|---|---|---|
| `Invalid(msg)` | `invalid_input` | `msg`, passed through: it's already zh-CN (§6) |
| `NotFound(msg)` | `not_found` | `msg`, passed through |
| `Plugin(e)` | `internal` | `发生内部错误，请重试。` (the existing internal-error text), with `e` logged at ERROR |

Don't route these errors through `CoreError`: its `NotFound`/`Validation` arms replace the
message with fixed text.

| Command | Input | Output |
|---|---|---|
| `list_food_sources` | — | `SourceInfo[]` |
| `preview_food_import` | raw file bytes (see below), header `x-file-name` | `ImportPreview & { token }` |
| `commit_food_import` | `{ token, name?, replace_source_id? }` | `ImportReport` |
| `cancel_food_import` | `{ token }` | — |
| `reorder_food_sources` | `{ ids }` | — |
| `set_food_source_enabled` | `{ id, enabled }` | — |
| `rename_food_source` | `{ id, name }` | — |
| `delete_food_source` | `{ id }` | — |
| `list_foods` | `{ source_id?, query?, limit }` | `Food[]` |

**Getting the file to Rust.** The UI uses a plain `<input type="file" accept=".csv,.json">`
and sends the bytes with Tauri 2's raw IPC body:
`invoke('preview_food_import', new Uint8Array(await file.arrayBuffer()), { headers: { 'x-file-name': encodeURIComponent(file.name) } })`.
The command takes `tauri::ipc::Request` and reads `InvokeBody::Raw`. This needs no new Tauri
plugin, no filesystem scope, and no JSON-encoded byte arrays. The frontend never parses the
file, so there's no business logic in the UI.

**Preview → commit token.** `preview_food_import` parses once and stores the `ParsedImport` in
managed state, `PendingImport { token: Uuid, parsed, created_at }`. There is a single slot:
a new preview replaces the old one, and an entry expires after 15 minutes via the injected
`Clock`. `commit_food_import` consumes the token, so what the user previewed is exactly what
gets committed, and the file isn't sent twice. An unknown or expired token returns
`not_found` ("导入已过期，请重新选择文件": the import expired, choose the file again).

### 8.3 Desktop UI: Settings → 食物数据

A new `apps/desktop/ui/src/settings/FoodDataSection.tsx`, placed after 目标. Strings are added
to `i18n/zh-CN.ts`.

```
食物数据                                              [导入数据…]
来源按优先级从高到低使用，先匹配到的来源生效。

 1  手动录入            你在对话中修正过的食物        23 种   🔒
 2  中国食物成分表 v6   数据集 · 2026-09-30 导入     1,532 种  [开] ↑ ↓ ⋯
 3  my_snacks          数据集 · 2026-10-02 导入        41 种  [开] ↑ ↓ ⋯
 ─  AI 估算            找不到时由模型估算（兜底）      87 种   🔒
                                               ⋯ = 重命名 / 替换数据 / 导出 / 删除
```

- **导入数据… (import)** opens the file picker, then calls `preview_food_import` and shows a
  dialog with:
  - the source name field, prefilled with `suggested_name`;
  - count tiles: 可导入 N (importable) · 跳过 N (skipped) · 警告 N (warnings);
  - the "N 种将覆盖 AI 估算" (will override AI estimates) and "N 种已被手动录入覆盖"
    (already overridden by manual entries) notes;
  - a collapsible error/warning table (行 = line, 名称 = name, 问题 = problem);
  - the sample table (per 100 g).

  Buttons are 取消 (cancel) and 导入 (import). 导入 is disabled when there are 0 importable
  rows.
- **替换数据 (replace data)** runs the same flow with `replace_source_id` set. The dialog title
  says which source will be replaced.
- **删除 (delete)** uses the existing `ConfirmDeleteButton`, with the text
  "删除后已记录的饮食不受影响" (deleting doesn't affect meals already logged).
- **Reorder** uses ↑/↓ buttons that call `reorder_food_sources` with the full new order.
  There's no drag-and-drop in v1.
- **导出 (export)** is CLI-only in v1 (Q3), so the menu item is hidden until then.
- Empty state (no datasets): one muted line explaining that the user can import CSV or JSON,
  with a link to the column table (§6.1) reproduced as a short in-app help popover. The popover
  also says that energy columns are read as kcal unless the header says kJ, and that checking
  the unit is up to the user (§6.1).

Today page: meal items may show `source_name` (from `MealItem::to_json`, §5) as the
title/tooltip of the item row. The "估" badge is unchanged.

## 9. Known limit: name matching against large datasets

Matching is exact on `normalize(name)` plus aliases. The parser model names foods in its own
words ("米饭"), and it only sees the 150 most-used names. A freshly imported dataset that
calls the same food "稻米饭(蒸)" won't be hit, so the model estimates the food instead. This
isn't wrong, but it wastes the dataset.

Mitigations, in order of cost:

1. **Aliases in the data (v1).** The import format supports `aliases`. Documentation and the
   in-app help say so.
2. **Aliases after the fact (v1).** `diet_add_alias` through chat ("米饭就是稻米饭(蒸)") already
   works across sources (§7.3).
3. **Candidate step before estimating (later, opt-in).** On a miss in every non-LLM source,
   search candidates with an FTS5 table using the `trigram` tokenizer (substring matching that
   works for CJK), taking the top 8 across enabled datasets. Then ask the parser model to pick
   one or answer `none` (a small JSON call; the §9.3 prompt style). A pick stores a user alias
   automatically, so each name costs this call at most once. `none` falls through to the
   estimate as today. This is behind a setting because it adds one model call per new food
   name.

   *(Superseded by `docs/FOOD_MATCHING.md`, which uses in-memory character bigrams instead of
   FTS5 trigram, also matches names whose only hit is a cached estimate, and has no setting.)*

## 10. Implementation stages (suggested)

Each stage ends with `cargo fmt`, `clippy -D warnings`, `cargo test --workspace` green, and
a `DECISIONS.md` entry.

1. **Schema + chain, no visible change.** Migration 2, `SourceKind`, `FoodSource`,
   `TableSource`, `SourceChain`. Refactor `resolver`, `update_food`, `add_alias`,
   `known_names`, `FoodTable::list`. Add `MealItem.source_id/source_name`. Every existing test
   passes, updated only where they construct `NutritionSource` or read `Food.aliases`
   (§4.1). `FoodDataError` and its desktop mapping land in stage 2 and 3 respectively.
2. **Import core + CLI.** `import::{parse, preview, commit}`, `sources` management and export,
   and the `foods …` subcommands. Add the `csv` dependency.
3. **Desktop.** Commands, the pending-import state, `FoodDataSection`, and zh-CN strings.
4. *(Optional)* The §9.3 candidate step.
5. *(Optional)* A first `RemoteSource`.

## 11. Tests to write

All offline, with `MockChatModel`.

- **Migration:** a Phase 1 DB with user and estimate rows and aliases is migrated. Ids, values,
  use counts and aliases are preserved, and lookups return the same foods as before.
  `PRAGMA foreign_key_check` is empty afterwards, and deleting a dataset cascades into
  `diet_food_aliases` (this proves its FK names the final `diet_foods`).
- **Chain priority:** the same name in manual, dataset and cache, with manual winning. Disable
  or delete the higher copy and the next one wins. Reorder two datasets and the winner flips.
  An alias hit in a higher source beats a name hit in a lower one. If the single-query
  optimization (§4.2) is used, run the same cases against it.
- **Dangling user alias:** delete the target's dataset. The alias's name then resolves by
  itself (and is estimated under that name on a miss). Re-import the target and the redirect
  works again.
- **Import markers and energy:** `Tr` → 0 with a warning; `—` in a required column → row
  error, in `fiber_g` → 0; a bare `能量` header is read as kcal.
- **Desktop errors:** an import `Invalid`/`NotFound` reaches the `CommandError` with its zh-CN
  message unchanged.
- **Estimate fallback:** a name in no source is estimated once and stored under
  `llm_estimate`. A later dataset import containing it wins without a model call (assert that
  `received_requests()` is empty).
- **`diet_update_food`:** a name that exists only in a dataset → a new manual row, dataset row
  untouched. Through a user alias → written under the target key.
- **Aliases:** user alias redirect before the chain. A source alias only applies within its
  source and disappears on delete or replace. A user alias survives the target dataset being
  replaced. Chain/cycle rejection in both directions (§7.3).
- **`import::parse` (pure, table-driven):** BOM; non-UTF-8; header synonyms (Chinese,
  full-width parentheses); kJ conversion and kcal+kJ disagreement; `basis_g` scaling; each
  hard-rule error with its line number; duplicate names; alias collisions; quoted CSV fields
  with commas and newlines; JSON bare array vs envelope; row and size limits; zero valid rows.
- **Commit:** new source priority placement; replace keeps id, priority and name and carries
  `use_count`; a failure mid-commit leaves nothing written (transaction).
- **Old events:** a Phase 1 `diet.meal_logged` payload without `source_id` still deserializes,
  and `estimated` is unchanged.
- **Desktop:** raw-body `preview_food_import` round trip through the mock runtime; token
  consumed once; expired token → `not_found` (driven by the test clock).
- **Export → import round trip** yields identical rows.

## 12. Open questions

- **Q1. Can the user disable the AI estimate?** Useful for "only trust my data". Then an
  unknown food would fail with a message asking for values (the `diet_update_food` path). v1
  says no, to keep logging from ever dead-ending.
- **Q2. Should import into `manual` be allowed?** It would let a user's own spreadsheet sit
  above everything. v1 says no, so `manual` means "entered by hand" exactly. A dataset moved
  to position 2 gets almost the same effect.
- **Q3. Desktop export.** Needs a save dialog (`tauri-plugin-dialog`) or a download via a
  blob URL. It's deferred to keep v1 free of new Tauri plugins.
- **Q4. Non-UTF-8 CSV (GBK from older Excel).** Detecting and decoding it would need
  `encoding_rs`. v1 rejects it with a clear message instead.
- **Q5. Deleting a single food, or clearing AI estimates.** v1 has no per-row delete. A bad
  estimate can only be shadowed, by a `diet_update_food` correction or a dataset. Candidates
  for later: delete one row of `manual` or `llm_estimate` from the browse list, and a
  "clear AI estimate cache" action. Both are safe for history, because meals keep snapshots.
- **Q6. Restoring into `manual`.** Tied to Q2: a `manual` export can only come back as a
  dataset (§7.1). If Q2 changes, allow `foods import --into manual` for exports of `manual`
  only.
