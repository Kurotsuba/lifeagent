//! `CoreError`: the error type shared by every service in this crate.
//!
//! Filled in during M1 per `docs/PHASE1_SPEC.md` §7. Grows a variant per external failure
//! mode as later milestones add services (e.g. an `Llm` variant once the orchestrator, M3,
//! starts calling `lifeagent-llm`).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("database migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("invalid stored data: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("secret store error: {0}")]
    Secret(String),
}
