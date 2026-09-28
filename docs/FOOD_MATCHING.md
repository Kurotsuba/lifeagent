# Design: matching food names against imported datasets

Status: **proposed 2026-09-28**. Written against branch `phase1` with `docs/DISH_NUTRITION.md`
fully implemented. Replaces mitigation 3 of `docs/NUTRITION_SOURCES.md` §9 (the FTS5 trigram
candidate step, never built) and extends that document's §5 (resolution) and §7.3 (user
aliases). Where they disagree, this document describes the new behavior.

## 1. Problem

Food lookup matches exactly on `normalize(name)` plus aliases (`NUTRITION_SOURCES.md` §5). The
parser names ingredients in its own words, and imported tables use theirs:

| Parser says | 中国食物成分表 says |
|---|---|
| 熟米饭 | 稻米饭(蒸) / 米饭(蒸，粳米) |
| 土豆 | 马铃薯 |
| 西红柿 | 番茄 |

So after a table is imported, almost every ingredient still misses and gets estimated by the
model. Two things make this worse over time:

- **Estimates stick.** A miss writes an `llm_estimate` row under the parser's name. From then on
  that row is an exact hit, so the table is never tried for that name again.
- **`KNOWN_FOODS` doesn't surface the table.** The parser sees the 150 most-used names. Imported
  rows start at `use_count = 0`, so the list is filled by earlier estimates, then by table rows
  in alphabetical order.

A user test on 2026-09-28 confirmed this: with 中国食物成分表 (标准版, 第六版) imported, meal
ingredients were still estimated.

## 2. Goal and decisions

When a name has no hit in `manual` or any dataset, find a few similar rows in those sources and
let the model pick the same food, or none. A pick is saved as an alias, so each name costs this
at most once.

| # | Question | Decision |
|---|---|---|
| F1 | How to find candidates | **Character bigrams, in memory** (§4). Not FTS5 trigram, and not English translation. See §2.1. |
| F2 | When a cached estimate is the only hit | **Still look for candidates.** Otherwise every name estimated before the import stays stuck (§1). |
| F3 | Remembering the answer | A pick is stored in `diet_name_aliases` with `origin = 'match'`. A "none" is stored in a new check table, and it is retried only when a dataset changes (§6). |
| F4 | Synonyms written with other characters (土豆/马铃薯) | The parser gives each component up to 2 **alternate Chinese names** (`alt_names`), which are searched too (§3). |
| F5 | Raw vs cooked | A candidate in a different state is **not** the same food (§5). |
| F6 | Setting to turn it off | **None.** The cost is bounded: at most one call per meal, only for names never seen before (§2.2). |

### 2.1 Why not trigram, or English

- **Trigram:** three-character windows are too coarse for short Chinese names. `熟米饭` has one
  trigram, `熟米饭`, which isn't in `稻米饭(蒸)`. SQLite's `trigram` tokenizer also can't search
  for strings shorter than 3 characters, so `鸡蛋`, `牛肉` and `米饭` would find nothing. Bigrams
  work: `熟米饭` → {熟米, 米饭} shares `米饭` with `稻米饭(蒸)`.
- **English:** both sides would need translating: every table row once, and every ingredient
  on every meal. The translations would be inconsistent ("lean beef" / "beef, lean"), and the
  mismatch is in the wording, not the script. The one case English helps with, synonyms written
  with different characters, is covered more cheaply by `alt_names` (F4).

### 2.2 Cost

- One extra model call **per `resolve_many` call**, not per food, and only when at least one
  name is new (no alias, no fresh check) and has candidates.
- Each name costs that at most once until a dataset changes (§6).
- With no dataset imported, the pool is only the user's `manual` rows, which are usually few,
  so a call happens only when a new name shares characters with one of them. That's rare,
  and it's still a real match opportunity (`熟米饭` vs a manual `米饭(自家电饭煲)`).

## 3. Parser change: `alt_names`

Each component in `MEAL_PARSER`'s output (`docs/DISH_NUTRITION.md` §5.1) gains an optional
field:

```json
{"name": "土豆", "grams": 150, "alt_names": ["马铃薯"]}
```

Prompt rule: *`alt_names`: up to 2 other common or standard Chinese names for the same food in
the same state (e.g. 土豆 → 马铃薯, 西红柿 → 番茄, 熟米饭 → 米饭(蒸)). Omit it when there
are none. Never use it for a different food or a different state.*

`validate_dishes` trims them, drops empty or duplicate entries and ones equal to the name, and
keeps the first 2. They aren't an error when invalid. `alt_names` are used **only** as search
queries (§4). They're never looked up exactly, never stored and never shown.

`resolver::resolve_many(ctx, names: &[String])` becomes
`resolve_many(ctx, queries: &[FoodQuery])`:

```rust
pub struct FoodQuery {
    pub name: String,
    pub alt_names: Vec<String>,
}
```

Plan validation (`validate.rs`, the other caller) passes an empty `alt_names`, so plans get the
matching step too, searching by name only.

## 4. Candidates

**Pool.** The winning food per `name_key` across enabled `manual` and `dataset` sources (the
same grouping as `known_names`), together with each winner's source-scoped aliases from
`diet_food_aliases`. `llm_estimate` rows are never candidates. The pool is loaded once per
`resolve_many` call, and only if some name needs matching. For a 50 000-row import that's
one query of a few tens of milliseconds.

**Index.** An in-memory map from bigram to pool rows, built from each row's `name_key` and
alias keys. A string shorter than 2 characters is its own single token.

**Ranking.** For each query string (the name and each `alt_name`, all normalized), score only
the rows that share at least one token with it, using the character-bigram Dice coefficient
already in `dish_resolver::dice_score` (moved to `foods.rs` and shared). A row's score is its
best score over its name, its aliases and all query strings. Keep the top **8**, ties broken by
`use_count` descending, then name. A name with no row sharing a token has no candidates.

## 5. The match call

New prompt `FOOD_MATCH`, role Parser, JSON, with **one call per `resolve_many`** covering every
pending name. It runs through `complete_json_once`: there's no repair attempt, and a failed or
wrong-shaped answer counts as `null` for every name (the same rule as the dish pick,
`DECISIONS.md` 2026-09-28).

Input, with candidates numbered explicitly:

```json
[{"food": 0, "name": "熟米饭", "candidates": [
   {"index": 0, "name": "稻米饭(蒸)"}, {"index": 1, "name": "稻米(粳，标一)"}, …]}]
```

Output: `{"matches": [{"food": 0, "candidate": 0}, {"food": 1, "candidate": null}]}`.

Prompt rules:

- Choose a candidate only if it is the **same food in the same state**. Raw/dry vs cooked is a
  different food (`大米(生)` is not `熟米饭`; raw `牛肉(瘦)` is not `牛肉(熟)`), because the
  per-100 g values differ a lot. A candidate with no stated state counts as the table's usual
  form, which is raw for meat, grains and vegetables in a composition table.
- A more specific variety of the same food is fine (`稻米饭(蒸)` for `熟米饭`). A different food
  that shares characters is not (`米粉` for `米饭`).
- When unsure, answer `null`: an estimate is better than a wrong row.
- Copy the candidate's `index` value exactly.

Out-of-range indices and names missing from the output count as `null`.

**Expected effect.** The state rule means raw-ingredient rows won't match cooked components.
Composition tables are mostly raw ingredients, and the parser gives cooked weights for meat and
cooked grains, so those will often still be estimated. That's correct, and better than applying
raw values to cooked grams. Foods that the table lists as eaten (rice, bread, fruit,
vegetables, milk, eggs, tofu, drinks) should match.

## 6. Resolution flow (replaces `NUTRITION_SOURCES.md` §5 step 4)

For each query in `resolve_many`:

1. Redirect and chain walk, exactly as today, giving the winner `W` or a miss.
2. `W` from `manual` or a dataset → done.
3. Otherwise (`W` is a cached estimate, or a miss), check `diet_food_match_checks` for `key`.
   If the check is **fresh** (§6.1), skip to step 6.
4. Build the candidates (§4). With none, record a check and go to step 6.
5. The name goes into the batch for the one `FOOD_MATCH` call.
   - **Pick:** store the alias (§6.2) and use the picked food.
   - **`null` from a valid answer:** record a check.
   - **Failed call:** record nothing, so a transient failure is retried on the next meal.
6. Fall back as today: use `W` (the cached estimate) if there is one, otherwise estimate the
   food and insert it into `llm_estimate`.
7. `record_use` on each winner.

The match call runs before the estimates, so a matched name never costs an estimate call.

### 6.1 Checks

```sql
-- diet plugin migration 4
CREATE TABLE diet_food_match_checks (
  name_key    TEXT PRIMARY KEY,
  checked_at  TEXT NOT NULL      -- UTC RFC 3339, from the injected Clock
);
ALTER TABLE diet_name_aliases ADD COLUMN origin TEXT NOT NULL DEFAULT 'user';  -- 'user' | 'match'
```

A check is **stale** when any `dataset` source has `updated_at > checked_at`. That covers
import, replace, enable and disable. Reordering also bumps `updated_at`, which causes a harmless
re-check. Deleting a dataset can't create a new match, so it needs nothing. Manual edits need
nothing either: a manual row under the same key is an exact hit in step 2.

Recording a check upserts `checked_at = now`.

### 6.2 Storing a pick

A pick is stored as `diet_name_aliases(alias_key = key, target_key = picked name_key,
origin = 'match')`, keeping `NUTRITION_SOURCES.md` §7.3's one-hop rule:

- If `target_key` is itself an alias key, the pick is used for this meal but not stored. This
  can't normally happen, since candidates are real food rows, but the rule keeps the invariant.
- If `key` is already the target of other aliases (the user once said `米饭` = `熟米饭`, and
  `熟米饭` now matches `稻米饭(蒸)`), those aliases are re-pointed to the new `target_key` in the
  same transaction, so no chain forms.

The shadowed `llm_estimate` row for `key` stays in the cache, as any shadowed row does.

## 7. Undoing a wrong match

A wrong pick has to be removable. Otherwise the redirect wins forever: it's applied before the
chain walk, and even `diet_update_food` writes through it (it upserts under the target key).

New tool `diet_remove_alias { alias }`:

- It deletes the redirect for `normalize(alias)`, whatever its origin, and records a check for
  that key. That stops the next meal from matching the same row again until a dataset changes.
- Tool description example: the user says "熟米饭不是那个稻米饭(蒸)，按估算算" → remove the
  alias.
- `NotFound` when there's no such alias.

The `foods lookup` CLI output gains the redirect, if any, with its origin, so a match can be
inspected.

## 8. Out of scope

- **Dishes.** `docs/DISH_NUTRITION.md` has its own pick step. Unmatched dishes' components go
  through `resolve_many`, so they benefit from this step automatically.
- **Changing `known_names`.** Once a name is matched, lookups hit the dataset row and its
  `use_count` grows, so the table's names reach `KNOWN_FOODS` naturally.
- **A settings toggle** (F6).
- **A UI to browse or edit aliases.** Chat (`diet_add_alias` / `diet_remove_alias`) and the CLI
  cover it for now.

## 9. Code changes

| Area | Files | Change |
|---|---|---|
| Schema | `plugin-diet/src/lib.rs` (+ migration file) | Migration 4 (§6.1) |
| Parser | `parser.rs`, `prompts.rs` | `alt_names` on components, prompt rule, validation (§3) |
| Similarity | `foods.rs`, `dish_resolver.rs` | Move `bigrams`/`dice_score` to `foods.rs`, add the token index |
| Matching | **new** `food_matcher.rs` | Pool, candidates, `FOOD_MATCH` call, checks, alias write (§4–§6) |
| Resolver | `resolver.rs` | `FoodQuery`, run the match step before estimating (§6) |
| Callers | `tools.rs`, `validate.rs` | Pass `FoodQuery`s |
| Aliases | `sources.rs`, `tools.rs` | `origin` on writes, `remove_alias`, `diet_remove_alias` tool (§7) |
| CLI | `crates/cli/src/main.rs` | `foods lookup` shows the redirect and its origin |

## 10. Tests (all with `MockChatModel`)

- **Ranking:** `熟米饭` ranks `稻米饭(蒸)` above unrelated rows. A 1-character name (`梨`) finds
  `梨(平均)`. A name sharing no bigram has no candidates and makes no call. Estimate rows never
  appear. A disabled dataset's rows never appear. Only the top 8 are sent.
- **alt_names:** `土豆` with `alt_names: ["马铃薯"]` gets `马铃薯(土豆)` as a candidate, and
  without `alt_names` it doesn't. Validation drops a third entry, blanks and duplicates.
- **Pick:** a mocked pick logs the dataset row, stores the alias with `origin = 'match'`, and
  makes no estimate call. The same name on the next meal makes no match call.
- **Stuck estimate (F2):** a name with an existing `llm_estimate` row and a new dataset
  containing the food gets matched to the dataset row.
- **Null:** it records a check, the meal uses the estimate, and the next meal makes no match
  call. After a new import (a newer `updated_at`), the name is checked again.
- **Failure:** a wrong-shaped answer or a provider error gives an estimate, records no check,
  and makes exactly one match request (no repair).
- **One call per resolve:** three pending names share one request.
- **Chain rule:** with a user alias `米饭 → 熟米饭`, matching `熟米饭 → 稻米饭(蒸)` re-points
  `米饭` to `稻米饭(蒸)`.
- **Undo:** `diet_remove_alias` deletes the redirect and records a check, and the next meal
  makes no match call. It returns `NotFound` for an unknown alias.
- **Plans:** plan validation with a dataset-only food name matches it (empty `alt_names`).
- **Prompt:** it contains the state rule and explicit `"index"` fields.

## 11. Stages

1. **Matching core.** Migration 4, the similarity move, `food_matcher.rs`, the resolver
   integration, `FoodQuery` (callers pass empty `alt_names`) and the tests above except
   `alt_names`.
2. **Parser `alt_names`** and their tests.
3. **Undo and inspection:** `diet_remove_alias` and `foods lookup`.

Each stage ends with `cargo fmt`, `clippy -D warnings`, `cargo test --workspace` green and a
`DECISIONS.md` entry.

## 12. Open questions

- **Q1. Skip the call on an exact `alt_name` hit?** If `马铃薯` is an exact dataset name, the
  model already said it's the same food. Using it directly would save a call, but it would trust
  `alt_names` without a check. v1 routes it through the pick like any other candidate. It
  ranks first, since its Dice score is 1.
- **Q2. Pool size.** Several large imports could make the pool hundreds of thousands of rows.
  One query plus an in-memory index is fine at the current 50 000-row import limit. Revisit if
  that limit grows.
