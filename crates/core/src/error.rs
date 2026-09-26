//! `CoreError`: the error type shared by every service in this crate.
//!
//! Filled in during M1 per `docs/PHASE1_SPEC.md` §7, plus the `Llm` variant added in M3 for
//! the orchestrator's calls into `lifeagent-llm`.

use chrono::{NaiveDate, NaiveTime};
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

    /// Free-form validation/format failures: internal invariants (a malformed value read back
    /// from the db, a startup-time tool-registration bug) rather than a specific rule a user's
    /// own input broke. A shell maps every one of these to the same generic zh-CN message.
    #[error("validation failed: {0}")]
    Validation(String),

    /// A named validation rule broken by input a user actually typed into a form or chat message
    /// (profile, goal, memory note, settings, weight, plan revision). Distinct from `Validation`
    /// so a shell (M7's `CommandError`) can give each rule its own zh-CN wording instead of one
    /// generic message for all of them (§14 M8: "Error messages in zh-CN for every
    /// `CommandError`").
    #[error("invalid input: {0}")]
    InvalidInput(ValidationRule),

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

    /// No LLM provider is configured yet (§7.2: "none (onboarding required)"). Distinct from
    /// `Validation` so a shell (M7's `CommandError`) can map it to its own `not_configured`
    /// code without matching on message text.
    #[error("models are not configured yet")]
    NotConfigured,
}

impl CoreError {
    /// A stable, short classification code, independent of any UI's wording. `CommandError`'s
    /// zh-CN mapping (`apps/desktop/src-tauri/src/error.rs`) uses it for `CommandError.code`,
    /// and the scheduler (`scheduler.rs`) attaches it to `AppEvent::PlanFailed` so a *scheduled*
    /// failure's cause survives past the event boundary without carrying English detail — one
    /// source of truth instead of two independent classifications that could drift apart.
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::NotConfigured => "not_configured",
            CoreError::Validation(_) | CoreError::InvalidInput(_) | CoreError::NothingToPlan => {
                "invalid_input"
            }
            CoreError::NotFound(_) => "not_found",
            CoreError::Llm(e) => e.code(),
            CoreError::Db(_) | CoreError::Migrate(_) | CoreError::Json(_) | CoreError::Io(_) => {
                "internal"
            }
            CoreError::Secret(_) | CoreError::Plugin(_) => "internal",
        }
    }
}

/// One specific validation rule, named so a shell can look it up and give it its own zh-CN
/// message (§14 M8) instead of matching on `Validation`'s free-form English text. Every variant
/// here is `Copy`: it carries only the offending value(s) and the bound, never owned strings,
/// so building one never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ValidationRule {
    #[error("height_cm must be within 120..=230, got {got}")]
    HeightOutOfRange { got: f64 },

    #[error("age must be within 16..=100, got {got} (birth_date {birth_date})")]
    AgeOutOfRange { got: i32, birth_date: NaiveDate },

    #[error("meals_per_day must be within 3..=5, got {got}")]
    MealsPerDayOutOfRange { got: u8 },

    #[error("rate_kg_per_week must be within 0.0..=1.0 for a non-maintain goal, got {got}")]
    GoalRateOutOfRange { got: f64 },

    #[error("target_weight_kg must be within 30.0..=300.0, got {got}")]
    GoalTargetOutOfRange { got: f64 },

    #[error("weight kg must be within 30..=300, got {got}")]
    WeightOutOfRange { got: f64 },

    #[error("note must not be empty")]
    NoteEmpty,

    #[error("note must be at most {max} characters, got {got}")]
    NoteTooLong { got: usize, max: usize },

    #[error("plan_time {plan_time} must not be earlier than day_boundary {day_boundary}")]
    PlanTimeBeforeDayBoundary {
        plan_time: NaiveTime,
        day_boundary: NaiveTime,
    },

    #[error("instruction must not be empty")]
    InstructionEmpty,
}
