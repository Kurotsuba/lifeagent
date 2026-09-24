//! The `diet_foods` table, name normalization and the per-100 g nutrition sanity check, per
//! `docs/PHASE1_SPEC.md` §5, §8.2 and §8.3.

use std::sync::Arc;

use lifeagent_core::clock::Clock;
use lifeagent_core::plugin::{PluginContext, PluginError};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::model::{Macros, NutritionSource};

/// Diet plugin migration 1 (§5).
pub const MIGRATION_1: &str = "CREATE TABLE diet_foods (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  name_key      TEXT NOT NULL UNIQUE,
  aliases_json  TEXT NOT NULL DEFAULT '[]',
  kcal          REAL NOT NULL,
  protein_g     REAL NOT NULL,
  carbs_g       REAL NOT NULL,
  fat_g         REAL NOT NULL,
  fiber_g       REAL NOT NULL DEFAULT 0,
  source        TEXT NOT NULL,
  use_count     INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
)";

#[derive(Debug, Clone, PartialEq)]
pub struct Food {
    pub id: String,
    /// Canonical display name, in the user's language.
    pub name: String,
    pub name_key: String,
    /// Normalized alias keys.
    pub aliases: Vec<String>,
    pub per_100g: Macros,
    pub source: NutritionSource,
    pub use_count: i64,
}

/// Punctuation stripped from the end of a name. Closing brackets are deliberately not in the
/// list: names like "牛肉(熟)" end in one.
const TRAILING_PUNCTUATION: &[char] = &[
    '.', ',', '!', '?', ';', ':', '。', '，', '！', '？', '；', '：', '、', '…', '~', '～',
];

/// Lookup key for a food name (§8.2): trimmed, ASCII-lowercased, whitespace collapsed,
/// trailing punctuation removed.
pub fn normalize(name: &str) -> String {
    let collapsed = name
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    collapsed
        .trim_end_matches(TRAILING_PUNCTUATION)
        .trim_end()
        .to_string()
}

/// The hard validity rules of §8.3, without the kcal correction. `Err` describes the first
/// rule that failed.
pub fn validate_per_100g(n: &Macros) -> Result<(), String> {
    let values = [n.kcal, n.protein_g, n.carbs_g, n.fat_g, n.fiber_g];
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("all values must be numbers ≥ 0".into());
    }
    if n.fiber_g > n.carbs_g {
        return Err(format!(
            "fiber_g ({}) must not exceed carbs_g ({}): carbs_g is total carbohydrate, fiber included",
            n.fiber_g, n.carbs_g
        ));
    }
    let sum = n.protein_g + n.carbs_g + n.fat_g;
    if sum > 100.0 {
        return Err(format!(
            "protein_g + carbs_g + fat_g must be ≤ 100 per 100 g, got {sum}"
        ));
    }
    if n.kcal > 900.0 {
        return Err(format!("kcal must be ≤ 900 per 100 g, got {}", n.kcal));
    }
    Ok(())
}

/// The full §8.3 check for estimated values: the validity rules, then `kcal` replaced by the
/// macro-derived energy when the two disagree by more than `max(20, 15%)`.
pub fn check_nutrition(n: &Macros) -> Result<Macros, String> {
    validate_per_100g(n)?;
    let e = 4.0 * n.protein_g + 4.0 * (n.carbs_g - n.fiber_g) + 9.0 * n.fat_g + 2.0 * n.fiber_g;
    let mut checked = *n;
    if (n.kcal - e).abs() > f64::max(20.0, 0.15 * e) {
        tracing::debug!(
            given = n.kcal,
            computed = e,
            "replacing kcal with macro-derived energy"
        );
        checked.kcal = e;
    }
    Ok(checked)
}

/// (id, name, name_key, aliases_json, kcal, protein_g, carbs_g, fat_g, fiber_g, source, use_count)
type FoodRow = (
    String,
    String,
    String,
    String,
    f64,
    f64,
    f64,
    f64,
    f64,
    String,
    i64,
);

#[derive(Clone)]
pub struct FoodTable {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
}

impl FoodTable {
    pub fn new(ctx: &PluginContext) -> Self {
        Self {
            db: ctx.db.clone(),
            clock: ctx.clock.clone(),
        }
    }

    /// Finds a food by normalized name or alias. A name match wins over an alias match.
    pub async fn find(&self, name: &str) -> Result<Option<Food>, PluginError> {
        let key = normalize(name);
        if key.is_empty() {
            return Ok(None);
        }
        let row: Option<FoodRow> = sqlx::query_as(
            "SELECT id, name, name_key, aliases_json, kcal, protein_g, carbs_g, fat_g, fiber_g,
                    source, use_count
             FROM diet_foods
             WHERE name_key = ?1
                OR EXISTS (SELECT 1 FROM json_each(diet_foods.aliases_json) WHERE value = ?1)
             ORDER BY name_key = ?1 DESC
             LIMIT 1",
        )
        .bind(&key)
        .fetch_optional(&self.db)
        .await?;
        row.map(row_to_food).transpose()
    }

    pub async fn record_use(&self, id: &str) -> Result<(), PluginError> {
        sqlx::query("UPDATE diet_foods SET use_count = use_count + 1 WHERE id = ?")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Inserts a food unless one with the same key already exists — an existing food (which
    /// may carry user-corrected values) is never overwritten. Returns whichever is stored.
    pub async fn insert(
        &self,
        name: &str,
        per_100g: Macros,
        source: NutritionSource,
    ) -> Result<Food, PluginError> {
        let name = name.trim();
        let key = normalize(name);
        if key.is_empty() {
            return Err(PluginError::InvalidArgs(
                "food name must not be empty".into(),
            ));
        }
        let now = self.clock.now().to_rfc3339();
        sqlx::query(
            "INSERT INTO diet_foods
                (id, name, name_key, aliases_json, kcal, protein_g, carbs_g, fat_g, fiber_g,
                 source, use_count, created_at, updated_at)
             VALUES (?, ?, ?, '[]', ?, ?, ?, ?, ?, ?, 0, ?, ?)
             ON CONFLICT(name_key) DO NOTHING",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(name)
        .bind(&key)
        .bind(per_100g.kcal)
        .bind(per_100g.protein_g)
        .bind(per_100g.carbs_g)
        .bind(per_100g.fat_g)
        .bind(per_100g.fiber_g)
        .bind(source.as_str())
        .bind(&now)
        .bind(&now)
        .execute(&self.db)
        .await?;

        self.find(name)
            .await?
            .ok_or_else(|| PluginError::Other(format!("food {name:?} missing after insert")))
    }

    /// Creates or overwrites a food's nutrition with user-supplied values (`diet_update_food`).
    /// A name that matches an alias updates the aliased food.
    pub async fn upsert_user(&self, name: &str, per_100g: Macros) -> Result<Food, PluginError> {
        let Some(existing) = self.find(name).await? else {
            return self.insert(name, per_100g, NutritionSource::User).await;
        };
        sqlx::query(
            "UPDATE diet_foods
             SET kcal = ?, protein_g = ?, carbs_g = ?, fat_g = ?, fiber_g = ?, source = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(per_100g.kcal)
        .bind(per_100g.protein_g)
        .bind(per_100g.carbs_g)
        .bind(per_100g.fat_g)
        .bind(per_100g.fiber_g)
        .bind(NutritionSource::User.as_str())
        .bind(self.clock.now().to_rfc3339())
        .bind(&existing.id)
        .execute(&self.db)
        .await?;
        Ok(Food {
            per_100g,
            source: NutritionSource::User,
            ..existing
        })
    }

    /// Adds `alias` to the food called `food_name`. Idempotent; fails if the alias already
    /// names a different food.
    pub async fn add_alias(&self, food_name: &str, alias: &str) -> Result<Food, PluginError> {
        let mut food = self
            .find(food_name)
            .await?
            .ok_or_else(|| PluginError::NotFound(format!("food {food_name:?}")))?;
        let key = normalize(alias);
        if key.is_empty() {
            return Err(PluginError::InvalidArgs("alias must not be empty".into()));
        }
        if key == food.name_key || food.aliases.contains(&key) {
            return Ok(food);
        }
        if let Some(other) = self.find(alias).await? {
            return Err(PluginError::InvalidArgs(format!(
                "{alias:?} already refers to the food {:?}",
                other.name
            )));
        }

        food.aliases.push(key);
        let aliases_json =
            serde_json::to_string(&food.aliases).map_err(|e| PluginError::Other(e.to_string()))?;
        sqlx::query("UPDATE diet_foods SET aliases_json = ?, updated_at = ? WHERE id = ?")
            .bind(aliases_json)
            .bind(self.clock.now().to_rfc3339())
            .bind(&food.id)
            .execute(&self.db)
            .await?;
        Ok(food)
    }

    /// Food names, most used first — the parser prompt's KNOWN_FOODS list (§8.4).
    pub async fn known_names(&self, limit: u32) -> Result<Vec<String>, PluginError> {
        let names: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM diet_foods ORDER BY use_count DESC, name ASC LIMIT ?")
                .bind(limit)
                .fetch_all(&self.db)
                .await?;
        Ok(names.into_iter().map(|(n,)| n).collect())
    }
}

fn row_to_food(row: FoodRow) -> Result<Food, PluginError> {
    let (
        id,
        name,
        name_key,
        aliases_json,
        kcal,
        protein_g,
        carbs_g,
        fat_g,
        fiber_g,
        source,
        use_count,
    ) = row;
    Ok(Food {
        id,
        name,
        name_key,
        aliases: serde_json::from_str(&aliases_json)
            .map_err(|e| PluginError::Other(format!("bad aliases_json in db: {e}")))?,
        per_100g: Macros {
            kcal,
            protein_g,
            carbs_g,
            fat_g,
            fiber_g,
        },
        source: NutritionSource::parse(&source)
            .ok_or_else(|| PluginError::Other(format!("bad food source in db: {source:?}")))?,
        use_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macros(kcal: f64, protein_g: f64, carbs_g: f64, fat_g: f64, fiber_g: f64) -> Macros {
        Macros {
            kcal,
            protein_g,
            carbs_g,
            fat_g,
            fiber_g,
        }
    }

    fn rice() -> Macros {
        macros(116.0, 2.6, 25.9, 0.3, 0.3)
    }

    #[test]
    fn normalize_trims_lowercases_collapses_and_strips_trailing_punctuation() {
        assert_eq!(normalize("  Greek   Yogurt. "), "greek yogurt");
        assert_eq!(normalize("熟米饭。"), "熟米饭");
        assert_eq!(normalize("卤蛋，，"), "卤蛋");
        // Closing brackets are part of the name, not punctuation to strip.
        assert_eq!(normalize("牛肉(熟)"), "牛肉(熟)");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn check_accepts_consistent_values_unchanged() {
        assert_eq!(check_nutrition(&rice()), Ok(rice()));
    }

    #[test]
    fn check_accepts_high_fiber_food_with_fiber_inside_carbs() {
        // Wheat bran: rejected under the old fiber-as-separate-addend rule (B2).
        let bran = macros(216.0, 15.5, 64.5, 4.3, 42.8);
        assert!(check_nutrition(&bran).is_ok());
    }

    #[test]
    fn check_rejects_fiber_above_carbs() {
        assert!(check_nutrition(&macros(50.0, 1.0, 5.0, 0.0, 10.0)).is_err());
    }

    #[test]
    fn check_rejects_negative_values_sum_over_100_and_kcal_over_900() {
        assert!(check_nutrition(&macros(100.0, -1.0, 10.0, 1.0, 0.0)).is_err());
        assert!(check_nutrition(&macros(500.0, 40.0, 40.0, 30.0, 0.0)).is_err());
        assert!(check_nutrition(&macros(950.0, 0.0, 0.0, 100.0, 0.0)).is_err());
    }

    #[test]
    fn check_replaces_kcal_with_macro_energy_when_far_off() {
        // e = 4·2.6 + 4·(25.9 − 0.3) + 9·0.3 + 2·0.3 = 116.1; 300 is far off.
        let checked = check_nutrition(&macros(300.0, 2.6, 25.9, 0.3, 0.3)).unwrap();
        assert!((checked.kcal - 116.1).abs() < 1e-9);
    }

    #[test]
    fn check_keeps_kcal_within_tolerance() {
        // 130 vs e = 116.1: within max(20, 17.4).
        let checked = check_nutrition(&macros(130.0, 2.6, 25.9, 0.3, 0.3)).unwrap();
        assert_eq!(checked.kcal, 130.0);
    }

    async fn table() -> (FoodTable, lifeagent_core::test_support::TestEnv) {
        let env = crate::testutil::env().await;
        (FoodTable::new(&env.ctx), env)
    }

    #[tokio::test]
    async fn insert_then_find_by_name_is_case_and_space_insensitive() {
        let (table, _env) = table().await;
        let food = table
            .insert("Greek Yogurt", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        assert_eq!(food.name, "Greek Yogurt");
        assert_eq!(table.find(" greek  yogurt ").await.unwrap(), Some(food));
    }

    #[tokio::test]
    async fn insert_never_overwrites_an_existing_food() {
        let (table, _env) = table().await;
        let user = table
            .upsert_user("熟米饭", macros(120.0, 3.0, 26.0, 0.5, 0.4))
            .await
            .unwrap();
        let again = table
            .insert("熟米饭", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        assert_eq!(again.id, user.id);
        assert_eq!(again.source, NutritionSource::User);
        assert_eq!(again.per_100g.kcal, 120.0);
    }

    #[tokio::test]
    async fn upsert_user_updates_an_existing_food_in_place() {
        let (table, _env) = table().await;
        let estimated = table
            .insert("熟米饭", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        let corrected = macros(120.0, 3.0, 26.0, 0.5, 0.4);

        table.upsert_user("熟米饭", corrected).await.unwrap();

        let found = table.find("熟米饭").await.unwrap().unwrap();
        assert_eq!(found.id, estimated.id);
        assert_eq!(found.per_100g, corrected);
        assert_eq!(found.source, NutritionSource::User);
    }

    #[tokio::test]
    async fn add_alias_makes_the_food_findable_by_the_alias() {
        let (table, _env) = table().await;
        let food = table
            .insert(
                "卤鸡蛋",
                macros(150.0, 13.0, 1.5, 10.0, 0.0),
                NutritionSource::LlmEstimate,
            )
            .await
            .unwrap();

        table.add_alias("卤鸡蛋", "卤蛋").await.unwrap();

        assert_eq!(table.find("卤蛋").await.unwrap().unwrap().id, food.id);
    }

    #[tokio::test]
    async fn add_alias_rejects_an_alias_that_names_another_food() {
        let (table, _env) = table().await;
        table
            .insert("卤鸡蛋", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        table
            .insert("鸡蛋", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        assert!(matches!(
            table.add_alias("卤鸡蛋", "鸡蛋").await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn add_alias_to_unknown_food_is_not_found() {
        let (table, _env) = table().await;
        assert!(matches!(
            table.add_alias("不存在", "x").await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn known_names_are_most_used_first() {
        let (table, _env) = table().await;
        let a = table
            .insert("a", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        table
            .insert("b", rice(), NutritionSource::LlmEstimate)
            .await
            .unwrap();
        table.record_use(&a.id).await.unwrap();
        table.record_use(&a.id).await.unwrap();

        assert_eq!(table.known_names(150).await.unwrap(), vec!["a", "b"]);
        assert_eq!(table.known_names(1).await.unwrap(), vec!["a"]);
    }
}
