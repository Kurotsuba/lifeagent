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
use lifeagent_llm::router::{UsageEvent, UsageSink};
use lifeagent_llm::types::Role;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;

pub struct DbUsageSink {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
}

impl DbUsageSink {
    pub fn new(db: SqlitePool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }
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
pub async fn summary(db: &SqlitePool, since: DateTime<Utc>) -> Result<Vec<RoleUsage>, CoreError> {
    let rows: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT role, SUM(prompt_tokens), SUM(completion_tokens)
         FROM llm_usage WHERE created_at >= ? GROUP BY role",
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
        let result = sqlx::query(
            "INSERT INTO llm_usage
                (id, role, provider_id, model, prompt_tokens, completion_tokens, latency_ms, ok, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(role_str(event.role))
        .bind(&event.provider_id)
        .bind(&event.model)
        .bind(event.prompt_tokens)
        .bind(event.completion_tokens)
        .bind(event.latency_ms as i64)
        .bind(event.ok)
        .bind(self.clock.now().to_rfc3339())
        .execute(&self.db)
        .await;

        // A usage-logging failure must never break the LLM call it's recording; there is
        // nothing more useful to do with it than log it.
        if let Err(e) = result {
            tracing::error!(error = %e, "failed to record llm usage");
        }
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
}
