//! Postgres pool setup, per-user schemas and core migrations, per `docs/PHASE1_SPEC.md` §3/§5
//! and `docs/HACKATHON.md` §1 ("Account management").
//!
//! All of one user's tables live in one Postgres schema. Every pooled connection starts with
//! `search_path` set to that schema, so queries name tables unqualified and stay single-user.
//!
//! In tenant mode ([`DbConfig::tenant`]) the schema is also owned by a `LOGIN` role of the
//! same name, and every connection in the tenant's pool logs in as that role. The tenant is a
//! member of no other role, so its sessions can't `SET ROLE` or `SET SESSION AUTHORIZATION`
//! into anything else, and a query that names another tenant's schema gets "permission
//! denied" from Postgres rather than another user's rows. That holds even for a connection
//! running arbitrary SQL.
//!
//! The server's login role (the one in [`DbConfig::url`], with `CREATEROLE`) creates each
//! tenant and so holds `ADMIN` on it, but keeps no `SET` membership: it can't read tenant data
//! as itself, and grants itself `SET` only inside the provisioning and dropping transactions.
//!
//! A tenant's password is never stored: it is an HMAC-SHA256 of the role name keyed by the
//! login URL, which carries the login role's password and is therefore exactly as secret as
//! the credential that already controls every tenant. Postgres receives only a SCRAM verifier,
//! so the plaintext never appears in a statement or a server log.
//!
//! Migrations are embedded at compile time from `crates/core/migrations` and applied on every
//! open, so opening is always safe to call.

use std::str::FromStr;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};

use crate::error::CoreError;

/// Where a `CoreApp`'s data lives, and who its connections act as.
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// A `postgres://` connection URL for the login role.
    pub url: String,
    /// The schema holding this app's tables. Must match `[a-z_][a-z0-9_]*`, at most 63 bytes.
    pub schema: String,
    /// `Some`: tenant mode. The schema is owned by this `LOGIN` role and every connection logs
    /// in as it. `None`: connections act as the login role itself (single-user CLI/desktop).
    pub role: Option<String>,
    /// Upper bound on this pool's connections. Tenant pools are small: one server holds one
    /// per active user, and Cloud SQL's smallest tier allows about 25 connections in total.
    pub max_connections: u32,
}

/// The local development database (`docs/HACKATHON.md` §1, "Steps").
pub const DEFAULT_DATABASE_URL: &str = "postgres://lifeagent:lifeagent@localhost/lifeagent_dev";

/// sqlx's own default, kept for single-user pools.
const SINGLE_USER_MAX_CONNECTIONS: u32 = 10;
const TENANT_MAX_CONNECTIONS: u32 = 2;
/// Tenant pools give connections back quickly, so idle users don't hold the server's budget.
const TENANT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

impl DbConfig {
    /// `LIFEAGENT_DATABASE_URL` (default [`DEFAULT_DATABASE_URL`]) and `LIFEAGENT_DB_SCHEMA`
    /// (default `public`), acting as the login role. The single-user composition roots' shared
    /// fallback when nothing more specific is given.
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("LIFEAGENT_DATABASE_URL")
                .unwrap_or_else(|_| DEFAULT_DATABASE_URL.into()),
            schema: std::env::var("LIFEAGENT_DB_SCHEMA").unwrap_or_else(|_| "public".into()),
            role: None,
            max_connections: SINGLE_USER_MAX_CONNECTIONS,
        }
    }

    /// One user's data: schema `name`, owned by role `name`, which every connection logs in as.
    /// The login role in `url` needs `CREATEROLE` and `CREATE` on the database, as Cloud SQL's
    /// built-in users have. Its URL is also the key for the tenant's password, so changing
    /// it changes every tenant's password; [`open`] re-applies the password each time.
    pub fn tenant(url: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            url: url.into(),
            schema: name.clone(),
            role: Some(name),
            max_connections: TENANT_MAX_CONNECTIONS,
        }
    }
}

/// Rejects anything that isn't a plain lowercase identifier, so schema and role names can be
/// spliced into DDL (identifiers can't be bind parameters) without any quoting concerns.
pub fn validate_identifier(kind: &str, name: &str) -> Result<(), CoreError> {
    let mut chars = name.chars();
    let valid = name.len() <= 63
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(CoreError::Validation(format!(
            "invalid {kind} name {name:?}: expected [a-z_][a-z0-9_]*, at most 63 bytes"
        )))
    }
}

fn validate(config: &DbConfig) -> Result<(), CoreError> {
    validate_identifier("schema", &config.schema)?;
    if let Some(role) = &config.role {
        validate_identifier("role", role)?;
    }
    Ok(())
}

/// Serializes provisioning and dropping across every app instance sharing the database, so
/// two first logins of the same user can't both try to create its role.
const PROVISION_LOCK: &str = "SELECT pg_advisory_xact_lock(hashtext('lifeagent.provision'))";

/// PBKDF2 rounds in the stored SCRAM verifier: Postgres's own default.
const SCRAM_ITERATIONS: u32 = 4096;

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    // HMAC accepts keys of any length, so `new_from_slice` cannot fail.
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 accepts keys of any length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// The tenant role's login password: hex HMAC-SHA256 of its name, keyed by the login URL.
fn tenant_password(url: &str, role: &str) -> String {
    hmac_sha256(
        url.as_bytes(),
        &[b"lifeagent tenant password\0", role.as_bytes()],
    )
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect()
}

/// The SCRAM-SHA-256 verifier Postgres stores for [`tenant_password`] (RFC 5802 §3, in
/// Postgres's `SCRAM-SHA-256$<iterations>:<salt>$<StoredKey>:<ServerKey>` form). Handing
/// `CREATE`/`ALTER ROLE` the verifier keeps the password itself out of SQL text, where it could
/// be logged. The salt is derived too, so re-applying it on every open writes the same value.
fn tenant_scram_verifier(url: &str, role: &str) -> String {
    let password = tenant_password(url, role);
    let salt = &hmac_sha256(
        url.as_bytes(),
        &[b"lifeagent tenant salt\0", role.as_bytes()],
    )[..16];

    // Hi(password, salt, i): PBKDF2-HMAC-SHA256 producing a single block.
    let mut block = hmac_sha256(password.as_bytes(), &[salt, &1u32.to_be_bytes()]);
    let mut salted = block;
    for _ in 1..SCRAM_ITERATIONS {
        block = hmac_sha256(password.as_bytes(), &[&block]);
        salted.iter_mut().zip(block).for_each(|(s, b)| *s ^= b);
    }
    let stored_key = Sha256::digest(hmac_sha256(&salted, &[b"Client Key"]));
    let server_key = hmac_sha256(&salted, &[b"Server Key"]);
    format!(
        "SCRAM-SHA-256${SCRAM_ITERATIONS}:{}${}:{}",
        BASE64.encode(salt),
        BASE64.encode(stored_key),
        BASE64.encode(server_key)
    )
}

async fn exec(conn: &mut PgConnection, sql: String) -> Result<(), CoreError> {
    sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Creates the schema (and in tenant mode its login role) if missing. Idempotent. In tenant
/// mode it also (re)applies the role's password and leaves the login role without `SET` on it,
/// which upgrades tenants made before tenants had their own logins.
async fn provision(base: &PgConnectOptions, config: &DbConfig) -> Result<(), CoreError> {
    let mut conn = PgConnection::connect_with(base).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(PROVISION_LOCK).execute(&mut *tx).await?;
    let schema = &config.schema;
    match &config.role {
        Some(role) => {
            let exists: Option<(i32,)> =
                sqlx::query_as("SELECT 1 FROM pg_roles WHERE rolname = $1")
                    .bind(role)
                    .fetch_optional(&mut *tx)
                    .await?;
            let verb = if exists.is_none() { "CREATE" } else { "ALTER" };
            // The verifier is base64 and `$:` only, so it can't break out of the literal.
            let verifier = tenant_scram_verifier(&config.url, role);
            exec(
                &mut tx,
                format!("{verb} ROLE \"{role}\" WITH LOGIN PASSWORD '{verifier}'"),
            )
            .await?;
            // `CREATE SCHEMA ... AUTHORIZATION` needs SET on the owner. Since Postgres 16 the
            // creator of a role gets ADMIN on it but not SET, so grant SET for this transaction
            // only. The revoke also removes the standing grant older tenants were created with;
            // it leaves the ADMIN membership from creation, which has a different grantor.
            exec(
                &mut tx,
                format!("GRANT \"{role}\" TO CURRENT_USER WITH INHERIT FALSE, SET TRUE"),
            )
            .await?;
            exec(
                &mut tx,
                format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\" AUTHORIZATION \"{role}\""),
            )
            .await?;
            exec(&mut tx, format!("REVOKE \"{role}\" FROM CURRENT_USER")).await?;
        }
        None => exec(&mut tx, format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"")).await?,
    }
    tx.commit().await?;
    conn.close().await?;
    Ok(())
}

/// Provisions `config`'s schema (and role) if needed, returns a pool whose connections all use
/// it as their `search_path` (and log in as its role), and applies pending core migrations
/// inside it, so tables are owned by the tenant role.
pub async fn open(config: &DbConfig) -> Result<PgPool, CoreError> {
    validate(config)?;
    let base = PgConnectOptions::from_str(&config.url)?;
    provision(&base, config).await?;

    // A startup parameter rather than `SET` after connecting, so every pooled connection has it.
    let mut options = base
        .clone()
        .options([("search_path", format!("\"{}\"", config.schema))]);
    let mut pool_options = PgPoolOptions::new().max_connections(config.max_connections);
    if let Some(role) = &config.role {
        // The tenant is the session user, not a role switched to: there is nothing to
        // `RESET ROLE` or `SET ROLE NONE` back to.
        options = options
            .username(role)
            .password(&tenant_password(&config.url, role));
        pool_options = pool_options.idle_timeout(TENANT_IDLE_TIMEOUT);
    }
    let pool = pool_options.connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Deletes `config`'s schema with everything in it, and in tenant mode its role: account
/// deletion. Idempotent. In tenant mode the login role isn't the schema's owner, so it grants
/// itself SET on the tenant for this transaction, drops the schema while acting as the tenant,
/// and then drops the role (and with it that grant) using the ADMIN option it got at creation.
pub async fn destroy(config: &DbConfig) -> Result<(), CoreError> {
    validate(config)?;
    let base = PgConnectOptions::from_str(&config.url)?;
    let mut conn = PgConnection::connect_with(&base).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(PROVISION_LOCK).execute(&mut *tx).await?;
    let schema = &config.schema;
    match &config.role {
        Some(role) => {
            let exists: Option<(i32,)> =
                sqlx::query_as("SELECT 1 FROM pg_roles WHERE rolname = $1")
                    .bind(role)
                    .fetch_optional(&mut *tx)
                    .await?;
            if exists.is_some() {
                exec(
                    &mut tx,
                    format!("GRANT \"{role}\" TO CURRENT_USER WITH INHERIT FALSE, SET TRUE"),
                )
                .await?;
                exec(&mut tx, format!("SET LOCAL ROLE \"{role}\"")).await?;
                exec(
                    &mut tx,
                    format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"),
                )
                .await?;
                exec(&mut tx, "RESET ROLE".into()).await?;
                exec(&mut tx, format!("DROP ROLE \"{role}\"")).await?;
            }
        }
        None => {
            exec(
                &mut tx,
                format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"),
            )
            .await?
        }
    }
    tx.commit().await?;
    conn.close().await?;
    Ok(())
}

/// Test-only: a fresh, uniquely named tenant (`test_<uuid v7>` schema and role) on the test
/// server, dropped with everything in it when the guard is dropped. Tests in parallel each get
/// their own tenant, so they never see each other's rows, and every test runs the way
/// production does: provisioned by a non-superuser login role, connected as the tenant's own
/// login role.
///
/// The login role comes from `LIFEAGENT_TEST_ADMIN_URL`, defaulting to
/// `postgres://lifeagent_test_admin:lifeagent_test_admin@localhost/lifeagent_test`. It is
/// created on first use with the superuser from `LIFEAGENT_TEST_DATABASE_URL` (default
/// `postgres://lifeagent:lifeagent@localhost/lifeagent_test`, the local setup in
/// `docs/HACKATHON.md`).
#[cfg(any(test, feature = "test-support"))]
pub struct TestDb {
    pub config: DbConfig,
}

#[cfg(any(test, feature = "test-support"))]
const TEST_ADMIN_ROLE: &str = "lifeagent_test_admin";

#[cfg(any(test, feature = "test-support"))]
fn test_superuser_url() -> String {
    std::env::var("LIFEAGENT_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://lifeagent:lifeagent@localhost/lifeagent_test".into())
}

#[cfg(any(test, feature = "test-support"))]
fn test_admin_url() -> String {
    std::env::var("LIFEAGENT_TEST_ADMIN_URL").unwrap_or_else(|_| {
        format!("postgres://{TEST_ADMIN_ROLE}:{TEST_ADMIN_ROLE}@localhost/lifeagent_test")
    })
}

/// Creates the non-superuser test login role if missing: `CREATEROLE` and `CREATE` on the
/// database, like Cloud SQL's built-in users, and nothing more.
#[cfg(any(test, feature = "test-support"))]
async fn ensure_test_admin() {
    static READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    READY
        .get_or_init(|| async {
            let mut conn = PgConnection::connect(&test_superuser_url())
                .await
                .expect("connect to the test database as its superuser");
            let mut tx = conn.begin().await.expect("begin");
            // Test binaries run in parallel processes, so the in-process OnceCell isn't enough.
            sqlx::query(PROVISION_LOCK)
                .execute(&mut *tx)
                .await
                .expect("lock");
            let exists: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM pg_roles WHERE rolname = $1")
                .bind(TEST_ADMIN_ROLE)
                .fetch_optional(&mut *tx)
                .await
                .expect("look up the test admin role");
            if exists.is_none() {
                exec(
                    &mut tx,
                    format!(
                        "CREATE ROLE {TEST_ADMIN_ROLE} LOGIN CREATEROLE PASSWORD '{TEST_ADMIN_ROLE}'"
                    ),
                )
                .await
                .expect("create the test admin role");
            }
            exec(
                &mut tx,
                format!(
                    "DO $$ BEGIN EXECUTE format('GRANT CREATE ON DATABASE %I TO {TEST_ADMIN_ROLE}', \
                     current_database()); END $$"
                ),
            )
            .await
            .expect("grant CREATE on the test database");
            tx.commit().await.expect("commit");
        })
        .await;
}

/// The test login role's URL, creating the role on first use: for tests that need to connect
/// the way a server does (e.g. `lifeagent-server`'s), rather than through a [`TestDb`].
#[cfg(any(test, feature = "test-support"))]
pub async fn test_login_url() -> String {
    ensure_test_admin().await;
    test_admin_url()
}

#[cfg(any(test, feature = "test-support"))]
impl TestDb {
    pub async fn new() -> Self {
        ensure_test_admin().await;
        let name = format!("test_{}", uuid::Uuid::now_v7().simple());
        Self {
            config: DbConfig::tenant(test_admin_url(), name),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestDb {
    fn drop(&mut self) {
        // Drop can't be async, and the test's own runtime may already be shutting down, so the
        // cleanup gets a thread and a runtime of its own. Best effort: a tenant left behind by
        // a crash is harmless, since every test uses a new name.
        let config = self.config.clone();
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let _ = rt.block_on(destroy(&config));
        })
        .join();
    }
}

/// Test-only helper shared across this crate's modules: an isolated, fully-migrated pool in
/// its own tenant. Keep the returned [`TestDb`] alive while the pool is in use — dropping it
/// drops the tenant.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn test_pool() -> (PgPool, TestDb) {
    let db = TestDb::new().await;
    let pool = open(&db.config).await.expect("open test db");
    (pool, db)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn count_settings_in(pool: &PgPool, schema: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM \"{schema}\".settings"
        )))
        .fetch_one(pool)
        .await
    }

    fn is_permission_denied(e: &sqlx::Error) -> bool {
        matches!(e, sqlx::Error::Database(db) if db.code().as_deref() == Some("42501"))
    }

    #[test]
    fn names_must_be_plain_lowercase_identifiers() {
        assert!(validate_identifier("schema", "u_0192abc").is_ok());
        assert!(validate_identifier("schema", "_x").is_ok());
        assert!(validate_identifier("schema", "").is_err());
        assert!(validate_identifier("schema", "1abc").is_err());
        assert!(validate_identifier("schema", "Upper").is_err());
        assert!(validate_identifier("schema", "a\"; DROP SCHEMA public; --").is_err());
        assert!(validate_identifier("schema", &"a".repeat(64)).is_err());

        let mut config = DbConfig::tenant("postgres://x", "ok");
        config.role = Some("Bad Role".into());
        assert!(validate(&config).is_err());
    }

    #[tokio::test]
    async fn open_provisions_a_tenant_whose_tables_it_owns() {
        let (pool, db) = test_pool().await;
        let name = db.config.schema.clone();

        let (session_user, user, schema): (String, String, String) =
            sqlx::query_as("SELECT session_user::TEXT, current_user::TEXT, current_schema()::TEXT")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (session_user.as_str(), user.as_str(), schema.as_str()),
            (name.as_str(), name.as_str(), name.as_str())
        );

        let owners: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT tableowner::TEXT FROM pg_tables WHERE schemaname = current_schema()",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(owners, vec![(name.clone(),)]);
        assert_eq!(count_settings_in(&pool, &name).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn open_is_idempotent_even_when_concurrent() {
        let db = TestDb::new().await;
        let (a, b) = tokio::join!(open(&db.config), open(&db.config));
        a.expect("first open");
        let pool = b.expect("concurrent open");
        // Re-opening must not fail or re-apply migrations destructively.
        let pool2 = open(&db.config).await.expect("re-open");

        sqlx::query("INSERT INTO settings (key, value_json, updated_at) VALUES ('k', '1', 'now')")
            .execute(&pool)
            .await
            .expect("insert works after reopen");
        assert_eq!(
            count_settings_in(&pool2, &db.config.schema).await.unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn a_tenant_cannot_read_another_tenants_schema() {
        let (a, a_db) = test_pool().await;
        let (b, b_db) = test_pool().await;
        sqlx::query("INSERT INTO settings (key, value_json, updated_at) VALUES ('k', '1', 'now')")
            .execute(&a)
            .await
            .unwrap();

        // Unqualified names resolve to each tenant's own tables.
        assert_eq!(count_settings_in(&a, &a_db.config.schema).await.unwrap(), 1);
        assert_eq!(count_settings_in(&b, &b_db.config.schema).await.unwrap(), 0);
        // Naming the other tenant's schema is refused by Postgres, not just filtered.
        let err = count_settings_in(&b, &a_db.config.schema)
            .await
            .unwrap_err();
        assert!(is_permission_denied(&err), "{err:?}");
    }

    #[tokio::test]
    async fn the_login_role_itself_cannot_read_tenant_data() {
        let (_pool, db) = test_pool().await;
        let login_only = PgPoolOptions::new()
            .max_connections(1)
            .connect(&db.config.url)
            .await
            .unwrap();
        let err = count_settings_in(&login_only, &db.config.schema)
            .await
            .unwrap_err();
        assert!(is_permission_denied(&err), "{err:?}");
    }

    #[tokio::test]
    async fn reset_role_stays_the_tenant() {
        let (pool, db) = test_pool().await;
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("RESET ROLE").execute(&mut *conn).await.unwrap();
        let user: String = sqlx::query_scalar("SELECT current_user::TEXT")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(user, db.config.schema);
    }

    async fn current_user(conn: &mut PgConnection) -> String {
        sqlx::query_scalar("SELECT current_user::TEXT")
            .fetch_one(&mut *conn)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_tenant_cannot_switch_to_any_other_role() {
        let (a, a_db) = test_pool().await;
        let (_b, b_db) = test_pool().await;
        let mut conn = a.acquire().await.unwrap();

        sqlx::query("SET ROLE NONE")
            .execute(&mut *conn)
            .await
            .unwrap();
        assert_eq!(current_user(&mut conn).await, a_db.config.schema);

        for target in [TEST_ADMIN_ROLE, b_db.config.schema.as_str()] {
            for sql in [
                format!("SET ROLE \"{target}\""),
                format!("SET SESSION AUTHORIZATION \"{target}\""),
            ] {
                let err = sqlx::raw_sql(AssertSqlSafe(sql.clone()))
                    .execute(&mut *conn)
                    .await
                    .unwrap_err();
                assert!(is_permission_denied(&err), "{sql}: {err:?}");
            }
        }
        assert_eq!(current_user(&mut conn).await, a_db.config.schema);
    }

    /// The login role creates tenants but can't act as one outside `provision`/`destroy`.
    async fn assert_login_role_cannot_become(config: &DbConfig) {
        let mut conn = PgConnection::connect(&config.url).await.unwrap();
        let role = config.role.as_deref().unwrap();
        let err = sqlx::raw_sql(AssertSqlSafe(format!("SET ROLE \"{role}\"")))
            .execute(&mut conn)
            .await
            .unwrap_err();
        assert!(is_permission_denied(&err), "{err:?}");
    }

    #[tokio::test]
    async fn the_login_role_keeps_no_set_membership_in_a_tenant() {
        let (_pool, db) = test_pool().await;
        assert_login_role_cannot_become(&db.config).await;
    }

    #[tokio::test]
    async fn tenants_made_before_tenant_logins_are_upgraded_on_open() {
        let db = TestDb::new().await;
        let name = &db.config.schema;
        // How `provision` used to make a tenant: a NOLOGIN role the login role may SET into.
        let mut conn = PgConnection::connect(&db.config.url).await.unwrap();
        for sql in [
            format!("CREATE ROLE \"{name}\" NOLOGIN"),
            format!("GRANT \"{name}\" TO CURRENT_USER WITH INHERIT FALSE, SET TRUE"),
            format!("CREATE SCHEMA \"{name}\" AUTHORIZATION \"{name}\""),
        ] {
            exec(&mut conn, sql).await.unwrap();
        }
        conn.close().await.unwrap();

        let pool = open(&db.config).await.expect("open upgrades the tenant");
        let session_user: String = sqlx::query_scalar("SELECT session_user::TEXT")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(&session_user, name);
        assert_login_role_cannot_become(&db.config).await;
    }

    #[test]
    fn tenant_passwords_are_per_tenant_and_per_login() {
        let url = "postgres://app:secret@localhost/db";
        assert_eq!(tenant_password(url, "u_a"), tenant_password(url, "u_a"));
        assert_ne!(tenant_password(url, "u_a"), tenant_password(url, "u_b"));
        assert_ne!(
            tenant_password(url, "u_a"),
            tenant_password("postgres://app:other@localhost/db", "u_a")
        );
        assert_eq!(tenant_password(url, "u_a").len(), 64);

        let verifier = tenant_scram_verifier(url, "u_a");
        assert_eq!(verifier, tenant_scram_verifier(url, "u_a"));
        assert!(verifier.starts_with("SCRAM-SHA-256$4096:"), "{verifier}");
        assert!(!verifier.contains(&tenant_password(url, "u_a")));
        assert!(!verifier.contains('\''));
    }

    #[tokio::test]
    async fn destroy_removes_the_schema_and_the_role() {
        let db = TestDb::new().await;
        open(&db.config).await.unwrap();
        destroy(&db.config).await.unwrap();
        destroy(&db.config).await.expect("destroying twice is fine");

        let mut conn = PgConnection::connect(&db.config.url).await.unwrap();
        let (schemas, roles): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM pg_namespace WHERE nspname = $1),
                    (SELECT COUNT(*) FROM pg_roles WHERE rolname = $1)",
        )
        .bind(&db.config.schema)
        .fetch_one(&mut conn)
        .await
        .unwrap();
        assert_eq!((schemas, roles), (0, 0));
    }

    #[tokio::test]
    async fn single_user_mode_uses_the_login_role() {
        ensure_test_admin().await;
        let name = format!("test_{}", uuid::Uuid::now_v7().simple());
        let config = DbConfig {
            url: test_admin_url(),
            schema: name.clone(),
            role: None,
            max_connections: 1,
        };
        let pool = open(&config).await.unwrap();
        let user: String = sqlx::query_scalar("SELECT current_user::TEXT")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(user, TEST_ADMIN_ROLE);
        assert_eq!(count_settings_in(&pool, &name).await.unwrap(), 0);
        pool.close().await;
        destroy(&config).await.unwrap();
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
