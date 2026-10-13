//! Proactive follow-ups (`docs/DECISIONS.md`, "Proactive follow-ups"): check-ins the scheduler
//! raises on its own, without the user asking.
//!
//! Each tick, every enabled plugin's [`crate::plugin::Plugin::follow_ups`] check says what is
//! worth asking about right now (a planned meal not logged by its check time, a day far off its
//! targets). Whether to ask is decided by those checks in Rust, never by a model, so it is
//! deterministic and costs no tokens. New ones are stored here and shown in the app with
//! one-tap answers; the user's answer is what acts. Limits: one per key per logical day, at most
//! [`MAX_PER_DAY`] a day, and a switch in Settings. Raising and answering are both recorded in
//! the activity log.
//!
//! Answers are kinds, worded by the UI: `core_dismiss` (not now), `core_reply` (the UI focuses
//! the chat; nothing is sent here), `core_adjust_tomorrow` (the planner revises tomorrow's plan
//! with the follow-up's instruction), or a plugin's own kind, prefixed with its id, which the
//! plugin carries out in [`crate::plugin::Plugin::answer_follow_up`].

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::plugin::ToolSummary;

/// Follow-ups raised per logical day at most, whatever their kind.
pub const MAX_PER_DAY: usize = 4;

/// Not now: closes the follow-up without doing anything.
pub const ANSWER_DISMISS: &str = "core_dismiss";
/// Reply in chat: the UI focuses the chat input. Never sent to the server.
pub const ANSWER_REPLY: &str = "core_reply";
/// Revise tomorrow's plan (or generate it, if there is none yet) with the follow-up's
/// `instruction`.
pub const ANSWER_ADJUST_TOMORROW: &str = "core_adjust_tomorrow";

/// What a plugin's check found worth asking about now.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowUpCandidate {
    /// Unique per logical day and prefixed with the plugin id, e.g. `diet_meal_missed:lunch`.
    pub key: String,
    /// What the UI words, e.g. `diet_meal_missed`.
    pub kind: String,
    /// Data the UI words it with. Numbers are the ones Rust computed.
    pub params: Value,
    /// Answer kinds offered, in display order.
    pub answers: Vec<String>,
    /// Why it was raised, in English, for the activity log.
    pub reason: String,
    /// The planner request for [`ANSWER_ADJUST_TOMORROW`].
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpStatus {
    Open,
    /// The user picked an answer that did something.
    Answered,
    /// The user said not now.
    Dismissed,
    /// The reason went away before the user answered (e.g. the meal was logged in chat).
    Cleared,
}

impl FollowUpStatus {
    fn as_str(self) -> &'static str {
        match self {
            FollowUpStatus::Open => "open",
            FollowUpStatus::Answered => "answered",
            FollowUpStatus::Dismissed => "dismissed",
            FollowUpStatus::Cleared => "cleared",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "open" => Ok(FollowUpStatus::Open),
            "answered" => Ok(FollowUpStatus::Answered),
            "dismissed" => Ok(FollowUpStatus::Dismissed),
            "cleared" => Ok(FollowUpStatus::Cleared),
            other => Err(CoreError::Validation(format!(
                "bad follow-up status in db: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FollowUp {
    pub id: String,
    pub logical_date: NaiveDate,
    pub key: String,
    pub kind: String,
    pub params: Value,
    pub answers: Vec<String>,
    pub reason: String,
    /// Kept server-side: the UI never needs the planner's wording.
    #[serde(skip)]
    pub instruction: Option<String>,
    pub status: FollowUpStatus,
    pub answer: Option<String>,
    pub result: Option<ToolSummary>,
    pub run_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl FollowUp {
    /// The plugin that owns this follow-up: the key's prefix before the first `_`.
    pub fn plugin_id(&self) -> &str {
        self.key.split('_').next().unwrap_or_default()
    }
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

const COLUMNS: &str =
    "id, logical_date, key, kind, params_json, actions_json, reason, instruction, \
                       status, answer, result_json, run_id, created_at, resolved_at";

#[derive(Clone)]
pub struct FollowUpStore {
    db: PgPool,
    clock: Arc<dyn Clock>,
}

impl FollowUpStore {
    pub fn new(db: PgPool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    /// Stores a new open follow-up for `date`, unless one with the same key already exists that
    /// day (whatever its status): `None` then. The current activity-log run, if any, is noted.
    pub async fn raise(
        &self,
        date: NaiveDate,
        candidate: &FollowUpCandidate,
    ) -> Result<Option<FollowUp>, CoreError> {
        let id = Uuid::now_v7().to_string();
        let created_at = self.clock.now();
        let run_id = crate::audit::current().map(|run| run.id().to_string());
        let inserted = sqlx::query(
            "INSERT INTO follow_ups
                (id, logical_date, key, kind, params_json, actions_json, reason, instruction,
                 status, run_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'open', $9, $10)
             ON CONFLICT (logical_date, key) DO NOTHING",
        )
        .bind(&id)
        .bind(date.to_string())
        .bind(&candidate.key)
        .bind(&candidate.kind)
        .bind(candidate.params.to_string())
        .bind(serde_json::to_string(&candidate.answers)?)
        .bind(&candidate.reason)
        .bind(&candidate.instruction)
        .bind(&run_id)
        .bind(created_at.to_rfc3339())
        .execute(&self.db)
        .await?;
        if inserted.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(FollowUp {
            id,
            logical_date: date,
            key: candidate.key.clone(),
            kind: candidate.kind.clone(),
            params: candidate.params.clone(),
            answers: candidate.answers.clone(),
            reason: candidate.reason.clone(),
            instruction: candidate.instruction.clone(),
            status: FollowUpStatus::Open,
            answer: None,
            result: None,
            run_id,
            created_at,
            resolved_at: None,
        }))
    }

    pub async fn get(&self, id: &str) -> Result<Option<FollowUp>, CoreError> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM follow_ups WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        row.map(from_row).transpose()
    }

    /// Every follow-up raised for `date`, oldest first, whatever its status.
    pub async fn on(&self, date: NaiveDate) -> Result<Vec<FollowUp>, CoreError> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM follow_ups WHERE logical_date = $1 ORDER BY id"
        )))
        .bind(date.to_string())
        .fetch_all(&self.db)
        .await?;
        rows.into_iter().map(from_row).collect()
    }

    /// Refreshes an open follow-up's params and instruction with a newer check's (e.g. the
    /// day's totals moved since it was raised).
    pub async fn refresh(&self, id: &str, candidate: &FollowUpCandidate) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE follow_ups SET params_json = $1, instruction = $2
             WHERE id = $3 AND status = 'open'",
        )
        .bind(candidate.params.to_string())
        .bind(&candidate.instruction)
        .bind(id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Closes an open follow-up. Errors `ActionNotPending` if it was closed meanwhile (answered
    /// from another tab, or cleared by a tick).
    pub async fn close(
        &self,
        id: &str,
        status: FollowUpStatus,
        answer: Option<&str>,
        result: Option<&ToolSummary>,
    ) -> Result<FollowUp, CoreError> {
        let updated = sqlx::query(
            "UPDATE follow_ups SET status = $1, answer = $2, result_json = $3, resolved_at = $4
             WHERE id = $5 AND status = 'open'",
        )
        .bind(status.as_str())
        .bind(answer)
        .bind(result.map(|r| r.0.to_string()))
        .bind(self.clock.now().to_rfc3339())
        .bind(id)
        .execute(&self.db)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(CoreError::ActionNotPending);
        }
        self.get(id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("follow-up {id}")))
    }
}

fn from_row(row: Row) -> Result<FollowUp, CoreError> {
    let (
        id,
        logical_date,
        key,
        kind,
        params,
        answers,
        reason,
        instruction,
        status,
        answer,
        result,
        run_id,
        created_at,
        resolved_at,
    ) = row;
    let instant = |s: &str| -> Result<DateTime<Utc>, CoreError> {
        DateTime::parse_from_rfc3339(s)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|e| CoreError::Validation(format!("bad timestamp in follow_ups: {e}")))
    };
    Ok(FollowUp {
        id,
        logical_date: NaiveDate::parse_from_str(&logical_date, "%Y-%m-%d")
            .map_err(|e| CoreError::Validation(format!("bad logical_date in follow_ups: {e}")))?,
        key,
        kind,
        params: serde_json::from_str(&params)?,
        answers: serde_json::from_str(&answers)?,
        reason,
        instruction,
        status: FollowUpStatus::parse(&status)?,
        answer,
        result: result
            .map(|r| serde_json::from_str(&r).map(ToolSummary))
            .transpose()?,
        run_id,
        created_at: instant(&created_at)?,
        resolved_at: resolved_at.as_deref().map(instant).transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use serde_json::json;

    fn candidate(key: &str) -> FollowUpCandidate {
        FollowUpCandidate {
            key: key.into(),
            kind: "test_thing".into(),
            params: json!({"n": 1}),
            answers: vec!["test_do".into(), ANSWER_DISMISS.into()],
            reason: "because".into(),
            instruction: Some("do it".into()),
        }
    }

    #[tokio::test]
    async fn raises_once_per_key_and_day_and_closes_once() {
        let (db, _dir) = crate::db::test_pool().await;
        let store = FollowUpStore::new(db, Arc::new(FixedClock::new(Utc::now())));
        let day = NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();

        let raised = store
            .raise(day, &candidate("test_a"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(raised.status, FollowUpStatus::Open);
        assert_eq!(raised.plugin_id(), "test");
        assert!(store
            .raise(day, &candidate("test_a"))
            .await
            .unwrap()
            .is_none());
        // The same key on another day is a new follow-up.
        assert!(store
            .raise(day.succ_opt().unwrap(), &candidate("test_a"))
            .await
            .unwrap()
            .is_some());

        let closed = store
            .close(
                &raised.id,
                FollowUpStatus::Answered,
                Some("test_do"),
                Some(&ToolSummary::new("test_done")),
            )
            .await
            .unwrap();
        assert_eq!(closed.status, FollowUpStatus::Answered);
        assert_eq!(closed.answer.as_deref(), Some("test_do"));
        assert_eq!(closed.result.unwrap().kind(), "test_done");
        assert_eq!(closed.instruction.as_deref(), Some("do it"));
        assert!(matches!(
            store
                .close(&raised.id, FollowUpStatus::Dismissed, None, None)
                .await,
            Err(CoreError::ActionNotPending)
        ));
        assert_eq!(store.on(day).await.unwrap().len(), 1);
    }
}
