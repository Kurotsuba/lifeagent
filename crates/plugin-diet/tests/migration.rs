//! Migration 2 (pluggable sources) against a Phase 1 database that already has foods and
//! aliases, per `docs/NUTRITION_SOURCES.md` §3.2 and §11 "Migration".

use lifeagent_core::plugin::{apply_migrations, Plugin, PluginManifest, PluginMigration};
use lifeagent_plugin_diet::sources::{self, SourceChain};
use lifeagent_plugin_diet::{foods, DietPlugin};

/// Only migration 1 — a stand-in for a database that hasn't seen migration 2 yet.
fn migration_1_only() -> PluginManifest {
    PluginManifest {
        id: "diet",
        name: "Diet",
        version: "0.1.0",
        event_types: vec![],
        migrations: vec![PluginMigration {
            version: 1,
            sql: foods::MIGRATION_1,
        }],
    }
}

#[tokio::test]
async fn migrates_phase1_foods_and_aliases_preserving_ids_values_use_counts_and_lookups() {
    let env = lifeagent_core::test_support::env().await;
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &migration_1_only())
        .await
        .unwrap();

    // Seed Phase 1 rows directly, as `FoodTable::insert`/`add_alias` would have written them.
    sqlx::query(
        "INSERT INTO diet_foods
            (id, name, name_key, aliases_json, kcal, protein_g, carbs_g, fat_g, fiber_g,
             source, use_count, created_at, updated_at)
         VALUES ('id-egg', '卤鸡蛋', '卤鸡蛋', '[\"卤蛋\"]', 150, 13, 1, 10, 0,
                 'user', 5, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
    )
    .execute(&env.ctx.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO diet_foods
            (id, name, name_key, aliases_json, kcal, protein_g, carbs_g, fat_g, fiber_g,
             source, use_count, created_at, updated_at)
         VALUES ('id-rice', '熟米饭', '熟米饭', '[]', 116, 2.6, 25.9, 0.3, 0.3,
                 'llm_estimate', 2, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
    )
    .execute(&env.ctx.db)
    .await
    .unwrap();

    // Migration 2 now runs (via the full manifest, which sees version 1 already applied).
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &DietPlugin.manifest())
        .await
        .unwrap();

    let chain = SourceChain::load(&env.ctx).await.unwrap();

    let egg = chain.lookup("卤鸡蛋").await.unwrap().unwrap();
    assert_eq!(egg.id, "id-egg");
    assert_eq!(egg.source_id, sources::MANUAL_SOURCE_ID);
    assert_eq!(egg.use_count, 5);
    assert_eq!(egg.per_100g.kcal, 150.0);

    let rice = chain.lookup("熟米饭").await.unwrap().unwrap();
    assert_eq!(rice.id, "id-rice");
    assert_eq!(rice.source_id, sources::LLM_ESTIMATE_SOURCE_ID);
    assert_eq!(rice.use_count, 2);

    // The user alias survived and still resolves to the same food.
    let via_alias = chain.lookup("卤蛋").await.unwrap().unwrap();
    assert_eq!(via_alias.id, "id-egg");

    // Both built-in sources exist alongside the migrated data.
    let all = sources::list_sources(&env.ctx).await.unwrap();
    let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&sources::MANUAL_SOURCE_ID));
    assert!(ids.contains(&sources::LLM_ESTIMATE_SOURCE_ID));

    let violations: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&env.ctx.db)
        .await
        .unwrap();
    assert!(violations.is_empty(), "{violations:?}");
}

/// Only migrations 1 and 2 — a stand-in for a database that hasn't seen migration 3 yet.
fn migration_1_and_2_only() -> PluginManifest {
    PluginManifest {
        id: "diet",
        name: "Diet",
        version: "0.1.0",
        event_types: vec![],
        migrations: vec![
            PluginMigration {
                version: 1,
                sql: foods::MIGRATION_1,
            },
            PluginMigration {
                version: 2,
                sql: sources::MIGRATION_2,
            },
        ],
    }
}

#[tokio::test]
async fn migration_3_is_additive_and_leaves_existing_foods_untouched() {
    let env = lifeagent_core::test_support::env().await;
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &migration_1_and_2_only())
        .await
        .unwrap();
    sources::upsert_manual_food(
        &env.ctx,
        "熟米饭",
        lifeagent_plugin_diet::model::Macros {
            kcal: 116.0,
            protein_g: 2.6,
            carbs_g: 25.9,
            fat_g: 0.3,
            fiber_g: 0.3,
        },
    )
    .await
    .unwrap();

    // Migration 3 now runs (via the full manifest, which sees versions 1-2 already applied).
    apply_migrations(&env.ctx.db, env.clock.as_ref(), &DietPlugin.manifest())
        .await
        .unwrap();

    let chain = SourceChain::load(&env.ctx).await.unwrap();
    let rice = chain.lookup("熟米饭").await.unwrap().unwrap();
    assert_eq!(rice.source_id, sources::MANUAL_SOURCE_ID);

    // The new dish is reachable through the same chain and the schema is sound.
    assert!(chain.lookup_dish("松屋", "牛めし").await.unwrap().is_none());

    let violations: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&env.ctx.db)
        .await
        .unwrap();
    assert!(violations.is_empty(), "{violations:?}");
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
         VALUES ('ds1', 'dataset', '测试菜单', 1, 1, '{}', ?, ?)",
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
         VALUES ('dish1', 'ds1', '松屋', '松屋', '牛めし', '牛めし', 350, 733, 25, 90, 28, 3, 0, ?, ?)",
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

    let violations: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&env.ctx.db)
        .await
        .unwrap();
    assert!(violations.is_empty(), "{violations:?}");
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
         VALUES ('ds1', 'dataset', '测试数据集', 1, 1, '{}', ?, ?)",
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
         VALUES ('food1', 'ds1', '芹菜', '芹菜', 16, 0.8, 3.3, 0.2, 1.4, 0, ?, ?)",
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

    let violations: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&env.ctx.db)
        .await
        .unwrap();
    assert!(violations.is_empty(), "{violations:?}");
}
