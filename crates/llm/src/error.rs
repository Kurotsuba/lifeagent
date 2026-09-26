//! `LlmError`: the error type returned by `ChatModel::complete` and the router.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.1 (`Http`, `Status`, `RateLimited`,
//! `Timeout`, `InvalidResponse`, `Auth`, `Config`).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http request failed: {0}")]
    Http(String),

    #[error("http status {code}: {body_excerpt}")]
    Status { code: u16, body_excerpt: String },

    #[error("rate limited")]
    RateLimited,

    #[error("request timed out")]
    Timeout,

    #[error("invalid response from provider: {0}")]
    InvalidResponse(String),

    #[error("authentication failed")]
    Auth,

    #[error("configuration error: {0}")]
    Config(String),
}

impl LlmError {
    /// A stable, short classification code, independent of any UI's wording — shared by
    /// `CommandError`'s zh-CN mapping (`apps/desktop/src-tauri/src/error.rs`) and the
    /// scheduler's `AppEvent::PlanFailed` (`crates/core/src/scheduler.rs`), so the two can never
    /// drift apart on what counts as, say, an auth failure.
    pub fn code(&self) -> &'static str {
        match self {
            LlmError::Auth => "auth",
            LlmError::Config(_) => "not_configured",
            LlmError::RateLimited | LlmError::Timeout => "llm",
            LlmError::Http(_) | LlmError::Status { .. } | LlmError::InvalidResponse(_) => "llm",
        }
    }
}
