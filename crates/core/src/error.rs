//! `CoreError`: the error type shared by every service in this crate.
//!
//! Filled in during M1 per `docs/PHASE1_SPEC.md` §7, plus the `Llm` variant added in M3 for
//! the orchestrator's calls into `lifeagent-llm`.

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

    #[error("LLM error: {0}")]
    Llm(#[from] lifeagent_llm::error::LlmError),

    /// A plugin hook called by core (context snippet, today card, ...) failed.
    #[error("plugin error: {0}")]
    Plugin(String),

    /// No plugin produced a plan input for the date (§7.12 step 1), e.g. because the profile
    /// or a weight is missing.
    #[error("nothing to plan: complete the profile and log a weight first")]
    NothingToPlan,
}
