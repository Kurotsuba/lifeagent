//! `PluginManifest`, `PluginContext`, the `Plugin` trait and `PluginError`, per
//! `docs/PHASE1_SPEC.md` §7.6, plus plugin-migration application (§5 "Plugin migrations").

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::router::LlmRouter;
use lifeagent_llm::types::ToolSpec;
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::events::{Event, EventStore};
use crate::goals::GoalsService;
use crate::memory::MemoryService;
use crate::profile::ProfileService;
use crate::settings::Settings;

/// One versioned schema change a plugin supplies. `sql` must be a single statement; core
/// applies missing versions in order, each inside its own transaction, and records them in
/// `plugin_migrations`. Plugin tables must be prefixed with the plugin id (e.g. `diet_`).
pub struct PluginMigration {
    pub version: u32,
    pub sql: &'static str,
}

pub struct PluginManifest {
    /// Also the tool-name and table prefix (e.g. `"diet"` -> tools `diet_*`, tables `diet_*`).
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub event_types: Vec<&'static str>,
    pub migrations: Vec<PluginMigration>,
}

/// Everything a plugin needs to do its work. Assembled fresh by `CoreApp` for each call —
/// cheap, since every field is a cloneable handle (`SqlitePool`, `Arc<...>`, or a service
/// wrapping one).
#[derive(Clone)]
pub struct PluginContext {
    pub db: SqlitePool,
    pub events: EventStore,
    pub llm: Arc<LlmRouter>,
    pub clock: Arc<dyn Clock>,
    pub settings: Settings,
    /// Profile access. Active goals and the latest weight (§7.6's "profile, active goals,
    /// latest weight") live on `goals` and `events` respectively rather than being folded into
    /// this one field.
    pub profile: ProfileService,
    pub goals: GoalsService,
    pub memory: MemoryService,
}

/// What a tool call returns: JSON handed back to the LLM as the tool result, plus a short
/// human-readable line for UI activity chips.
pub struct ToolOutput {
    pub content: Value,
    pub summary: String,
}

/// A plugin's contribution to the next plan (M5).
pub struct PlanInput {
    pub section: String,
    pub brief: String,
    pub targets: Value,
    pub output_schema: Value,
}

/// The result of validating and enriching a plugin's raw planner output for one section (M5).
pub struct PlanSection {
    pub data: Value,
    pub warnings: Vec<String>,
    pub violations: Vec<String>,
}

#[derive(Debug, Error)]
pub enum PluginError {
    /// Returned to the LLM as a tool error so it can fix the call.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("unsupported")]
    Unsupported,
    #[error("{0}")]
    Other(String),
}

impl From<CoreError> for PluginError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NotFound(m) => PluginError::NotFound(m),
            CoreError::Validation(m) => PluginError::InvalidArgs(m),
            other => PluginError::Other(other.to_string()),
        }
    }
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;
    fn tools(&self) -> Vec<ToolSpec>;
    async fn call_tool(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: Value,
    ) -> Result<ToolOutput, PluginError>;

    async fn on_event(&self, _ctx: &PluginContext, _e: &Event) -> Result<(), PluginError> {
        Ok(())
    }

    async fn context_snippet(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
    ) -> Result<Option<String>, PluginError> {
        Ok(None)
    }

    async fn today_card(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
    ) -> Result<Option<Value>, PluginError> {
        Ok(None)
    }

    async fn plan_input(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
    ) -> Result<Option<PlanInput>, PluginError> {
        Ok(None)
    }

    async fn finalize_plan_section(
        &self,
        _ctx: &PluginContext,
        _date: NaiveDate,
        _raw: Value,
    ) -> Result<PlanSection, PluginError> {
        Err(PluginError::Unsupported)
    }
}

/// Applies any of `manifest`'s migrations not yet recorded in `plugin_migrations`, in version
/// order, each inside its own transaction (§5 "Plugin migrations").
pub async fn apply_migrations(
    db: &SqlitePool,
    clock: &dyn Clock,
    manifest: &PluginManifest,
) -> Result<(), CoreError> {
    let applied_rows: Vec<(i64,)> =
        sqlx::query_as("SELECT version FROM plugin_migrations WHERE plugin_id = ?")
            .bind(manifest.id)
            .fetch_all(db)
            .await?;
    let applied: HashSet<i64> = applied_rows.into_iter().map(|(v,)| v).collect();

    let mut migrations: Vec<&PluginMigration> = manifest.migrations.iter().collect();
    migrations.sort_by_key(|m| m.version);

    for migration in migrations {
        if applied.contains(&(migration.version as i64)) {
            continue;
        }
        let mut tx = db.begin().await?;
        sqlx::query(migration.sql).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO plugin_migrations (plugin_id, version, applied_at) VALUES (?, ?, ?)",
        )
        .bind(manifest.id)
        .bind(migration.version as i64)
        .bind(clock.now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn manifest(migrations: Vec<PluginMigration>) -> PluginManifest {
        PluginManifest {
            id: "test",
            name: "Test",
            version: "0.1.0",
            event_types: vec![],
            migrations,
        }
    }

    #[tokio::test]
    async fn applies_missing_migrations_in_order_and_records_them() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = crate::clock::FixedClock::new(Utc::now());

        let manifest = manifest(vec![
            PluginMigration {
                version: 1,
                sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
            },
            PluginMigration {
                version: 2,
                sql: "ALTER TABLE test_things ADD COLUMN name TEXT",
            },
        ]);

        apply_migrations(&db, &clock, &manifest).await.unwrap();

        // Both migrations landed: the table has the column the second one added.
        sqlx::query("INSERT INTO test_things (id, name) VALUES ('a', 'b')")
            .execute(&db)
            .await
            .unwrap();

        let applied: Vec<(i64,)> = sqlx::query_as(
            "SELECT version FROM plugin_migrations WHERE plugin_id = 'test' ORDER BY version",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(applied, vec![(1,), (2,)]);
    }

    #[tokio::test]
    async fn does_not_reapply_already_applied_migrations() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = crate::clock::FixedClock::new(Utc::now());
        let manifest = manifest(vec![PluginMigration {
            version: 1,
            sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
        }]);

        apply_migrations(&db, &clock, &manifest).await.unwrap();
        // A second run must not try to CREATE TABLE again (which would error).
        apply_migrations(&db, &clock, &manifest).await.unwrap();
    }

    #[tokio::test]
    async fn only_applies_migrations_not_yet_recorded() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = crate::clock::FixedClock::new(Utc::now());

        apply_migrations(
            &db,
            &clock,
            &manifest(vec![PluginMigration {
                version: 1,
                sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
            }]),
        )
        .await
        .unwrap();

        // A later run with an additional migration applies only the new one.
        apply_migrations(
            &db,
            &clock,
            &manifest(vec![
                PluginMigration {
                    version: 1,
                    sql: "CREATE TABLE test_things (id TEXT PRIMARY KEY)",
                },
                PluginMigration {
                    version: 2,
                    sql: "ALTER TABLE test_things ADD COLUMN name TEXT",
                },
            ]),
        )
        .await
        .unwrap();

        let applied: Vec<(i64,)> = sqlx::query_as(
            "SELECT version FROM plugin_migrations WHERE plugin_id = 'test' ORDER BY version",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(applied, vec![(1,), (2,)]);
    }

    #[test]
    fn plugin_error_from_core_error_maps_variants() {
        assert!(matches!(
            PluginError::from(CoreError::NotFound("x".into())),
            PluginError::NotFound(_)
        ));
        assert!(matches!(
            PluginError::from(CoreError::Validation("x".into())),
            PluginError::InvalidArgs(_)
        ));
        assert!(matches!(
            PluginError::from(CoreError::Secret("x".into())),
            PluginError::Other(_)
        ));
    }
}
