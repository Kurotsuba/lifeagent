//! `DbUsageSink`: writes every `LlmRouter` call to the `llm_usage` table, per
//! `docs/PHASE1_SPEC.md` §5/§6.3.
//!
//! Needed as soon as an `LlmRouter` can be constructed at all — `LlmRouter::new` and
//! `with_providers` both require a `UsageSink` — so it lands in M3 (`CoreApp::build`) even
//! though `lib.rs`'s milestone note groups this module with the plan service and scheduler
//! (M5). The `llm_usage` table itself already exists from the M1 core migration.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lifeagent_llm::error::LlmError;
use lifeagent_llm::router::{UsageEvent, UsageSink};
use lifeagent_llm::types::Role;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;

pub struct DbUsageSink {
    db: PgPool,
    clock: Arc<dyn Clock>,
    /// Prompt + completion tokens allowed per rolling 24 hours (`CoreAppBuilder::daily_token_budget`).
    /// `None`: no limit.
    daily_token_budget: Option<u64>,
}

impl DbUsageSink {
    pub fn new(db: PgPool, clock: Arc<dyn Clock>) -> Self {
        Self {
            db,
            clock,
            daily_token_budget: None,
        }
    }

    pub fn with_daily_token_budget(mut self, budget: Option<u64>) -> Self {
        self.daily_token_budget = budget;
        self
    }
}

/// Prompt + completion tokens of every call recorded at or after `since`, all roles together.
pub async fn tokens_since(db: &PgPool, since: DateTime<Utc>) -> Result<i64, CoreError> {
    let total: Option<i64> = sqlx::query_scalar(
        "SELECT SUM(prompt_tokens + completion_tokens)::BIGINT FROM llm_usage WHERE created_at >= $1",
    )
    .bind(since.to_rfc3339())
    .fetch_one(db)
    .await?;
    Ok(total.unwrap_or(0))
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::Parser => "parser",
        Role::Planner => "planner",
        Role::Conversation => "conversation",
    }
}

fn parse_role(s: &str) -> Result<Role, CoreError> {
    match s {
        "parser" => Ok(Role::Parser),
        "planner" => Ok(Role::Planner),
        "conversation" => Ok(Role::Conversation),
        other => Err(CoreError::Validation(format!(
            "unknown llm_usage role in db: {other:?}"
        ))),
    }
}

/// Token usage for one LLM role over some window (§10.3's Settings usage display). A role with
/// no calls in that window doesn't appear in [`summary`]'s result at all.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RoleUsage {
    pub role: Role,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// Total prompt/completion tokens per role, for calls recorded at or after `since`. Used by
/// `CoreApp::usage_summary` with `since` = the start of the current calendar month (UTC).
pub async fn summary(db: &PgPool, since: DateTime<Utc>) -> Result<Vec<RoleUsage>, CoreError> {
    let rows: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT role, SUM(prompt_tokens)::BIGINT, SUM(completion_tokens)::BIGINT
         FROM llm_usage WHERE created_at >= $1 GROUP BY role",
    )
    .bind(since.to_rfc3339())
    .fetch_all(db)
    .await?;

    rows.into_iter()
        .map(|(role, prompt_tokens, completion_tokens)| {
            Ok(RoleUsage {
                role: parse_role(&role)?,
                prompt_tokens: prompt_tokens.unwrap_or(0),
                completion_tokens: completion_tokens.unwrap_or(0),
            })
        })
        .collect()
}

#[async_trait]
impl UsageSink for DbUsageSink {
    async fn record(&self, event: UsageEvent) {
        // The activity log's copy, for display; `llm_usage` stays the source of truth.
        crate::audit::record(
            crate::audit::StepKind::ModelCall,
            serde_json::json!({
                "role": role_str(event.role),
                "provider": event.provider_id,
                "model": event.model,
                "prompt_tokens": event.prompt_tokens,
                "completion_tokens": event.completion_tokens,
                "latency_ms": event.latency_ms,
                "ok": event.ok,
            }),
        )
        .await;
        let result = sqlx::query(
            "INSERT INTO llm_usage
                (id, role, provider_id, model, prompt_tokens, completion_tokens, latency_ms, ok, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(role_str(event.role))
        .bind(&event.provider_id)
        .bind(&event.model)
        .bind(i64::from(event.prompt_tokens))
        .bind(i64::from(event.completion_tokens))
        .bind(event.latency_ms as i64)
        .bind(i64::from(event.ok))
        .bind(self.clock.now().to_rfc3339())
        .execute(&self.db)
        .await;

        // A usage-logging failure must never break the LLM call it's recording; there is
        // nothing more useful to do with it than log it.
        if let Err(e) = result {
            tracing::error!(error = %e, "failed to record llm usage");
        }
    }

    /// Refuses the call once the last 24 hours' tokens reach the budget. The call that crosses
    /// it still completes (its size isn't known in advance); the next one is refused.
    async fn check_budget(&self, role: Role) -> Result<(), LlmError> {
        let Some(budget) = self.daily_token_budget else {
            return Ok(());
        };
        let since = self.clock.now() - chrono::Duration::hours(24);
        let used = match tokens_since(&self.db, since).await {
            Ok(used) => used,
            // Fail closed: without the count there's no way to tell the budget holds.
            Err(e) => {
                tracing::error!(error = %e, "failed to read llm usage for the token budget");
                return Err(LlmError::BudgetExceeded);
            }
        };
        if used >= i64::try_from(budget).unwrap_or(i64::MAX) {
            tracing::warn!(role = role_str(role), used, budget, "token budget exceeded");
            crate::audit::record(
                crate::audit::StepKind::Guardrail,
                serde_json::json!({
                    "rule": "token_budget",
                    "role": role_str(role),
                    "used": used,
                    "budget": budget,
                }),
            )
            .await;
            return Err(LlmError::BudgetExceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[tokio::test]
    async fn record_inserts_a_row() {
        let (db, _dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        let sink = DbUsageSink::new(db.clone(), clock);

        sink.record(UsageEvent {
            role: Role::Parser,
            provider_id: "p1".into(),
            model: "model-x".into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            latency_ms: 42,
            ok: true,
        })
        .await;

        let row: (String, String, i64, i64, i64) = sqlx::query_as(
            "SELECT role, model, prompt_tokens, completion_tokens, ok FROM llm_usage",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(row.0, "parser");
        assert_eq!(row.1, "model-x");
        assert_eq!(row.2, 10);
        assert_eq!(row.3, 5);
        assert_eq!(row.4, 1);
    }

    fn event(role: Role, prompt_tokens: u32, completion_tokens: u32) -> UsageEvent {
        UsageEvent {
            role,
            provider_id: "p1".into(),
            model: "model-x".into(),
            prompt_tokens,
            completion_tokens,
            latency_ms: 1,
            ok: true,
        }
    }

    #[tokio::test]
    async fn summary_sums_tokens_per_role_since_the_given_time() {
        let (db, _dir) = crate::db::test_pool().await;
        let before = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let since = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 9, 15, 0, 0, 0).unwrap();

        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(before)))
            .record(event(Role::Parser, 100, 100))
            .await;
        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(after)))
            .record(event(Role::Parser, 10, 5))
            .await;
        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(after)))
            .record(event(Role::Parser, 20, 8))
            .await;
        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(after)))
            .record(event(Role::Conversation, 7, 3))
            .await;

        let mut result = summary(&db, since).await.unwrap();
        result.sort_by_key(|r| role_str(r.role));

        assert_eq!(
            result,
            vec![
                RoleUsage {
                    role: Role::Conversation,
                    prompt_tokens: 7,
                    completion_tokens: 3,
                },
                RoleUsage {
                    role: Role::Parser,
                    prompt_tokens: 30,
                    completion_tokens: 13,
                },
            ]
        );
    }

    #[tokio::test]
    async fn summary_is_empty_when_no_calls_fall_in_the_window() {
        let (db, _dir) = crate::db::test_pool().await;
        let earlier = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(earlier)))
            .record(event(Role::Parser, 10, 5))
            .await;

        let since = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        assert_eq!(summary(&db, since).await.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn summary_omits_roles_with_no_calls_in_the_window() {
        let (db, _dir) = crate::db::test_pool().await;
        let since = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(since)))
            .record(event(Role::Planner, 10, 5))
            .await;

        let result = summary(&db, since).await.unwrap();

        assert_eq!(
            result,
            vec![RoleUsage {
                role: Role::Planner,
                prompt_tokens: 10,
                completion_tokens: 5,
            }]
        );
    }

    #[tokio::test]
    async fn the_budget_counts_the_last_24_hours_of_all_roles() {
        let (db, _dir) = crate::db::test_pool().await;
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        let day_ago = now - chrono::Duration::hours(25);
        DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(day_ago)))
            .record(event(Role::Planner, 1000, 1000))
            .await;
        let sink = DbUsageSink::new(db.clone(), Arc::new(crate::clock::FixedClock::new(now)))
            .with_daily_token_budget(Some(100));

        // Older than 24 hours: not counted.
        assert!(sink.check_budget(Role::Conversation).await.is_ok());
        sink.record(event(Role::Parser, 60, 10)).await;
        assert!(sink.check_budget(Role::Conversation).await.is_ok());
        sink.record(event(Role::Conversation, 20, 10)).await;
        assert!(matches!(
            sink.check_budget(Role::Parser).await,
            Err(LlmError::BudgetExceeded)
        ));
    }

    #[tokio::test]
    async fn without_a_budget_nothing_is_refused() {
        let (db, _dir) = crate::db::test_pool().await;
        let sink = DbUsageSink::new(
            db.clone(),
            Arc::new(crate::clock::FixedClock::new(Utc::now())),
        );
        sink.record(event(Role::Planner, 1_000_000, 1_000_000))
            .await;
        assert!(sink.check_budget(Role::Planner).await.is_ok());
    }
}
