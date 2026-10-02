//! The diet plugin's Postgres migration (`sources::MIGRATION_1`): the seeded sources and the
//! `ON DELETE CASCADE` rules that `sources::delete_source` relies on, per
//! `docs/NUTRITION_SOURCES.md` §3 and `docs/DISH_NUTRITION.md` §3.
//!
//! The SQLite build's upgrade-path tests (migrations 1→2→3→4) went away with that history; see
//! `docs/DECISIONS.md`, "Postgres port".

use lifeagent_core::plugin::{apply_migrations, Plugin};
use lifeagent_plugin_diet::sources;
use lifeagent_plugin_diet::DietPlugin;

#[tokio::test]
async fn seeds_the_manual_and_llm_estimate_sources_with_rfc3339_timestamps() {
    let env = lifeagent_core::test_support::env().await;
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &DietPlugin.manifest())
        .await
        .unwrap();

    let rows: Vec<(String, String, i64, i64, String)> = sqlx::query_as(
        "SELECT id, kind, priority, enabled, created_at FROM diet_food_sources ORDER BY priority",
    )
    .fetch_all(&env.ctx.db)
    .await
    .unwrap();
    let ids: Vec<(&str, &str, i64, i64)> = rows
        .iter()
        .map(|(id, kind, priority, enabled, _)| (id.as_str(), kind.as_str(), *priority, *enabled))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("manual", "manual", 0, 1),
            ("llm_estimate", "llm_estimate", 1_000_000, 1)
        ]
    );
    for (_, _, _, _, created_at) in &rows {
        chrono::DateTime::parse_from_rfc3339(created_at)
            .unwrap_or_else(|e| panic!("{created_at:?} is not RFC 3339: {e}"));
    }
}

#[tokio::test]
async fn deleting_a_dataset_cascades_into_dishes_and_dish_source_aliases() {
    let env = lifeagent_core::test_support::env().await;
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &DietPlugin.manifest())
        .await
        .unwrap();

    let now = "2026-01-01T00:00:00+00:00";
    sqlx::query(
        "INSERT INTO diet_food_sources (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
         VALUES ('ds1', 'dataset', '测试菜单', 1, 1, '{}', $1, $2)",
    )
    .bind(now)
    .bind(now)
    .execute(&env.ctx.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO diet_dishes
            (id, source_id, brand, brand_key, name, name_key, serving_g, kcal, protein_g,
             carbs_g, fat_g, fiber_g, use_count, created_at, updated_at)
         VALUES ('dish1', 'ds1', '松屋', '松屋', '牛めし', '牛めし', 350, 733, 25, 90, 28, 3, 0, $1, $2)",
    )
    .bind(now)
    .bind(now)
    .execute(&env.ctx.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO diet_dish_source_aliases (source_id, brand_key, alias_key, dish_id)
         VALUES ('ds1', '松屋', '牛丼並盛', 'dish1')",
    )
    .execute(&env.ctx.db)
    .await
    .unwrap();

    sources::delete_source(&env.ctx, "ds1").await.unwrap();

    let dish_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM diet_dishes WHERE source_id = 'ds1'")
            .fetch_one(&env.ctx.db)
            .await
            .unwrap();
    assert_eq!(dish_count.0, 0);
    let alias_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM diet_dish_source_aliases WHERE source_id = 'ds1'")
            .fetch_one(&env.ctx.db)
            .await
            .unwrap();
    assert_eq!(alias_count.0, 0);
}

#[tokio::test]
async fn deleting_a_dataset_cascades_into_foods_and_source_aliases() {
    let env = lifeagent_core::test_support::env().await;
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &DietPlugin.manifest())
        .await
        .unwrap();

    let now = "2026-01-01T00:00:00+00:00";
    sqlx::query(
        "INSERT INTO diet_food_sources (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
         VALUES ('ds1', 'dataset', '测试数据集', 1, 1, '{}', $1, $2)",
    )
    .bind(now)
    .bind(now)
    .execute(&env.ctx.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO diet_foods
            (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
             use_count, created_at, updated_at)
         VALUES ('food1', 'ds1', '芹菜', '芹菜', 16, 0.8, 3.3, 0.2, 1.4, 0, $1, $2)",
    )
    .bind(now)
    .bind(now)
    .execute(&env.ctx.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO diet_food_aliases (source_id, alias_key, food_id) VALUES ('ds1', '西芹', 'food1')")
        .execute(&env.ctx.db)
        .await
        .unwrap();

    sources::delete_source(&env.ctx, "ds1").await.unwrap();

    let food_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM diet_foods WHERE source_id = 'ds1'")
            .fetch_one(&env.ctx.db)
            .await
            .unwrap();
    assert_eq!(food_count.0, 0);
    let alias_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM diet_food_aliases WHERE source_id = 'ds1'")
            .fetch_one(&env.ctx.db)
            .await
            .unwrap();
    assert_eq!(alias_count.0, 0);
}
