//! `DbUsageSink`: writes every `LlmRouter` call to the `llm_usage` table, per
//! `docs/PHASE1_SPEC.md` §5/§6.3.
//!
//! Needed as soon as an `LlmRouter` can be constructed at all — `LlmRouter::new` and
//! `with_providers` both require a `UsageSink` — so it lands in M3 (`CoreApp::build`) even
//! though `lib.rs`'s milestone note groups this module with the plan service and scheduler
//! (M5). The `llm_usage` table itself already exists from the M1 core migration.

use std::sync::Arc;

use async_trait::async_trait;
use lifeagent_llm::router::{UsageEvent, UsageSink};
use lifeagent_llm::types::Role;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::clock::Clock;

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
    use chrono::Utc;

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
}
