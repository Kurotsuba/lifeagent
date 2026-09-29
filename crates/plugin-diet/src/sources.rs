//! Pluggable nutrition data sources: the source chain, resolution and source management, per
//! `docs/NUTRITION_SOURCES.md` §2–§7, extended by `docs/DISH_NUTRITION.md` §3–§4 with per-serving
//! dishes alongside per-100 g foods.
//!
//! `foods.rs` keeps `normalize` and the §8.3 nutrition checks; this module owns everything about
//! *where* a food's or dish's numbers come from: the `diet_food_sources` / `diet_foods` /
//! `diet_food_aliases` / `diet_name_aliases` / `diet_dishes` / `diet_dish_source_aliases` /
//! `diet_dish_name_aliases` tables, the `FoodSource` trait and its `TableSource` implementation,
//! `SourceChain` resolution, and the management operations behind the CLI's `foods source …`
//! commands and the desktop's 食物数据 settings section.

use std::collections::HashSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use lifeagent_core::plugin::{PluginContext, PluginError};

use crate::foods::normalize;
use crate::model::{Dish, Macros, SourceKind};

pub const MANUAL_SOURCE_ID: &str = "manual";
pub const LLM_ESTIMATE_SOURCE_ID: &str = "llm_estimate";

/// Diet plugin migration 2: rebuilds `diet_foods` around a chain of sources, per
/// `docs/NUTRITION_SOURCES.md` §3.
pub const MIGRATION_2: &str = "
CREATE TABLE diet_food_sources (
  id           TEXT PRIMARY KEY,
  kind         TEXT NOT NULL,
  name         TEXT NOT NULL,
  priority     INTEGER NOT NULL,
  enabled      INTEGER NOT NULL DEFAULT 1,
  origin_json  TEXT NOT NULL DEFAULT '{}',
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

INSERT INTO diet_food_sources (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
VALUES
  ('manual', 'manual', '手动录入', 0, 1, '{}',
   strftime('%Y-%m-%dT%H:%M:%S+00:00','now'), strftime('%Y-%m-%dT%H:%M:%S+00:00','now')),
  ('llm_estimate', 'llm_estimate', 'AI 估算', 1000000, 1, '{}',
   strftime('%Y-%m-%dT%H:%M:%S+00:00','now'), strftime('%Y-%m-%dT%H:%M:%S+00:00','now'));

CREATE TABLE diet_foods_v2 (
  id          TEXT PRIMARY KEY,
  source_id   TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL,
  kcal        REAL NOT NULL,
  protein_g   REAL NOT NULL,
  carbs_g     REAL NOT NULL,
  fat_g       REAL NOT NULL,
  fiber_g     REAL NOT NULL DEFAULT 0,
  use_count   INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (source_id, name_key)
);

INSERT INTO diet_foods_v2
  (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, use_count,
   created_at, updated_at)
SELECT id,
       CASE source WHEN 'user' THEN 'manual' WHEN 'llm_estimate' THEN 'llm_estimate' END,
       name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, use_count, created_at, updated_at
FROM diet_foods;

CREATE TABLE diet_name_aliases (
  alias_key   TEXT PRIMARY KEY,
  target_key  TEXT NOT NULL,
  created_at  TEXT NOT NULL
);

INSERT INTO diet_name_aliases (alias_key, target_key, created_at)
SELECT j.value, f.name_key, f.updated_at
FROM diet_foods f, json_each(f.aliases_json) j;

DROP TABLE diet_foods;

ALTER TABLE diet_foods_v2 RENAME TO diet_foods;

CREATE INDEX diet_foods_name_key ON diet_foods(name_key);

CREATE TABLE diet_food_aliases (
  source_id  TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  alias_key  TEXT NOT NULL,
  food_id    TEXT NOT NULL REFERENCES diet_foods(id) ON DELETE CASCADE,
  PRIMARY KEY (source_id, alias_key)
);
";

/// Diet plugin migration 3: per-serving dishes alongside per-100 g foods, per
/// `docs/DISH_NUTRITION.md` §3. Additive only — no existing table or row changes, so no rebuild
/// is needed.
pub const MIGRATION_3: &str = "
CREATE TABLE diet_dishes (
  id          TEXT PRIMARY KEY,
  source_id   TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  brand       TEXT NOT NULL DEFAULT '',
  brand_key   TEXT NOT NULL DEFAULT '',
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL,
  serving_g   REAL,
  kcal        REAL NOT NULL,
  protein_g   REAL NOT NULL,
  carbs_g     REAL NOT NULL,
  fat_g       REAL NOT NULL,
  fiber_g     REAL NOT NULL DEFAULT 0,
  use_count   INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (source_id, brand_key, name_key)
);

CREATE INDEX diet_dishes_brand_name ON diet_dishes(brand_key, name_key);

CREATE TABLE diet_dish_source_aliases (
  source_id  TEXT NOT NULL REFERENCES diet_food_sources(id) ON DELETE CASCADE,
  brand_key  TEXT NOT NULL,
  alias_key  TEXT NOT NULL,
  dish_id    TEXT NOT NULL REFERENCES diet_dishes(id) ON DELETE CASCADE,
  PRIMARY KEY (source_id, brand_key, alias_key)
);

CREATE TABLE diet_dish_name_aliases (
  brand_key        TEXT NOT NULL,
  alias_key        TEXT NOT NULL,
  target_name_key  TEXT NOT NULL,
  origin           TEXT NOT NULL,
  created_at       TEXT NOT NULL,
  PRIMARY KEY (brand_key, alias_key)
);
";

/// Diet plugin migration 4: remembers which names were already checked against the imported
/// data, and where a redirect came from, per `docs/FOOD_MATCHING.md` §6.1. Additive only.
pub const MIGRATION_4: &str = "
CREATE TABLE diet_food_match_checks (
  name_key    TEXT PRIMARY KEY,
  checked_at  TEXT NOT NULL
);

ALTER TABLE diet_name_aliases ADD COLUMN origin TEXT NOT NULL DEFAULT 'user';
";

/// A dataset source's provenance, stored as `diet_food_sources.origin_json`. `manual` and
/// `llm_estimate` store `{}`, which decodes to `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetOrigin {
    pub file_name: String,
    pub format: String,
    pub imported_at: String,
    pub rows: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub kind: SourceKind,
    pub name: String,
    pub priority: i64,
    pub enabled: bool,
    pub food_count: i64,
    pub dish_count: i64,
    pub origin: Option<DatasetOrigin>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Food {
    pub id: String,
    pub name: String,
    pub name_key: String,
    pub per_100g: Macros,
    pub source_id: String,
    pub source_kind: SourceKind,
    pub source_name: String,
    pub use_count: i64,
}

/// A named provider of per-100 g nutrition rows, and (`docs/DISH_NUTRITION.md` §4.2) optionally
/// per-serving dish rows.
#[async_trait]
pub trait FoodSource: Send + Sync {
    fn info(&self) -> &SourceInfo;

    /// Exact match on a normalized key, by name first and then by this source's own aliases.
    /// Never calls a model. `Ok(None)` is a miss.
    async fn lookup(&self, key: &str) -> Result<Option<Food>, PluginError>;

    /// Called on the food that won a resolution. DB-backed sources bump `use_count`.
    async fn record_use(&self, _food: &Food) -> Result<(), PluginError> {
        Ok(())
    }

    /// Exact match on `(brand_key, name_key)`, then this source's dish aliases. `Ok(None)` is a
    /// miss. The default "always miss" body means a source that never carries dishes (a later
    /// `RemoteSource`, or `llm_estimate`, which never has dish rows per D3) needs no override.
    async fn lookup_dish(&self, _brand_key: &str, _key: &str) -> Result<Option<Dish>, PluginError> {
        Ok(None)
    }

    /// All dishes of one brand in this source, for the candidate step (§5.3).
    async fn brand_dishes(&self, _brand_key: &str) -> Result<Vec<Dish>, PluginError> {
        Ok(vec![])
    }

    /// Called on the dish that won a resolution. DB-backed sources bump `use_count`.
    async fn record_dish_use(&self, _dish: &Dish) -> Result<(), PluginError> {
        Ok(())
    }
}

/// (id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, use_count)
type FoodRow = (String, String, String, f64, f64, f64, f64, f64, i64);

/// (id, brand, brand_key, name, name_key, serving_g, kcal, protein_g, carbs_g, fat_g, fiber_g,
///  use_count)
#[allow(clippy::type_complexity)]
type DishRow = (
    String,
    String,
    String,
    String,
    String,
    Option<f64>,
    f64,
    f64,
    f64,
    f64,
    f64,
    i64,
);

/// Backs `manual`, every `dataset`, and the `llm_estimate` cache: one indexed query per lookup.
pub struct TableSource {
    info: SourceInfo,
    db: SqlitePool,
}

impl TableSource {
    pub fn new(info: SourceInfo, db: SqlitePool) -> Self {
        Self { info, db }
    }

    fn row_to_food(&self, row: FoodRow) -> Food {
        let (id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, use_count) = row;
        Food {
            id,
            name,
            name_key,
            per_100g: Macros {
                kcal,
                protein_g,
                carbs_g,
                fat_g,
                fiber_g,
            },
            source_id: self.info.id.clone(),
            source_kind: self.info.kind,
            source_name: self.info.name.clone(),
            use_count,
        }
    }

    fn row_to_dish(&self, row: DishRow) -> Dish {
        let (
            id,
            brand,
            brand_key,
            name,
            name_key,
            serving_g,
            kcal,
            protein_g,
            carbs_g,
            fat_g,
            fiber_g,
            use_count,
        ) = row;
        Dish {
            id,
            brand,
            brand_key,
            name,
            name_key,
            serving_g,
            per_serving: Macros {
                kcal,
                protein_g,
                carbs_g,
                fat_g,
                fiber_g,
            },
            source_id: self.info.id.clone(),
            source_kind: self.info.kind,
            source_name: self.info.name.clone(),
            use_count,
        }
    }
}

#[async_trait]
impl FoodSource for TableSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    async fn lookup(&self, key: &str) -> Result<Option<Food>, PluginError> {
        if key.is_empty() {
            return Ok(None);
        }
        let by_name: Option<FoodRow> = sqlx::query_as(
            "SELECT id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, use_count
             FROM diet_foods WHERE source_id = ? AND name_key = ?",
        )
        .bind(&self.info.id)
        .bind(key)
        .fetch_optional(&self.db)
        .await?;
        let row = match by_name {
            Some(row) => Some(row),
            None => {
                sqlx::query_as(
                    "SELECT f.id, f.name, f.name_key, f.kcal, f.protein_g, f.carbs_g, f.fat_g,
                            f.fiber_g, f.use_count
                     FROM diet_food_aliases a JOIN diet_foods f ON f.id = a.food_id
                     WHERE a.source_id = ? AND a.alias_key = ?",
                )
                .bind(&self.info.id)
                .bind(key)
                .fetch_optional(&self.db)
                .await?
            }
        };
        Ok(row.map(|r| self.row_to_food(r)))
    }

    async fn record_use(&self, food: &Food) -> Result<(), PluginError> {
        sqlx::query("UPDATE diet_foods SET use_count = use_count + 1 WHERE id = ?")
            .bind(&food.id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    async fn lookup_dish(&self, brand_key: &str, key: &str) -> Result<Option<Dish>, PluginError> {
        if key.is_empty() {
            return Ok(None);
        }
        let by_name: Option<DishRow> = sqlx::query_as(
            "SELECT id, brand, brand_key, name, name_key, serving_g, kcal, protein_g, carbs_g,
                    fat_g, fiber_g, use_count
             FROM diet_dishes WHERE source_id = ? AND brand_key = ? AND name_key = ?",
        )
        .bind(&self.info.id)
        .bind(brand_key)
        .bind(key)
        .fetch_optional(&self.db)
        .await?;
        let row =
            match by_name {
                Some(row) => Some(row),
                None => sqlx::query_as(
                    "SELECT d.id, d.brand, d.brand_key, d.name, d.name_key, d.serving_g, d.kcal,
                            d.protein_g, d.carbs_g, d.fat_g, d.fiber_g, d.use_count
                     FROM diet_dish_source_aliases a JOIN diet_dishes d ON d.id = a.dish_id
                     WHERE a.source_id = ? AND a.brand_key = ? AND a.alias_key = ?",
                )
                .bind(&self.info.id)
                .bind(brand_key)
                .bind(key)
                .fetch_optional(&self.db)
                .await?,
            };
        Ok(row.map(|r| self.row_to_dish(r)))
    }

    async fn brand_dishes(&self, brand_key: &str) -> Result<Vec<Dish>, PluginError> {
        let rows: Vec<DishRow> = sqlx::query_as(
            "SELECT id, brand, brand_key, name, name_key, serving_g, kcal, protein_g, carbs_g,
                    fat_g, fiber_g, use_count
             FROM diet_dishes WHERE source_id = ? AND brand_key = ?",
        )
        .bind(&self.info.id)
        .bind(brand_key)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(|r| self.row_to_dish(r)).collect())
    }

    async fn record_dish_use(&self, dish: &Dish) -> Result<(), PluginError> {
        sqlx::query("UPDATE diet_dishes SET use_count = use_count + 1 WHERE id = ?")
            .bind(&dish.id)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

/// Enabled sources in priority order (§4.3).
pub struct SourceChain {
    sources: Vec<Box<dyn FoodSource>>,
    db: SqlitePool,
}

impl SourceChain {
    /// Enabled sources in priority order, built from `diet_food_sources`. Costs one small query,
    /// so reorder/enable/import take effect immediately with no cache to invalidate.
    pub async fn load(ctx: &PluginContext) -> Result<Self, PluginError> {
        let rows: Vec<(String, String, String, i64, String)> = sqlx::query_as(
            "SELECT id, kind, name, priority, origin_json FROM diet_food_sources
             WHERE enabled = 1 ORDER BY priority ASC",
        )
        .fetch_all(&ctx.db)
        .await?;
        let mut sources: Vec<Box<dyn FoodSource>> = Vec::with_capacity(rows.len());
        for (id, kind, name, priority, origin_json) in rows {
            let kind = parse_kind(&kind)?;
            let info = SourceInfo {
                id,
                kind,
                name,
                priority,
                enabled: true,
                food_count: 0,
                dish_count: 0,
                origin: parse_origin(&origin_json)?,
            };
            // Only `TableSource` exists today; a later kind (§4.4) adds a branch here.
            sources.push(Box::new(TableSource::new(info, ctx.db.clone())));
        }
        Ok(SourceChain {
            sources,
            db: ctx.db.clone(),
        })
    }

    /// User alias redirect, then the first hit in priority order (§5 steps 2–3). Within one
    /// source, a name match beats an alias match; across sources, priority comes first.
    pub async fn lookup(&self, name: &str) -> Result<Option<Food>, PluginError> {
        let key = normalize(name);
        if key.is_empty() {
            return Ok(None);
        }
        if let Some(target_key) = self.alias_target(&key).await? {
            if let Some(food) = self.walk(&target_key).await? {
                return Ok(Some(food));
            }
            // Dangling: the target's source was deleted or disabled. Fall through to the key
            // itself rather than erroring — it works again once the target comes back.
        }
        self.walk(&key).await
    }

    /// Every copy of `name` across all sources, winners first: enabled sources in priority
    /// order, then disabled sources in priority order. For `foods lookup` / debugging.
    pub async fn lookup_all(ctx: &PluginContext, name: &str) -> Result<Vec<Food>, PluginError> {
        let key = normalize(name);
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, String, String, i64, String)> = sqlx::query_as(
            "SELECT id, kind, name, priority, origin_json FROM diet_food_sources
             ORDER BY enabled DESC, priority ASC",
        )
        .fetch_all(&ctx.db)
        .await?;
        let mut out = Vec::new();
        for (id, kind, name, priority, origin_json) in rows {
            let info = SourceInfo {
                id,
                kind: parse_kind(&kind)?,
                name,
                priority,
                enabled: true,
                food_count: 0,
                dish_count: 0,
                origin: parse_origin(&origin_json)?,
            };
            let source = TableSource::new(info, ctx.db.clone());
            if let Some(food) = source.lookup(&key).await? {
                out.push(food);
            }
        }
        Ok(out)
    }

    /// Every copy of `(brand, name)` across all sources, winners first — the dish counterpart of
    /// `lookup_all`, for `foods lookup` / debugging (`docs/DISH_NUTRITION.md` §8.1). No alias
    /// redirect, same as `lookup_all`.
    pub async fn lookup_dish_all(
        ctx: &PluginContext,
        brand: &str,
        name: &str,
    ) -> Result<Vec<Dish>, PluginError> {
        let brand_key = normalize(brand);
        let key = normalize(name);
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, String, String, i64, String)> = sqlx::query_as(
            "SELECT id, kind, name, priority, origin_json FROM diet_food_sources
             ORDER BY enabled DESC, priority ASC",
        )
        .fetch_all(&ctx.db)
        .await?;
        let mut out = Vec::new();
        for (id, kind, name, priority, origin_json) in rows {
            let info = SourceInfo {
                id,
                kind: parse_kind(&kind)?,
                name,
                priority,
                enabled: true,
                food_count: 0,
                dish_count: 0,
                origin: parse_origin(&origin_json)?,
            };
            let source = TableSource::new(info, ctx.db.clone());
            if let Some(dish) = source.lookup_dish(&brand_key, &key).await? {
                out.push(dish);
            }
        }
        Ok(out)
    }

    /// The first hit in priority order for an already-normalized key, with no alias redirect
    /// (`docs/FOOD_MATCHING.md` §6.2: a picked target must not be redirected again).
    pub(crate) async fn lookup_direct(&self, key: &str) -> Result<Option<Food>, PluginError> {
        self.walk(key).await
    }

    async fn walk(&self, key: &str) -> Result<Option<Food>, PluginError> {
        for source in &self.sources {
            if let Some(food) = source.lookup(key).await? {
                return Ok(Some(food));
            }
        }
        Ok(None)
    }

    async fn alias_target(&self, key: &str) -> Result<Option<String>, PluginError> {
        let target: Option<(String,)> =
            sqlx::query_as("SELECT target_key FROM diet_name_aliases WHERE alias_key = ?")
                .bind(key)
                .fetch_optional(&self.db)
                .await?;
        Ok(target.map(|(t,)| t))
    }

    /// Bumps `use_count` on the source that produced `food`. A no-op if that source was
    /// disabled or deleted between lookup and this call.
    pub async fn record_use(&self, food: &Food) -> Result<(), PluginError> {
        if let Some(source) = self.sources.iter().find(|s| s.info().id == food.source_id) {
            source.record_use(food).await?;
        }
        Ok(())
    }

    /// The user/pick redirect in `diet_dish_name_aliases`, then the first hit in priority order
    /// (`docs/DISH_NUTRITION.md` §4.3). Mirrors `lookup`, including the dangling-alias
    /// fall-through. A `brand` of `""` matches only brandless dishes (§2 rule 3).
    pub async fn lookup_dish(&self, brand: &str, name: &str) -> Result<Option<Dish>, PluginError> {
        let brand_key = normalize(brand);
        let key = normalize(name);
        if key.is_empty() {
            return Ok(None);
        }
        if let Some(target_key) = self.dish_alias_target(&brand_key, &key).await? {
            if let Some(dish) = self.walk_dish(&brand_key, &target_key).await? {
                return Ok(Some(dish));
            }
            // Dangling: the target's source was deleted or disabled. Fall through to the key
            // itself rather than erroring — it works again once the target comes back.
        }
        self.walk_dish(&brand_key, &key).await
    }

    async fn walk_dish(&self, brand_key: &str, key: &str) -> Result<Option<Dish>, PluginError> {
        for source in &self.sources {
            if let Some(dish) = source.lookup_dish(brand_key, key).await? {
                return Ok(Some(dish));
            }
        }
        Ok(None)
    }

    async fn dish_alias_target(
        &self,
        brand_key: &str,
        key: &str,
    ) -> Result<Option<String>, PluginError> {
        let target: Option<(String,)> = sqlx::query_as(
            "SELECT target_name_key FROM diet_dish_name_aliases WHERE brand_key = ? AND alias_key = ?",
        )
        .bind(brand_key)
        .bind(key)
        .fetch_optional(&self.db)
        .await?;
        Ok(target.map(|(t,)| t))
    }

    /// Bumps `use_count` on the source that produced `dish`. A no-op if that source was disabled
    /// or deleted between lookup and this call.
    pub async fn record_dish_use(&self, dish: &Dish) -> Result<(), PluginError> {
        if let Some(source) = self.sources.iter().find(|s| s.info().id == dish.source_id) {
            source.record_dish_use(dish).await?;
        }
        Ok(())
    }

    /// The winning copy of each of `brand_key`'s dishes across enabled sources, in priority
    /// order (`docs/DISH_NUTRITION.md` §4.2), for the §5.3 candidate step. `self.sources` is
    /// already priority-ordered, so the first copy seen of a `name_key` is the winner and later
    /// (shadowed) copies are skipped.
    pub async fn brand_candidates(&self, brand_key: &str) -> Result<Vec<Dish>, PluginError> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for source in &self.sources {
            for dish in source.brand_dishes(brand_key).await? {
                if seen.insert(dish.name_key.clone()) {
                    out.push(dish);
                }
            }
        }
        Ok(out)
    }

    /// Distinct display brands of enabled sources' dishes, ordered by summed `use_count` (over
    /// every menu item of that brand) then by name (§4.2), for the parser's `{known_brands}`.
    pub async fn known_brands(&self, limit: u32) -> Result<Vec<String>, PluginError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "WITH enabled_dishes AS (
                SELECT d.brand, d.brand_key, d.use_count, s.priority
                FROM diet_dishes d JOIN diet_food_sources s ON d.source_id = s.id
                WHERE s.enabled = 1 AND d.brand_key != ''
             ), ranked AS (
                SELECT brand, brand_key,
                       ROW_NUMBER() OVER (PARTITION BY brand_key ORDER BY priority ASC) AS rn,
                       SUM(use_count) OVER (PARTITION BY brand_key) AS total_uses
                FROM enabled_dishes
             )
             SELECT brand FROM ranked WHERE rn = 1 ORDER BY total_uses DESC, brand ASC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(|(b,)| b).collect())
    }

    /// Display names of **brandless** dishes of enabled sources, one per `name_key` (the copy
    /// from the highest-priority source), ordered by summed `use_count` then by name (§4.2), for
    /// the parser's `{known_dishes}`. Branded dishes are excluded: the parser reaches them
    /// through `{known_brands}` and the §5.3 pick step.
    pub async fn known_dishes(&self, limit: u32) -> Result<Vec<String>, PluginError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "WITH enabled_dishes AS (
                SELECT d.name, d.name_key, d.use_count, s.priority
                FROM diet_dishes d JOIN diet_food_sources s ON d.source_id = s.id
                WHERE s.enabled = 1 AND d.brand_key = ''
             ), ranked AS (
                SELECT name, name_key,
                       ROW_NUMBER() OVER (PARTITION BY name_key ORDER BY priority ASC) AS rn,
                       SUM(use_count) OVER (PARTITION BY name_key) AS total_uses
                FROM enabled_dishes
             )
             SELECT name FROM ranked WHERE rn = 1 ORDER BY total_uses DESC, name ASC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }
}

fn parse_kind(s: &str) -> Result<SourceKind, PluginError> {
    SourceKind::from_db_str(s).ok_or_else(|| PluginError::Other(format!("bad source kind {s:?}")))
}

fn parse_origin(json: &str) -> Result<Option<DatasetOrigin>, PluginError> {
    if json == "{}" {
        return Ok(None);
    }
    serde_json::from_str(json)
        .map(Some)
        .map_err(|e| PluginError::Other(format!("bad origin_json in db: {e}")))
}

// ---- Resolution helpers used by `resolver.rs` ------------------------------------------------

/// Inserts `name` under the `llm_estimate` source, unless a row for its key already exists
/// there (never overwrites — mirrors Phase 1's `FoodTable::insert`).
pub async fn insert_estimate(
    ctx: &PluginContext,
    name: &str,
    per_100g: Macros,
) -> Result<(), PluginError> {
    let name = name.trim();
    let key = normalize(name);
    if key.is_empty() {
        return Err(PluginError::InvalidArgs(
            "food name must not be empty".into(),
        ));
    }
    let now = ctx.clock.now().to_rfc3339();
    sqlx::query(
        "INSERT INTO diet_foods
            (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
             use_count, created_at, updated_at)
         VALUES (?, 'llm_estimate', ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
         ON CONFLICT(source_id, name_key) DO NOTHING",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(name)
    .bind(&key)
    .bind(per_100g.kcal)
    .bind(per_100g.protein_g)
    .bind(per_100g.carbs_g)
    .bind(per_100g.fat_g)
    .bind(per_100g.fiber_g)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db)
    .await?;
    Ok(())
}

/// Names of winning foods across enabled sources, grouped by `name_key`, ordered by summed
/// `use_count` and then by name — the parser prompt's `KNOWN_FOODS` list (§8.4, extended by §5).
pub async fn known_names(ctx: &PluginContext, limit: u32) -> Result<Vec<String>, PluginError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "WITH enabled_foods AS (
            SELECT f.name, f.name_key, f.use_count, s.priority
            FROM diet_foods f JOIN diet_food_sources s ON f.source_id = s.id
            WHERE s.enabled = 1
         ), ranked AS (
            SELECT name, name_key,
                   ROW_NUMBER() OVER (PARTITION BY name_key ORDER BY priority ASC) AS rn,
                   SUM(use_count) OVER (PARTITION BY name_key) AS total_uses
            FROM enabled_foods
         )
         SELECT name FROM ranked WHERE rn = 1 ORDER BY total_uses DESC, name ASC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&ctx.db)
    .await?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

// ---- Manual writes (§7.2) ---------------------------------------------------------------------

/// `diet_update_food` always upserts into `manual` (§7.2): creates or updates the row
/// `(manual, key)`, where `key` is resolved through `diet_name_aliases` first. Never edits an
/// imported dataset or the estimate cache — it shadows them.
pub async fn upsert_manual_food(
    ctx: &PluginContext,
    name: &str,
    per_100g: Macros,
) -> Result<Food, PluginError> {
    let name = name.trim();
    let raw_key = normalize(name);
    if raw_key.is_empty() {
        return Err(PluginError::InvalidArgs(
            "food name must not be empty".into(),
        ));
    }
    let redirect: Option<(String,)> =
        sqlx::query_as("SELECT target_key FROM diet_name_aliases WHERE alias_key = ?")
            .bind(&raw_key)
            .fetch_optional(&ctx.db)
            .await?;
    let key = redirect.map_or(raw_key, |(t,)| t);

    let existing: Option<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM diet_foods WHERE source_id = 'manual' AND name_key = ?",
    )
    .bind(&key)
    .fetch_optional(&ctx.db)
    .await?;
    // An update keeps the row's existing display name (Phase 1's `upsert_user` behavior); only a
    // brand-new row takes the name the caller gave.
    let (id, display_name) = match existing {
        Some((id, existing_name)) => (id, existing_name),
        None => (Uuid::now_v7().to_string(), name.to_string()),
    };

    let now = ctx.clock.now().to_rfc3339();
    sqlx::query(
        "INSERT INTO diet_foods
            (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
             use_count, created_at, updated_at)
         VALUES (?, 'manual', ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
         ON CONFLICT(source_id, name_key) DO UPDATE SET
            kcal = excluded.kcal, protein_g = excluded.protein_g, carbs_g = excluded.carbs_g,
            fat_g = excluded.fat_g, fiber_g = excluded.fiber_g, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&display_name)
    .bind(&key)
    .bind(per_100g.kcal)
    .bind(per_100g.protein_g)
    .bind(per_100g.carbs_g)
    .bind(per_100g.fat_g)
    .bind(per_100g.fiber_g)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db)
    .await?;

    let manual_info = fetch_source_info(&ctx.db, MANUAL_SOURCE_ID).await?;
    TableSource::new(manual_info, ctx.db.clone())
        .lookup(&key)
        .await?
        .ok_or_else(|| PluginError::Other(format!("manual food {key:?} missing after upsert")))
}

/// The §7.2 hard rules for a per-serving dish row (the warning-only energy-vs-macros check is
/// import-only, stage 3). Used by `diet_update_dish`; `Err` describes the first rule that failed.
pub fn validate_dish_macros(per_serving: &Macros, serving_g: Option<f64>) -> Result<(), String> {
    let values = [
        per_serving.kcal,
        per_serving.protein_g,
        per_serving.carbs_g,
        per_serving.fat_g,
        per_serving.fiber_g,
    ];
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("all values must be numbers ≥ 0".into());
    }
    if per_serving.fiber_g > per_serving.carbs_g {
        return Err(format!(
            "fiber_g ({}) must not exceed carbs_g ({}): carbs_g is total carbohydrate, fiber included",
            per_serving.fiber_g, per_serving.carbs_g
        ));
    }
    if per_serving.kcal > 5000.0 {
        return Err(format!(
            "kcal must be ≤ 5000 per serving, got {}",
            per_serving.kcal
        ));
    }
    if let Some(g) = serving_g {
        if g <= 0.0 {
            return Err("serving_g must be > 0".into());
        }
        let sum = per_serving.protein_g + per_serving.carbs_g + per_serving.fat_g;
        if sum > g {
            return Err(format!(
                "protein_g + carbs_g + fat_g ({sum}) must not exceed serving_g ({g})"
            ));
        }
        if per_serving.kcal > 9.0 * g {
            return Err(format!(
                "kcal ({}) must not exceed 9 × serving_g ({g})",
                per_serving.kcal
            ));
        }
    }
    Ok(())
}

/// `diet_update_dish` (§7.4). Always upserts into `manual`: creates or updates the row
/// `(manual, brand_key, key)`, where `key` is resolved through `diet_dish_name_aliases` first
/// (the pick/user redirect), symmetric with `upsert_manual_food`. Never edits an imported dataset
/// — it shadows it.
pub async fn upsert_manual_dish(
    ctx: &PluginContext,
    dish_name: &str,
    brand: Option<&str>,
    per_serving: Macros,
    serving_g: Option<f64>,
) -> Result<Dish, PluginError> {
    let name = dish_name.trim();
    let raw_key = normalize(name);
    if raw_key.is_empty() {
        return Err(PluginError::InvalidArgs(
            "dish name must not be empty".into(),
        ));
    }
    let brand = brand.unwrap_or("").trim();
    let brand_key = normalize(brand);

    let redirect: Option<(String,)> = sqlx::query_as(
        "SELECT target_name_key FROM diet_dish_name_aliases WHERE brand_key = ? AND alias_key = ?",
    )
    .bind(&brand_key)
    .bind(&raw_key)
    .fetch_optional(&ctx.db)
    .await?;
    let key = redirect.map_or(raw_key, |(t,)| t);

    let existing: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, name, brand FROM diet_dishes WHERE source_id = 'manual' AND brand_key = ? AND name_key = ?",
    )
    .bind(&brand_key)
    .bind(&key)
    .fetch_optional(&ctx.db)
    .await?;
    // An update keeps the row's existing display name/brand; only a brand-new row takes the
    // caller's spelling (mirrors `upsert_manual_food`).
    let (id, display_name, display_brand) = match existing {
        Some((id, existing_name, existing_brand)) => (id, existing_name, existing_brand),
        None => (
            Uuid::now_v7().to_string(),
            name.to_string(),
            brand.to_string(),
        ),
    };

    let now = ctx.clock.now().to_rfc3339();
    sqlx::query(
        "INSERT INTO diet_dishes
            (id, source_id, brand, brand_key, name, name_key, serving_g, kcal, protein_g,
             carbs_g, fat_g, fiber_g, use_count, created_at, updated_at)
         VALUES (?, 'manual', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
         ON CONFLICT(source_id, brand_key, name_key) DO UPDATE SET
            serving_g = excluded.serving_g, kcal = excluded.kcal, protein_g = excluded.protein_g,
            carbs_g = excluded.carbs_g, fat_g = excluded.fat_g, fiber_g = excluded.fiber_g,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&display_brand)
    .bind(&brand_key)
    .bind(&display_name)
    .bind(&key)
    .bind(serving_g)
    .bind(per_serving.kcal)
    .bind(per_serving.protein_g)
    .bind(per_serving.carbs_g)
    .bind(per_serving.fat_g)
    .bind(per_serving.fiber_g)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db)
    .await?;

    let manual_info = fetch_source_info(&ctx.db, MANUAL_SOURCE_ID).await?;
    TableSource::new(manual_info, ctx.db.clone())
        .lookup_dish(&brand_key, &key)
        .await?
        .ok_or_else(|| PluginError::Other(format!("manual dish {key:?} missing after upsert")))
}

// ---- User aliases (§7.3) -----------------------------------------------------------------------

/// `diet_add_alias`. Returns the aliased food and the normalized alias key that was stored.
pub async fn add_user_alias(
    ctx: &PluginContext,
    food_name: &str,
    alias: &str,
) -> Result<(Food, String), PluginError> {
    let alias_key = normalize(alias);
    if alias_key.is_empty() {
        return Err(PluginError::InvalidArgs("alias must not be empty".into()));
    }
    let chain = SourceChain::load(ctx).await?;
    let food = chain
        .lookup(food_name)
        .await?
        .ok_or_else(|| PluginError::NotFound(format!("food {food_name:?}")))?;
    let target_key = food.name_key.clone();

    if alias_key == target_key {
        return Ok((food, alias_key));
    }
    if let Some(existing) = chain.lookup(&alias_key).await? {
        if existing.id != food.id {
            return Err(PluginError::InvalidArgs(format!(
                "{alias:?} already refers to the food {:?}",
                existing.name
            )));
        }
        // Already resolves to the same food (idempotent).
        return Ok((food, alias_key));
    }

    // No `X -> alias`: reject when `target_key` is itself an existing alias key.
    let target_is_alias: Option<(String,)> =
        sqlx::query_as("SELECT alias_key FROM diet_name_aliases WHERE alias_key = ? LIMIT 1")
            .bind(&target_key)
            .fetch_optional(&ctx.db)
            .await?;
    if target_is_alias.is_some() {
        return Err(PluginError::InvalidArgs(
            "cannot add an alias pointing at a name that is itself an alias".into(),
        ));
    }
    // No `Y -> X` followed by `X -> Z`: reject when `alias_key` is already some alias's target.
    let alias_is_a_target: Option<(String,)> =
        sqlx::query_as("SELECT alias_key FROM diet_name_aliases WHERE target_key = ? LIMIT 1")
            .bind(&alias_key)
            .fetch_optional(&ctx.db)
            .await?;
    if alias_is_a_target.is_some() {
        return Err(PluginError::InvalidArgs(
            "this name is already the target of another alias".into(),
        ));
    }

    sqlx::query(
        "INSERT INTO diet_name_aliases (alias_key, target_key, created_at) VALUES (?, ?, ?)",
    )
    .bind(&alias_key)
    .bind(&target_key)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok((food, alias_key))
}

/// A stored name redirect from `diet_name_aliases`, for `foods lookup`
/// (`docs/FOOD_MATCHING.md` §7).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Redirect {
    pub alias: String,
    pub target: String,
    /// `"user"` (`diet_add_alias`, or a migrated Phase 1 alias) or `"match"` (a picked dataset
    /// row, `docs/FOOD_MATCHING.md` §6.2).
    pub origin: String,
}

/// The redirect stored for `name`, if any.
pub async fn lookup_redirect(
    ctx: &PluginContext,
    name: &str,
) -> Result<Option<Redirect>, PluginError> {
    let key = normalize(name);
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT target_key, origin FROM diet_name_aliases WHERE alias_key = ?")
            .bind(&key)
            .fetch_optional(&ctx.db)
            .await?;
    Ok(row.map(|(target, origin)| Redirect {
        alias: key,
        target,
        origin,
    }))
}

/// `diet_remove_alias` (`docs/FOOD_MATCHING.md` §7): deletes the redirect for `alias`, whatever
/// its origin, and records a match check for that key so the next meal doesn't pick the same
/// row again until a dataset changes. Returns the normalized alias key. `NotFound` when there is
/// no such redirect.
pub async fn remove_alias(ctx: &PluginContext, alias: &str) -> Result<String, PluginError> {
    let key = normalize(alias);
    if key.is_empty() {
        return Err(PluginError::InvalidArgs("alias must not be empty".into()));
    }
    let deleted = sqlx::query("DELETE FROM diet_name_aliases WHERE alias_key = ?")
        .bind(&key)
        .execute(&ctx.db)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(PluginError::NotFound(format!("alias {alias:?}")));
    }
    crate::food_matcher::record_check(ctx, &key).await?;
    Ok(key)
}

// ---- Source management (§7.1) ------------------------------------------------------------------

async fn fetch_source_info(db: &SqlitePool, id: &str) -> Result<SourceInfo, PluginError> {
    let row: Option<(String, String, i64, i64, String)> = sqlx::query_as(
        "SELECT kind, name, priority, enabled, origin_json FROM diet_food_sources WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let (kind, name, priority, enabled, origin_json) =
        row.ok_or_else(|| PluginError::NotFound(format!("source {id:?}")))?;
    Ok(SourceInfo {
        id: id.to_string(),
        kind: parse_kind(&kind)?,
        name,
        priority,
        enabled: enabled != 0,
        food_count: 0,
        dish_count: 0,
        origin: parse_origin(&origin_json)?,
    })
}

/// One source, with `food_count`/`dish_count` filled in.
pub async fn get_source(ctx: &PluginContext, id: &str) -> Result<SourceInfo, PluginError> {
    let mut info = fetch_source_info(&ctx.db, id).await?;
    let (food_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM diet_foods WHERE source_id = ?")
            .bind(id)
            .fetch_one(&ctx.db)
            .await?;
    let (dish_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM diet_dishes WHERE source_id = ?")
            .bind(id)
            .fetch_one(&ctx.db)
            .await?;
    info.food_count = food_count;
    info.dish_count = dish_count;
    Ok(info)
}

/// (id, brand, brand_key, name, serving_g, kcal, protein_g, carbs_g, fat_g, fiber_g)
#[allow(clippy::type_complexity)]
type ExportDishRow = (
    String,
    String,
    String,
    String,
    Option<f64>,
    f64,
    f64,
    f64,
    f64,
    f64,
);

/// Same CSV/JSON formats as import, so an export can be re-imported. Includes the source's own
/// aliases, for both foods and dishes (`docs/DISH_NUTRITION.md` §7.3). Any source can be
/// exported, including `manual` and `llm_estimate` (which never has dish rows).
pub async fn export_source(
    ctx: &PluginContext,
    source_id: &str,
    format: crate::import::ImportFormat,
) -> Result<Vec<u8>, PluginError> {
    let info = fetch_source_info(&ctx.db, source_id).await?;
    let foods: Vec<(String, String, f64, f64, f64, f64, f64)> = sqlx::query_as(
        "SELECT id, name, kcal, protein_g, carbs_g, fat_g, fiber_g FROM diet_foods
         WHERE source_id = ? ORDER BY name ASC",
    )
    .bind(source_id)
    .fetch_all(&ctx.db)
    .await?;
    let mut food_rows = Vec::with_capacity(foods.len());
    for (id, name, kcal, protein_g, carbs_g, fat_g, fiber_g) in foods {
        let aliases: Vec<(String,)> = sqlx::query_as(
            "SELECT alias_key FROM diet_food_aliases WHERE source_id = ? AND food_id = ?",
        )
        .bind(source_id)
        .bind(&id)
        .fetch_all(&ctx.db)
        .await?;
        food_rows.push(crate::import::ExportRow {
            basis: "100g",
            brand: String::new(),
            serving_g: None,
            name,
            macros: Macros {
                kcal,
                protein_g,
                carbs_g,
                fat_g,
                fiber_g,
            },
            aliases: aliases.into_iter().map(|(a,)| a).collect(),
        });
    }

    let dishes: Vec<ExportDishRow> = sqlx::query_as(
        "SELECT id, brand, brand_key, name, serving_g, kcal, protein_g, carbs_g, fat_g, fiber_g
             FROM diet_dishes WHERE source_id = ? ORDER BY brand ASC, name ASC",
    )
    .bind(source_id)
    .fetch_all(&ctx.db)
    .await?;
    let mut dish_rows = Vec::with_capacity(dishes.len());
    for (id, brand, brand_key, name, serving_g, kcal, protein_g, carbs_g, fat_g, fiber_g) in dishes
    {
        let aliases: Vec<(String,)> = sqlx::query_as(
            "SELECT alias_key FROM diet_dish_source_aliases
             WHERE source_id = ? AND brand_key = ? AND dish_id = ?",
        )
        .bind(source_id)
        .bind(&brand_key)
        .bind(&id)
        .fetch_all(&ctx.db)
        .await?;
        dish_rows.push(crate::import::ExportRow {
            basis: "serving",
            brand,
            serving_g,
            name,
            macros: Macros {
                kcal,
                protein_g,
                carbs_g,
                fat_g,
                fiber_g,
            },
            aliases: aliases.into_iter().map(|(a,)| a).collect(),
        });
    }

    Ok(match format {
        crate::import::ImportFormat::Csv => {
            let mut all = food_rows;
            all.extend(dish_rows);
            crate::import::write_csv(&all)
        }
        crate::import::ImportFormat::Json => {
            crate::import::write_json(&info.name, &food_rows, &dish_rows)
        }
    })
}

/// (id, brand, brand_key, name, name_key, serving_g, kcal, protein_g, carbs_g, fat_g, fiber_g,
///  use_count, source_id, source_kind, source_name)
#[allow(clippy::type_complexity)]
type DishWithSourceRow = (
    String,
    String,
    String,
    String,
    String,
    Option<f64>,
    f64,
    f64,
    f64,
    f64,
    f64,
    i64,
    String,
    String,
    String,
);

fn row_to_dish_with_source(row: DishWithSourceRow) -> Result<Dish, PluginError> {
    let (
        id,
        brand,
        brand_key,
        name,
        name_key,
        serving_g,
        kcal,
        protein_g,
        carbs_g,
        fat_g,
        fiber_g,
        use_count,
        source_id,
        source_kind,
        source_name,
    ) = row;
    Ok(Dish {
        id,
        brand,
        brand_key,
        name,
        name_key,
        serving_g,
        per_serving: Macros {
            kcal,
            protein_g,
            carbs_g,
            fat_g,
            fiber_g,
        },
        source_id,
        source_kind: parse_kind(&source_kind)?,
        source_name,
        use_count,
    })
}

/// Browsing: `foods list --dishes`. `brand`, when given, filters to that brand exactly
/// (`docs/DISH_NUTRITION.md` §8.1). Unlike `list_foods`, there's no name substring filter — not
/// asked for by the CLI spec, and would multiply the combinations `query_as`'s `&'static str`
/// requirement forces us to spell out (see `list_foods`'s own comment).
pub async fn list_dishes(
    ctx: &PluginContext,
    source_id: Option<&str>,
    brand: Option<&str>,
    limit: u32,
) -> Result<Vec<Dish>, PluginError> {
    let brand_key = brand.map(normalize);
    let rows: Vec<DishWithSourceRow> = match (source_id, &brand_key) {
        (Some(sid), Some(bk)) => {
            sqlx::query_as(
                "SELECT d.id, d.brand, d.brand_key, d.name, d.name_key, d.serving_g, d.kcal, \
                        d.protein_g, d.carbs_g, d.fat_g, d.fiber_g, d.use_count, s.id, s.kind, s.name
                 FROM diet_dishes d JOIN diet_food_sources s ON d.source_id = s.id
                 WHERE d.source_id = ? AND d.brand_key = ?
                 ORDER BY d.use_count DESC, d.name ASC LIMIT ?",
            )
            .bind(sid)
            .bind(bk)
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
        (Some(sid), None) => {
            sqlx::query_as(
                "SELECT d.id, d.brand, d.brand_key, d.name, d.name_key, d.serving_g, d.kcal, \
                        d.protein_g, d.carbs_g, d.fat_g, d.fiber_g, d.use_count, s.id, s.kind, s.name
                 FROM diet_dishes d JOIN diet_food_sources s ON d.source_id = s.id
                 WHERE d.source_id = ?
                 ORDER BY d.use_count DESC, d.name ASC LIMIT ?",
            )
            .bind(sid)
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
        (None, Some(bk)) => {
            sqlx::query_as(
                "SELECT d.id, d.brand, d.brand_key, d.name, d.name_key, d.serving_g, d.kcal, \
                        d.protein_g, d.carbs_g, d.fat_g, d.fiber_g, d.use_count, s.id, s.kind, s.name
                 FROM diet_dishes d JOIN diet_food_sources s ON d.source_id = s.id
                 WHERE d.brand_key = ?
                 ORDER BY d.use_count DESC, d.name ASC LIMIT ?",
            )
            .bind(bk)
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as(
                "SELECT d.id, d.brand, d.brand_key, d.name, d.name_key, d.serving_g, d.kcal, \
                        d.protein_g, d.carbs_g, d.fat_g, d.fiber_g, d.use_count, s.id, s.kind, s.name
                 FROM diet_dishes d JOIN diet_food_sources s ON d.source_id = s.id
                 ORDER BY d.use_count DESC, d.name ASC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
    };
    rows.into_iter().map(row_to_dish_with_source).collect()
}

/// (id, kind, name, priority, enabled, origin_json, food_count, dish_count)
type SourceRow = (String, String, String, i64, i64, String, i64, i64);

/// All sources in priority order, with `food_count`/`dish_count` filled in.
pub async fn list_sources(ctx: &PluginContext) -> Result<Vec<SourceInfo>, PluginError> {
    let rows: Vec<SourceRow> = sqlx::query_as(
        "SELECT s.id, s.kind, s.name, s.priority, s.enabled, s.origin_json,
                (SELECT COUNT(*) FROM diet_foods f WHERE f.source_id = s.id),
                (SELECT COUNT(*) FROM diet_dishes d WHERE d.source_id = s.id)
         FROM diet_food_sources s ORDER BY s.priority ASC",
    )
    .fetch_all(&ctx.db)
    .await?;
    rows.into_iter()
        .map(
            |(id, kind, name, priority, enabled, origin_json, food_count, dish_count)| {
                Ok(SourceInfo {
                    id,
                    kind: parse_kind(&kind)?,
                    name,
                    priority,
                    enabled: enabled != 0,
                    food_count,
                    dish_count,
                    origin: parse_origin(&origin_json)?,
                })
            },
        )
        .collect()
}

async fn ensure_dataset(ctx: &PluginContext, id: &str) -> Result<(), PluginError> {
    let kind: Option<(String,)> = sqlx::query_as("SELECT kind FROM diet_food_sources WHERE id = ?")
        .bind(id)
        .fetch_optional(&ctx.db)
        .await?;
    match kind {
        Some((k,)) if k == SourceKind::Dataset.db_str() => Ok(()),
        Some(_) => Err(PluginError::InvalidArgs(format!(
            "{id:?} is not a dataset source"
        ))),
        None => Err(PluginError::NotFound(format!("source {id:?}"))),
    }
}

/// Must be exactly the current dataset ids. Renumbers them 1..n. `manual` and `llm_estimate`
/// are rejected implicitly, since they're never in the current dataset id set.
pub async fn reorder_datasets(ctx: &PluginContext, ids: &[String]) -> Result<(), PluginError> {
    let current: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM diet_food_sources WHERE kind = 'dataset' ORDER BY priority ASC",
    )
    .fetch_all(&ctx.db)
    .await?;
    let mut given_sorted: Vec<&String> = ids.iter().collect();
    let mut current_sorted: Vec<&String> = current.iter().map(|(id,)| id).collect();
    given_sorted.sort();
    current_sorted.sort();
    if given_sorted != current_sorted {
        return Err(PluginError::InvalidArgs(
            "the new order must contain exactly the current dataset ids".into(),
        ));
    }

    let now = ctx.clock.now().to_rfc3339();
    let mut tx = ctx.db.begin().await?;
    for (i, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE diet_food_sources SET priority = ?, updated_at = ? WHERE id = ?")
            .bind(i as i64 + 1)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Datasets only.
pub async fn set_enabled(ctx: &PluginContext, id: &str, enabled: bool) -> Result<(), PluginError> {
    ensure_dataset(ctx, id).await?;
    sqlx::query("UPDATE diet_food_sources SET enabled = ?, updated_at = ? WHERE id = ?")
        .bind(enabled)
        .bind(ctx.clock.now().to_rfc3339())
        .bind(id)
        .execute(&ctx.db)
        .await?;
    Ok(())
}

/// Datasets only. 1–60 chars.
pub async fn rename_source(ctx: &PluginContext, id: &str, name: &str) -> Result<(), PluginError> {
    ensure_dataset(ctx, id).await?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(PluginError::InvalidArgs(
            "source name must be 1-60 characters".into(),
        ));
    }
    sqlx::query("UPDATE diet_food_sources SET name = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(ctx.clock.now().to_rfc3339())
        .bind(id)
        .execute(&ctx.db)
        .await?;
    Ok(())
}

/// Datasets only. Cascades to its rows (`ON DELETE CASCADE`). Past meals are unaffected —
/// they keep a snapshot of their values.
pub async fn delete_source(ctx: &PluginContext, id: &str) -> Result<(), PluginError> {
    ensure_dataset(ctx, id).await?;
    sqlx::query("DELETE FROM diet_food_sources WHERE id = ?")
        .bind(id)
        .execute(&ctx.db)
        .await?;
    Ok(())
}

/// (id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g, use_count, source_id,
///  source_kind, source_name)
type FoodWithSourceRow = (
    String,
    String,
    String,
    f64,
    f64,
    f64,
    f64,
    f64,
    i64,
    String,
    String,
    String,
);

fn row_to_food_with_source(row: FoodWithSourceRow) -> Result<Food, PluginError> {
    let (
        id,
        name,
        name_key,
        kcal,
        protein_g,
        carbs_g,
        fat_g,
        fiber_g,
        use_count,
        source_id,
        source_kind,
        source_name,
    ) = row;
    Ok(Food {
        id,
        name,
        name_key,
        per_100g: Macros {
            kcal,
            protein_g,
            carbs_g,
            fat_g,
            fiber_g,
        },
        source_id,
        source_kind: parse_kind(&source_kind)?,
        source_name,
        use_count,
    })
}

/// Browsing: `foods list`/`list_foods`. `query` is a `name_key LIKE %q%` substring match.
///
/// sqlx 0.9 requires `query_as`'s SQL to be a `&'static str` literal (its `SqlSafeStr` guard
/// against dynamically built SQL), so the four filter combinations are spelled out rather than
/// assembled with `format!`.
pub async fn list_foods(
    ctx: &PluginContext,
    source_id: Option<&str>,
    query: Option<&str>,
    limit: u32,
) -> Result<Vec<Food>, PluginError> {
    let like = query.map(|q| format!("%{}%", normalize(q)));
    let rows: Vec<FoodWithSourceRow> = match (source_id, &like) {
        (Some(sid), Some(pat)) => {
            sqlx::query_as(
                "SELECT f.id, f.name, f.name_key, f.kcal, f.protein_g, f.carbs_g, f.fat_g, \
                        f.fiber_g, f.use_count, s.id, s.kind, s.name
                 FROM diet_foods f JOIN diet_food_sources s ON f.source_id = s.id
                 WHERE f.source_id = ? AND f.name_key LIKE ?
                 ORDER BY f.use_count DESC, f.name ASC LIMIT ?",
            )
            .bind(sid)
            .bind(pat)
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
        (Some(sid), None) => {
            sqlx::query_as(
                "SELECT f.id, f.name, f.name_key, f.kcal, f.protein_g, f.carbs_g, f.fat_g, \
                        f.fiber_g, f.use_count, s.id, s.kind, s.name
                 FROM diet_foods f JOIN diet_food_sources s ON f.source_id = s.id
                 WHERE f.source_id = ?
                 ORDER BY f.use_count DESC, f.name ASC LIMIT ?",
            )
            .bind(sid)
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
        (None, Some(pat)) => {
            sqlx::query_as(
                "SELECT f.id, f.name, f.name_key, f.kcal, f.protein_g, f.carbs_g, f.fat_g, \
                        f.fiber_g, f.use_count, s.id, s.kind, s.name
                 FROM diet_foods f JOIN diet_food_sources s ON f.source_id = s.id
                 WHERE f.name_key LIKE ?
                 ORDER BY f.use_count DESC, f.name ASC LIMIT ?",
            )
            .bind(pat)
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as(
                "SELECT f.id, f.name, f.name_key, f.kcal, f.protein_g, f.carbs_g, f.fat_g, \
                        f.fiber_g, f.use_count, s.id, s.kind, s.name
                 FROM diet_foods f JOIN diet_food_sources s ON f.source_id = s.id
                 ORDER BY f.use_count DESC, f.name ASC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(&ctx.db)
            .await?
        }
    };
    rows.into_iter().map(row_to_food_with_source).collect()
}

/// Every copy of `name` across all sources — `foods lookup`.
pub async fn lookup_all(ctx: &PluginContext, name: &str) -> Result<Vec<Food>, PluginError> {
    SourceChain::lookup_all(ctx, name).await
}

/// Every copy of `(brand, name)` across all sources — `foods lookup`'s dish half.
pub async fn lookup_dish_all(
    ctx: &PluginContext,
    brand: &str,
    name: &str,
) -> Result<Vec<Dish>, PluginError> {
    SourceChain::lookup_dish_all(ctx, brand, name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, EGG, RICE};
    use lifeagent_core::clock::Clock;

    #[tokio::test]
    async fn chain_priority_manual_beats_dataset_beats_estimate() {
        let env = env().await;
        insert_estimate(&env.ctx, "熟米饭", RICE).await.unwrap();
        let dataset_id = new_dataset(&env, "数据集").await;
        insert_dataset_food(&env, &dataset_id, "熟米饭", EGG).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let winner = chain.lookup("熟米饭").await.unwrap().unwrap();
        assert_eq!(winner.source_id, dataset_id);
        assert_eq!(winner.per_100g, EGG);

        upsert_manual_food(&env.ctx, "熟米饭", RICE).await.unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let winner = chain.lookup("熟米饭").await.unwrap().unwrap();
        assert_eq!(winner.source_id, MANUAL_SOURCE_ID);
    }

    #[tokio::test]
    async fn disabling_or_deleting_the_higher_copy_falls_through() {
        let env = env().await;
        insert_estimate(&env.ctx, "熟米饭", RICE).await.unwrap();
        let dataset_id = new_dataset(&env, "数据集").await;
        insert_dataset_food(&env, &dataset_id, "熟米饭", EGG).await;

        set_enabled(&env.ctx, &dataset_id, false).await.unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let winner = chain.lookup("熟米饭").await.unwrap().unwrap();
        assert_eq!(winner.source_id, LLM_ESTIMATE_SOURCE_ID);

        set_enabled(&env.ctx, &dataset_id, true).await.unwrap();
        delete_source(&env.ctx, &dataset_id).await.unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let winner = chain.lookup("熟米饭").await.unwrap().unwrap();
        assert_eq!(winner.source_id, LLM_ESTIMATE_SOURCE_ID);
    }

    #[tokio::test]
    async fn reordering_two_datasets_flips_the_winner() {
        let env = env().await;
        let a = new_dataset(&env, "A").await;
        let b = new_dataset(&env, "B").await;
        insert_dataset_food(&env, &a, "熟米饭", RICE).await;
        insert_dataset_food(&env, &b, "熟米饭", EGG).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(chain.lookup("熟米饭").await.unwrap().unwrap().source_id, a);

        reorder_datasets(&env.ctx, &[b.clone(), a.clone()])
            .await
            .unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(chain.lookup("熟米饭").await.unwrap().unwrap().source_id, b);
    }

    #[tokio::test]
    async fn alias_hit_in_a_higher_source_beats_a_name_hit_in_a_lower_one() {
        let env = env().await;
        let a = new_dataset(&env, "A").await;
        let b = new_dataset(&env, "B").await;
        insert_dataset_food(&env, &b, "熟米饭", RICE).await;
        // `a` (higher priority) doesn't have "米饭" as a name, but has it as an alias of a
        // different food.
        let other = insert_dataset_food(&env, &a, "白米饭", EGG).await;
        add_source_alias(&env, &a, "米饭", &other).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let winner = chain.lookup("米饭").await.unwrap().unwrap();
        assert_eq!(winner.source_id, a);
        assert_eq!(winner.id, other);
    }

    #[tokio::test]
    async fn dangling_user_alias_falls_back_and_recovers() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "数据集").await;
        insert_dataset_food(&env, &dataset_id, "卤鸡蛋", EGG).await;
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap();

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain.lookup("卤蛋").await.unwrap().unwrap().source_id,
            dataset_id
        );

        delete_source(&env.ctx, &dataset_id).await.unwrap();
        // The redirect's target is gone from every enabled source: a dangling alias. A later
        // resolution of the raw key (e.g. an estimate made because nothing else knows it now)
        // still works, and isn't blocked by the (now-dangling) alias.
        insert_estimate(&env.ctx, "卤蛋", RICE).await.unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain.lookup("卤蛋").await.unwrap().unwrap().source_id,
            LLM_ESTIMATE_SOURCE_ID
        );

        // Re-import the target: the redirect works again.
        let dataset_id2 = new_dataset(&env, "数据集2").await;
        insert_dataset_food(&env, &dataset_id2, "卤鸡蛋", EGG).await;
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain.lookup("卤蛋").await.unwrap().unwrap().source_id,
            dataset_id2
        );
    }

    #[tokio::test]
    async fn add_user_alias_rejects_chains_and_cycles() {
        let env = env().await;
        insert_estimate(&env.ctx, "卤鸡蛋", EGG).await.unwrap();
        insert_estimate(&env.ctx, "鸡蛋", RICE).await.unwrap();
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap(); // "卤蛋" -> "卤鸡蛋"
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋2").await.unwrap(); // "卤蛋2" -> "卤鸡蛋"

        // No Y -> X followed by X -> Z: "卤蛋" is already the target of nothing, but it *is*
        // itself a registered alias key, so using it as a brand-new alias's target is rejected
        // (chain.lookup("卤蛋") resolves to a different food than "鸡蛋").
        assert!(add_user_alias(&env.ctx, "鸡蛋", "卤蛋").await.is_err());

        // No X -> alias: build a food reachable only through a source-scoped alias, whose own
        // name_key ("卤蛋2") happens to already be a registered alias key. Aliasing to it would
        // form a 2-hop chain (new -> 卤蛋2 -> 卤鸡蛋).
        let dataset_id = new_dataset(&env, "数据集").await;
        let shadowed = insert_dataset_food(&env, &dataset_id, "卤蛋2", RICE).await;
        add_source_alias(&env, &dataset_id, "咸蛋", &shadowed).await;
        assert!(add_user_alias(&env.ctx, "咸蛋", "新别名").await.is_err());
    }

    #[tokio::test]
    async fn add_user_alias_is_idempotent() {
        let env = env().await;
        insert_estimate(&env.ctx, "卤鸡蛋", EGG).await.unwrap();
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap();
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap();
    }

    #[tokio::test]
    async fn known_names_sum_use_counts_across_shadowed_copies() {
        let env = env().await;
        insert_estimate(&env.ctx, "熟米饭", RICE).await.unwrap();
        let dataset_id = new_dataset(&env, "数据集").await;
        insert_dataset_food(&env, &dataset_id, "熟米饭", EGG).await;
        // Bump the shadowed estimate's use_count directly.
        sqlx::query("UPDATE diet_foods SET use_count = 5 WHERE source_id = 'llm_estimate'")
            .execute(&env.ctx.db)
            .await
            .unwrap();

        let names = known_names(&env.ctx, 10).await.unwrap();
        assert_eq!(names, vec!["熟米饭"]);
    }

    #[tokio::test]
    async fn upsert_manual_creates_new_and_updates_existing() {
        let env = env().await;
        let created = upsert_manual_food(&env.ctx, "熟米饭", RICE).await.unwrap();
        assert_eq!(created.source_id, MANUAL_SOURCE_ID);
        assert_eq!(created.name, "熟米饭");

        let updated = upsert_manual_food(&env.ctx, "熟米饭", EGG).await.unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.per_100g, EGG);
    }

    #[tokio::test]
    async fn upsert_manual_through_a_user_alias_writes_under_the_target_key() {
        let env = env().await;
        insert_estimate(&env.ctx, "卤鸡蛋", EGG).await.unwrap();
        add_user_alias(&env.ctx, "卤鸡蛋", "卤蛋").await.unwrap();

        let updated = upsert_manual_food(&env.ctx, "卤蛋", RICE).await.unwrap();
        assert_eq!(updated.name_key, "卤鸡蛋");
    }

    async fn new_dataset(env: &lifeagent_core::test_support::TestEnv, name: &str) -> String {
        let id = Uuid::now_v7().to_string();
        let now = env.clock.now().to_rfc3339();
        let priority: (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(priority), 0) + 1 FROM diet_food_sources WHERE kind = 'dataset'",
        )
        .fetch_one(&env.ctx.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO diet_food_sources (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
             VALUES (?, 'dataset', ?, ?, 1, '{}', ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(priority.0)
        .bind(&now)
        .bind(&now)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        id
    }

    /// Inserts a food row directly into `source_id` and returns its id.
    async fn insert_dataset_food(
        env: &lifeagent_core::test_support::TestEnv,
        source_id: &str,
        name: &str,
        per_100g: Macros,
    ) -> String {
        let id = Uuid::now_v7().to_string();
        let now = env.clock.now().to_rfc3339();
        sqlx::query(
            "INSERT INTO diet_foods
                (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
                 use_count, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(source_id)
        .bind(name)
        .bind(normalize(name))
        .bind(per_100g.kcal)
        .bind(per_100g.protein_g)
        .bind(per_100g.carbs_g)
        .bind(per_100g.fat_g)
        .bind(per_100g.fiber_g)
        .bind(&now)
        .bind(&now)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        id
    }

    async fn add_source_alias(
        env: &lifeagent_core::test_support::TestEnv,
        source_id: &str,
        alias: &str,
        food_id: &str,
    ) {
        sqlx::query(
            "INSERT INTO diet_food_aliases (source_id, alias_key, food_id) VALUES (?, ?, ?)",
        )
        .bind(source_id)
        .bind(normalize(alias))
        .bind(food_id)
        .execute(&env.ctx.db)
        .await
        .unwrap();
    }

    // ---- Dish chain tests (`docs/DISH_NUTRITION.md` §4.2/§4.3) --------------------------------

    #[tokio::test]
    async fn lookup_dish_exact_match_by_brand_and_name() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "松屋菜单").await;
        insert_dataset_dish(
            &env,
            &dataset_id,
            "松屋",
            "牛めし（並盛）",
            Some(350.0),
            RICE,
        )
        .await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain
            .lookup_dish("松屋", "牛めし（並盛）")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dish.source_id, dataset_id);
        assert_eq!(dish.per_serving, RICE);
        assert_eq!(dish.serving_g, Some(350.0));
    }

    #[tokio::test]
    async fn lookup_dish_requires_the_matching_brand() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "松屋菜单").await;
        insert_dataset_dish(&env, &dataset_id, "松屋", "牛丼", None, RICE).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        // Brandless never matches a branded dish (§2 rule 3).
        assert!(chain.lookup_dish("", "牛丼").await.unwrap().is_none());
        // A different brand doesn't match either.
        assert!(chain.lookup_dish("吉野家", "牛丼").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn lookup_dish_priority_and_reorder() {
        let env = env().await;
        let a = new_dataset(&env, "A").await;
        let b = new_dataset(&env, "B").await;
        insert_dataset_dish(&env, &a, "松屋", "牛めし", None, RICE).await;
        insert_dataset_dish(&env, &b, "松屋", "牛めし", None, EGG).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain
                .lookup_dish("松屋", "牛めし")
                .await
                .unwrap()
                .unwrap()
                .source_id,
            a
        );

        reorder_datasets(&env.ctx, &[b.clone(), a.clone()])
            .await
            .unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain
                .lookup_dish("松屋", "牛めし")
                .await
                .unwrap()
                .unwrap()
                .source_id,
            b
        );
    }

    #[tokio::test]
    async fn lookup_dish_via_source_alias_scoped_to_its_brand() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "松屋菜单").await;
        let dish_id =
            insert_dataset_dish(&env, &dataset_id, "松屋", "牛めし（並盛）", None, RICE).await;
        add_dish_source_alias(&env, &dataset_id, "松屋", "牛丼並盛", &dish_id).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain
            .lookup_dish("松屋", "牛丼並盛")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dish.id, dish_id);
        // The alias is scoped to its brand: it doesn't leak to a different one.
        assert!(chain
            .lookup_dish("吉野家", "牛丼並盛")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn lookup_dish_name_alias_falls_back_while_dangling_and_recovers() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "松屋菜单").await;
        insert_dataset_dish(&env, &dataset_id, "松屋", "牛めし（並盛）", None, RICE).await;
        add_dish_name_alias(&env, "松屋", "牛丼並盛", "牛めし（並盛）").await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain
                .lookup_dish("松屋", "牛丼並盛")
                .await
                .unwrap()
                .unwrap()
                .source_id,
            dataset_id
        );

        delete_source(&env.ctx, &dataset_id).await.unwrap();
        // Dangling: the target is gone from every enabled source, and dishes have no estimate
        // fallback (D3), so the redirect and the raw key both miss now.
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert!(chain
            .lookup_dish("松屋", "牛丼並盛")
            .await
            .unwrap()
            .is_none());

        // Re-import the target: the redirect works again.
        let dataset_id2 = new_dataset(&env, "松屋菜单2").await;
        insert_dataset_dish(&env, &dataset_id2, "松屋", "牛めし（並盛）", None, RICE).await;
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain
                .lookup_dish("松屋", "牛丼並盛")
                .await
                .unwrap()
                .unwrap()
                .source_id,
            dataset_id2
        );
    }

    #[tokio::test]
    async fn record_dish_use_bumps_the_winning_copy_only() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "松屋菜单").await;
        insert_dataset_dish(&env, &dataset_id, "松屋", "牛めし", None, RICE).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain.lookup_dish("松屋", "牛めし").await.unwrap().unwrap();
        assert_eq!(dish.use_count, 0);
        chain.record_dish_use(&dish).await.unwrap();

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain.lookup_dish("松屋", "牛めし").await.unwrap().unwrap();
        assert_eq!(dish.use_count, 1);
    }

    #[tokio::test]
    async fn source_info_reports_dish_count_alongside_food_count() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "混合数据集").await;
        insert_dataset_food(&env, &dataset_id, "米饭", RICE).await;
        insert_dataset_dish(&env, &dataset_id, "松屋", "牛めし", None, EGG).await;
        insert_dataset_dish(&env, &dataset_id, "松屋", "味噌汁", None, EGG).await;

        let info = get_source(&env.ctx, &dataset_id).await.unwrap();
        assert_eq!(info.food_count, 1);
        assert_eq!(info.dish_count, 2);

        let all = list_sources(&env.ctx).await.unwrap();
        let listed = all.iter().find(|s| s.id == dataset_id).unwrap();
        assert_eq!(listed.food_count, 1);
        assert_eq!(listed.dish_count, 2);
    }

    /// Inserts a dish row directly into `source_id` and returns its id.
    async fn insert_dataset_dish(
        env: &lifeagent_core::test_support::TestEnv,
        source_id: &str,
        brand: &str,
        name: &str,
        serving_g: Option<f64>,
        per_serving: Macros,
    ) -> String {
        let id = Uuid::now_v7().to_string();
        let now = env.clock.now().to_rfc3339();
        sqlx::query(
            "INSERT INTO diet_dishes
                (id, source_id, brand, brand_key, name, name_key, serving_g, kcal, protein_g,
                 carbs_g, fat_g, fiber_g, use_count, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(source_id)
        .bind(brand)
        .bind(normalize(brand))
        .bind(name)
        .bind(normalize(name))
        .bind(serving_g)
        .bind(per_serving.kcal)
        .bind(per_serving.protein_g)
        .bind(per_serving.carbs_g)
        .bind(per_serving.fat_g)
        .bind(per_serving.fiber_g)
        .bind(&now)
        .bind(&now)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        id
    }

    async fn add_dish_source_alias(
        env: &lifeagent_core::test_support::TestEnv,
        source_id: &str,
        brand: &str,
        alias: &str,
        dish_id: &str,
    ) {
        sqlx::query(
            "INSERT INTO diet_dish_source_aliases (source_id, brand_key, alias_key, dish_id)
             VALUES (?, ?, ?, ?)",
        )
        .bind(source_id)
        .bind(normalize(brand))
        .bind(normalize(alias))
        .bind(dish_id)
        .execute(&env.ctx.db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn known_brands_orders_by_summed_use_count_then_name() {
        let env = env().await;
        let a = new_dataset(&env, "A").await;
        let b = new_dataset(&env, "B").await;
        insert_dataset_dish(&env, &a, "松屋", "牛めし", None, RICE).await;
        insert_dataset_dish(&env, &a, "吉野家", "牛丼", None, RICE).await;
        // Brandless dishes never appear in known_brands.
        insert_dataset_dish(&env, &a, "", "家常菜", None, RICE).await;
        sqlx::query("UPDATE diet_dishes SET use_count = 5 WHERE brand_key = '吉野家'")
            .execute(&env.ctx.db)
            .await
            .unwrap();

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(
            chain.known_brands(10).await.unwrap(),
            vec!["吉野家".to_string(), "松屋".to_string()]
        );
        assert_eq!(chain.known_brands(1).await.unwrap(), vec!["吉野家"]);

        // A disabled source's brands are absent.
        set_enabled(&env.ctx, &a, false).await.unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(chain.known_brands(10).await.unwrap(), Vec::<String>::new());
        let _ = b; // kept enabled but has no dishes
    }

    #[tokio::test]
    async fn known_dishes_lists_brandless_dishes_deduped_by_priority() {
        let env = env().await;
        let a = new_dataset(&env, "A").await;
        let b = new_dataset(&env, "B").await;
        // Same brandless name_key in both sources: `a` (higher priority) wins the spelling.
        insert_dataset_dish(&env, &a, "", "A定食（大）", None, RICE).await;
        insert_dataset_dish(&env, &b, "", "a定食（大）", None, RICE).await;
        // A branded dish is excluded even though it's in an enabled source.
        insert_dataset_dish(&env, &a, "松屋", "牛めし", None, RICE).await;
        sqlx::query("UPDATE diet_dishes SET use_count = 3 WHERE source_id = ?")
            .bind(&b)
            .execute(&env.ctx.db)
            .await
            .unwrap();

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(chain.known_dishes(10).await.unwrap(), vec!["A定食（大）"]);

        set_enabled(&env.ctx, &a, false).await.unwrap();
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert_eq!(chain.known_dishes(10).await.unwrap(), vec!["a定食（大）"]);
    }

    #[tokio::test]
    async fn brand_candidates_returns_the_winning_copy_per_name_key_in_priority_order() {
        let env = env().await;
        let a = new_dataset(&env, "A").await;
        let b = new_dataset(&env, "B").await;
        insert_dataset_dish(&env, &a, "松屋", "牛めし（並盛）", None, RICE).await;
        insert_dataset_dish(&env, &a, "松屋", "味噌汁", None, EGG).await;
        // Shadowed by `a`'s copy.
        insert_dataset_dish(&env, &b, "松屋", "牛めし（並盛）", None, EGG).await;
        // A different brand never appears.
        insert_dataset_dish(&env, &b, "吉野家", "牛丼", None, EGG).await;

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let candidates = chain.brand_candidates(&normalize("松屋")).await.unwrap();

        assert_eq!(candidates.len(), 2);
        let rice_row = candidates
            .iter()
            .find(|d| d.name == "牛めし（並盛）")
            .unwrap();
        assert_eq!(rice_row.source_id, a);
        assert_eq!(rice_row.per_serving, RICE);
    }

    #[tokio::test]
    async fn brand_candidates_empty_for_a_brand_with_no_local_rows() {
        let env = env().await;
        let chain = SourceChain::load(&env.ctx).await.unwrap();
        assert!(chain
            .brand_candidates(&normalize("不存在的品牌"))
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn validate_dish_macros_hard_rules() {
        assert!(validate_dish_macros(&RICE, Some(350.0)).is_ok());
        assert!(validate_dish_macros(
            &Macros {
                fiber_g: 999.0,
                ..RICE
            },
            None
        )
        .is_err());
        assert!(validate_dish_macros(
            &Macros {
                kcal: 6000.0,
                ..RICE
            },
            None
        )
        .is_err());
        assert!(validate_dish_macros(&RICE, Some(0.0)).is_err());
        // protein+carbs+fat exceeds serving_g.
        assert!(validate_dish_macros(
            &Macros {
                protein_g: 200.0,
                carbs_g: 200.0,
                fat_g: 200.0,
                ..RICE
            },
            Some(100.0)
        )
        .is_err());
        // kcal exceeds 9 x serving_g.
        assert!(validate_dish_macros(
            &Macros {
                kcal: 4000.0,
                ..RICE
            },
            Some(100.0)
        )
        .is_err());
    }

    #[tokio::test]
    async fn upsert_manual_dish_creates_and_updates_keeping_display_name() {
        let env = env().await;
        let created = upsert_manual_dish(&env.ctx, "牛めし並盛", Some("松屋"), RICE, Some(350.0))
            .await
            .unwrap();
        assert_eq!(created.source_id, MANUAL_SOURCE_ID);
        assert_eq!(created.name, "牛めし並盛");
        assert_eq!(created.brand, "松屋");

        let updated = upsert_manual_dish(&env.ctx, "牛めし並盛", Some("松屋"), EGG, Some(300.0))
            .await
            .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "牛めし並盛");
        assert_eq!(updated.per_serving, EGG);
        assert_eq!(updated.serving_g, Some(300.0));
    }

    #[tokio::test]
    async fn upsert_manual_dish_shadows_a_dataset_copy_and_through_a_pick_alias() {
        let env = env().await;
        let dataset_id = new_dataset(&env, "松屋菜单").await;
        insert_dataset_dish(&env, &dataset_id, "松屋", "牛めし（並盛）", None, RICE).await;
        add_dish_name_alias(&env, "松屋", "牛肉饭", "牛めし（並盛）").await;

        let manual = upsert_manual_dish(&env.ctx, "牛肉饭", Some("松屋"), EGG, None)
            .await
            .unwrap();
        assert_eq!(manual.name_key, normalize("牛めし（並盛）"));

        let chain = SourceChain::load(&env.ctx).await.unwrap();
        let winner = chain
            .lookup_dish("松屋", "牛めし（並盛）")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(winner.source_id, MANUAL_SOURCE_ID);
        assert_eq!(winner.per_serving, EGG);
    }

    async fn add_dish_name_alias(
        env: &lifeagent_core::test_support::TestEnv,
        brand: &str,
        alias: &str,
        target_name: &str,
    ) {
        sqlx::query(
            "INSERT INTO diet_dish_name_aliases
                (brand_key, alias_key, target_name_key, origin, created_at)
             VALUES (?, ?, ?, 'user', ?)",
        )
        .bind(normalize(brand))
        .bind(normalize(alias))
        .bind(normalize(target_name))
        .bind(env.clock.now().to_rfc3339())
        .execute(&env.ctx.db)
        .await
        .unwrap();
    }
}
