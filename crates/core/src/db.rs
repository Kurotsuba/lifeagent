//! Postgres pool setup and core migrations, per `docs/PHASE1_SPEC.md` §3/§5 and
//! `docs/HACKATHON.md` §1.
//!
//! All of one user's tables live in one Postgres schema. Every pooled connection starts with
//! `search_path` set to that schema, so queries name tables unqualified and never see another
//! schema's data. Migrations are embedded at compile time from `crates/core/migrations` and
//! applied on every open, so opening is always safe to call.

use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

use crate::error::CoreError;

/// Where a `CoreApp`'s data lives: a Postgres server and the schema holding its tables.
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// A `postgres://` connection URL.
    pub url: String,
    /// The schema holding this app's tables. Must match `[a-z_][a-z0-9_]*`, at most 63 bytes.
    pub schema: String,
}

/// The local development database (`docs/HACKATHON.md` §1, "Steps").
pub const DEFAULT_DATABASE_URL: &str = "postgres://lifeagent:lifeagent@localhost/lifeagent_dev";

impl DbConfig {
    /// `LIFEAGENT_DATABASE_URL` (default [`DEFAULT_DATABASE_URL`]) and `LIFEAGENT_DB_SCHEMA`
    /// (default `public`). The composition roots' shared fallback when nothing more specific
    /// is given.
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("LIFEAGENT_DATABASE_URL")
                .unwrap_or_else(|_| DEFAULT_DATABASE_URL.into()),
            schema: std::env::var("LIFEAGENT_DB_SCHEMA").unwrap_or_else(|_| "public".into()),
        }
    }
}

/// Rejects anything that isn't a plain lowercase identifier, so `schema` can be spliced into
/// DDL (identifiers can't be bind parameters) without any quoting concerns.
fn validate_schema(schema: &str) -> Result<(), CoreError> {
    let mut chars = schema.chars();
    let valid = schema.len() <= 63
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(CoreError::Validation(format!(
            "invalid schema name {schema:?}: expected [a-z_][a-z0-9_]*, at most 63 bytes"
        )))
    }
}

/// Creates `config.schema` if needed, returns a pool whose connections all use it as their
/// `search_path`, and applies pending core migrations inside it.
pub async fn open(config: &DbConfig) -> Result<PgPool, CoreError> {
    validate_schema(&config.schema)?;
    let base = PgConnectOptions::from_str(&config.url)?;

    // The scoped pool below can't create its own schema: with `search_path` pointing at a
    // schema that doesn't exist yet, Postgres has nowhere to put the migrations table.
    let setup = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(base.clone())
        .await?;
    sqlx::query(AssertSqlSafe(format!(
        "CREATE SCHEMA IF NOT EXISTS \"{}\"",
        config.schema
    )))
    .execute(&setup)
    .await?;
    setup.close().await;

    let options = base.options([("search_path", format!("\"{}\"", config.schema))]);
    let pool = PgPoolOptions::new().connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Test-only: a fresh, uniquely named schema on the test server, dropped with everything in it
/// when the guard is dropped. Tests in parallel each get their own schema, so they never see
/// each other's rows.
///
/// The server comes from `LIFEAGENT_TEST_DATABASE_URL`, defaulting to the local development
/// setup in `docs/HACKATHON.md` (`postgres://lifeagent:lifeagent@localhost/lifeagent_test`).
#[cfg(any(test, feature = "test-support"))]
pub struct TestDb {
    pub config: DbConfig,
}

#[cfg(any(test, feature = "test-support"))]
impl TestDb {
    pub fn new() -> Self {
        let url = std::env::var("LIFEAGENT_TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://lifeagent:lifeagent@localhost/lifeagent_test".into());
        let schema = format!("test_{}", uuid::Uuid::now_v7().simple());
        Self {
            config: DbConfig { url, schema },
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for TestDb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestDb {
    fn drop(&mut self) {
        // Drop can't be async, and the test's own runtime may already be shutting down, so the
        // cleanup gets a thread and a runtime of its own. Best effort: a schema left behind by
        // a crash is harmless, since every test uses a new name.
        let config = self.config.clone();
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async {
                if let Ok(pool) = PgPoolOptions::new()
                    .max_connections(1)
                    .connect(&config.url)
                    .await
                {
                    let _ = sqlx::query(AssertSqlSafe(format!(
                        "DROP SCHEMA IF EXISTS \"{}\" CASCADE",
                        config.schema
                    )))
                    .execute(&pool)
                    .await;
                    pool.close().await;
                }
            });
        })
        .join();
    }
}

/// Test-only helper shared across this crate's modules: an isolated, fully-migrated pool in
/// its own schema. Keep the returned [`TestDb`] alive while the pool is in use — dropping it
/// drops the schema.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn test_pool() -> (PgPool, TestDb) {
    let db = TestDb::new();
    let pool = open(&db.config).await.expect("open test db");
    (pool, db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_names_must_be_plain_lowercase_identifiers() {
        assert!(validate_schema("u_0192abc").is_ok());
        assert!(validate_schema("_x").is_ok());
        assert!(validate_schema("").is_err());
        assert!(validate_schema("1abc").is_err());
        assert!(validate_schema("Upper").is_err());
        assert!(validate_schema("a\"; DROP SCHEMA public; --").is_err());
        assert!(validate_schema(&"a".repeat(64)).is_err());
    }

    #[tokio::test]
    async fn open_creates_the_schema_and_applies_migrations() {
        let (pool, db) = test_pool().await;

        // A representative table from the migration should exist, be empty, and live in the
        // test's own schema.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
            .fetch_one(&pool)
            .await
            .expect("query settings table");
        assert_eq!(count, 0);
        let schema: String = sqlx::query_scalar(
            "SELECT table_schema::TEXT FROM information_schema.tables WHERE table_name = 'settings'
             AND table_schema = current_schema()",
        )
        .fetch_one(&pool)
        .await
        .expect("find settings table");
        assert_eq!(schema, db.config.schema);
    }

    #[tokio::test]
    async fn open_is_idempotent() {
        let db = TestDb::new();
        open(&db.config).await.expect("first open");
        // Re-opening the same schema must not fail or re-apply migrations destructively.
        let pool = open(&db.config).await.expect("second open");

        sqlx::query("INSERT INTO settings (key, value_json, updated_at) VALUES ('k', '1', 'now')")
            .execute(&pool)
            .await
            .expect("insert works after reopen");
    }

    #[tokio::test]
    async fn schemas_do_not_see_each_others_rows() {
        let (a, _a_db) = test_pool().await;
        let (b, _b_db) = test_pool().await;

        sqlx::query("INSERT INTO settings (key, value_json, updated_at) VALUES ('k', '1', 'now')")
            .execute(&a)
            .await
            .unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
            .fetch_one(&b)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let (pool, _db) = test_pool().await;
        sqlx::query("CREATE TABLE parent (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE child (parent_id TEXT NOT NULL REFERENCES parent(id))")
            .execute(&pool)
            .await
            .unwrap();
        let result = sqlx::query("INSERT INTO child (parent_id) VALUES ('missing')")
            .execute(&pool)
            .await;
        assert!(result.is_err());
    }
}
