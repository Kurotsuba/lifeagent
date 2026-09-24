//! Plan types and storage, per `docs/PHASE1_SPEC.md` §7.12.
//!
//! M4 adds the types and read/insert access, so the diet plugin's plan-vs-actual and context
//! snippet and `CoreApp::today_view` can read plans. Generation, revision and acceptance are
//! M5 and build on `PlanStore::insert`.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Proposed,
    Accepted,
}

impl PlanStatus {
    fn as_str(&self) -> &'static str {
        match self {
            PlanStatus::Proposed => "proposed",
            PlanStatus::Accepted => "accepted",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "proposed" => Ok(PlanStatus::Proposed),
            "accepted" => Ok(PlanStatus::Accepted),
            other => Err(CoreError::Validation(format!(
                "unknown plan status {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanTrigger {
    Schedule,
    Manual,
    Revision,
}

impl PlanTrigger {
    fn as_str(&self) -> &'static str {
        match self {
            PlanTrigger::Schedule => "schedule",
            PlanTrigger::Manual => "manual",
            PlanTrigger::Revision => "revision",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "schedule" => Ok(PlanTrigger::Schedule),
            "manual" => Ok(PlanTrigger::Manual),
            "revision" => Ok(PlanTrigger::Revision),
            other => Err(CoreError::Validation(format!(
                "unknown plan trigger {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanContent {
    /// Short message to the user.
    pub summary: String,
    /// Plugin id -> that plugin's finalized section data.
    pub sections: BTreeMap<String, Value>,
    pub disclaimer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPlan {
    pub plan_date: NaiveDate,
    pub version: u32,
    pub status: PlanStatus,
    pub trigger: PlanTrigger,
    /// The user's change request, for `revision` plans.
    pub instruction: Option<String>,
    pub content: PlanContent,
    pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct PlanStore {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
}

/// (plan_date, version, status, trigger, instruction, content_json, warnings_json, created_at)
type PlanRow = (
    String,
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
);

impl PlanStore {
    pub fn new(db: SqlitePool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    /// The highest version stored for `date`, if any.
    pub async fn current(&self, date: NaiveDate) -> Result<Option<StoredPlan>, CoreError> {
        let row: Option<PlanRow> = sqlx::query_as(
            "SELECT plan_date, version, status, trigger, instruction, content_json, warnings_json,
                    created_at
             FROM plans WHERE plan_date = ? ORDER BY version DESC LIMIT 1",
        )
        .bind(date.to_string())
        .fetch_optional(&self.db)
        .await?;
        row.map(Self::row_to_plan).transpose()
    }

    /// Stores a new `proposed` plan as version = previous max + 1 (1 for the first).
    pub async fn insert(
        &self,
        date: NaiveDate,
        trigger: PlanTrigger,
        instruction: Option<&str>,
        content: &PlanContent,
        warnings: &[String],
    ) -> Result<StoredPlan, CoreError> {
        let mut tx = self.db.begin().await?;
        let max: Option<i64> =
            sqlx::query_scalar("SELECT MAX(version) FROM plans WHERE plan_date = ?")
                .bind(date.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let version = max.unwrap_or(0) + 1;
        let created_at = self.clock.now();
        let status = PlanStatus::Proposed;

        sqlx::query(
            "INSERT INTO plans
                (plan_date, version, status, trigger, instruction, content_json, warnings_json,
                 created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(date.to_string())
        .bind(version)
        .bind(status.as_str())
        .bind(trigger.as_str())
        .bind(instruction)
        .bind(serde_json::to_string(content)?)
        .bind(serde_json::to_string(warnings)?)
        .bind(created_at.to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(StoredPlan {
            plan_date: date,
            version: version as u32,
            status,
            trigger,
            instruction: instruction.map(str::to_string),
            content: content.clone(),
            warnings: warnings.to_vec(),
            created_at,
        })
    }

    fn row_to_plan(row: PlanRow) -> Result<StoredPlan, CoreError> {
        let (plan_date, version, status, trigger, instruction, content, warnings, created_at) = row;
        Ok(StoredPlan {
            plan_date: plan_date
                .parse()
                .map_err(|e| CoreError::Validation(format!("bad plan_date in db: {e}")))?,
            version: u32::try_from(version)
                .map_err(|e| CoreError::Validation(format!("bad plan version in db: {e}")))?,
            status: PlanStatus::parse(&status)?,
            trigger: PlanTrigger::parse(&trigger)?,
            instruction,
            content: serde_json::from_str(&content)?,
            warnings: serde_json::from_str(&warnings)?,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .map_err(|e| CoreError::Validation(format!("bad created_at in db: {e}")))?
                .with_timezone(&Utc),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(summary: &str) -> PlanContent {
        PlanContent {
            summary: summary.into(),
            sections: BTreeMap::from([("diet".to_string(), serde_json::json!({"meals": []}))]),
            disclaimer: None,
        }
    }

    async fn store() -> (PlanStore, tempfile::TempDir) {
        let (db, dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        (PlanStore::new(db, clock), dir)
    }

    const DAY: NaiveDate = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();

    #[tokio::test]
    async fn current_is_none_when_no_plan_exists() {
        let (store, _dir) = store().await;
        assert_eq!(store.current(DAY).await.unwrap(), None);
    }

    #[tokio::test]
    async fn insert_then_current_round_trips() {
        let (store, _dir) = store().await;
        let inserted = store
            .insert(
                DAY,
                PlanTrigger::Manual,
                None,
                &content("v1"),
                &["w".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(inserted.version, 1);
        assert_eq!(inserted.status, PlanStatus::Proposed);
        assert_eq!(store.current(DAY).await.unwrap(), Some(inserted));
    }

    #[tokio::test]
    async fn versions_increment_per_date_and_current_is_the_latest() {
        let (store, _dir) = store().await;
        store
            .insert(DAY, PlanTrigger::Schedule, None, &content("v1"), &[])
            .await
            .unwrap();
        let v2 = store
            .insert(
                DAY,
                PlanTrigger::Revision,
                Some("午饭换成沙拉"),
                &content("v2"),
                &[],
            )
            .await
            .unwrap();
        let other_day = store
            .insert(
                DAY.succ_opt().unwrap(),
                PlanTrigger::Manual,
                None,
                &content("x"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(v2.version, 2);
        assert_eq!(other_day.version, 1);
        let current = store.current(DAY).await.unwrap().unwrap();
        assert_eq!(current, v2);
        assert_eq!(current.instruction.as_deref(), Some("午饭换成沙拉"));
    }
}
