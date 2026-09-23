//! SQLite pool setup and core migrations, per `docs/PHASE1_SPEC.md` §3/§5.
//!
//! WAL mode and foreign keys are turned on per connection (see `CLAUDE.md` §12). Migrations
//! are embedded at compile time from `crates/core/migrations` and applied on every open, so
//! opening the database is always safe to call.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::error::CoreError;

/// Opens (creating if needed) `{data_dir}/lifeagent.db` and applies pending core migrations.
pub async fn open(data_dir: &Path) -> Result<SqlitePool, CoreError> {
    std::fs::create_dir_all(data_dir)?;

    let options = SqliteConnectOptions::new()
        .filename(data_dir.join("lifeagent.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new().connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Test-only helper shared across this crate's modules: an isolated, fully-migrated pool
/// backed by a temp directory that lives as long as the returned guard. Keep the
/// [`tempfile::TempDir`] alive for as long as the pool is in use — dropping it deletes the
/// database file.
#[cfg(test)]
pub(crate) async fn test_pool() -> (SqlitePool, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let pool = open(dir.path()).await.expect("open test db");
    (pool, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_data_dir_and_applies_migrations() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pool = open(dir.path()).await.expect("open db");

        assert!(dir.path().join("lifeagent.db").exists());

        // A representative table from the migration should exist and be empty.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
            .fetch_one(&pool)
            .await
            .expect("query settings table");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn open_is_idempotent() {
        let dir = tempfile::tempdir().expect("create temp dir");
        open(dir.path()).await.expect("first open");
        // Re-opening the same directory must not fail or re-apply migrations destructively.
        let pool = open(dir.path()).await.expect("second open");

        sqlx::query("INSERT INTO settings (key, value_json, updated_at) VALUES ('k', '1', 'now')")
            .execute(&pool)
            .await
            .expect("insert works after reopen");
    }

    #[tokio::test]
    async fn wal_mode_and_foreign_keys_are_enabled() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let pool = open(dir.path()).await.expect("open db");

        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .expect("query journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("query foreign_keys");
        assert_eq!(foreign_keys, 1);
    }
}
