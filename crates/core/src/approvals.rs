//! Tool calls that wait for the user's approval (`docs/DECISIONS.md`, "Approval for sensitive
//! tool calls").
//!
//! A tool whose owner returns a preview from `approval_preview` (core: `core_tools`, plugins:
//! `Plugin::approval_preview`) is not run when the conversation model calls it. The call is
//! stored here as a pending action, the model is told it awaits the user, and the app shows
//! the preview with approve/reject buttons. Approving runs the stored call exactly as the model
//! made it; nothing the model says afterwards can approve it. A pending action expires after
//! [`EXPIRY_HOURS`].

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;

/// A pending action older than this can no longer be approved.
pub const EXPIRY_HOURS: i64 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Pending,
    Approved,
    Rejected,
    /// Approved, but the tool refused or failed when it ran.
    Failed,
    /// Never resolved within [`EXPIRY_HOURS`]. Not stored: derived when read.
    Expired,
}

impl ActionStatus {
    fn as_str(self) -> &'static str {
        match self {
            ActionStatus::Pending => "pending",
            ActionStatus::Approved => "approved",
            ActionStatus::Rejected => "rejected",
            ActionStatus::Failed => "failed",
            ActionStatus::Expired => "expired",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "pending" => Ok(ActionStatus::Pending),
            "approved" => Ok(ActionStatus::Approved),
            "rejected" => Ok(ActionStatus::Rejected),
            "failed" => Ok(ActionStatus::Failed),
            other => Err(CoreError::Validation(format!(
                "bad pending action status in db: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingAction {
    pub id: String,
    pub tool_name: String,
    pub arguments: Value,
    /// `{"kind": ..., ...}`: what the action will do, as data the UI words in its language.
    pub preview: Value,
    pub status: ActionStatus,
    pub result_summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
);

const COLUMNS: &str = "id, tool_name, arguments_json, preview_json, status, result_summary, \
                       created_at, resolved_at";

#[derive(Clone)]
pub struct ApprovalStore {
    db: PgPool,
    clock: Arc<dyn Clock>,
}

impl ApprovalStore {
    pub fn new(db: PgPool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    pub async fn create(
        &self,
        tool_name: &str,
        arguments: &Value,
        preview: &Value,
    ) -> Result<PendingAction, CoreError> {
        let id = Uuid::now_v7().to_string();
        let created_at = self.clock.now();
        sqlx::query(
            "INSERT INTO pending_actions
                (id, tool_name, arguments_json, preview_json, status, created_at)
             VALUES ($1, $2, $3, $4, 'pending', $5)",
        )
        .bind(&id)
        .bind(tool_name)
        .bind(arguments.to_string())
        .bind(preview.to_string())
        .bind(created_at.to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(PendingAction {
            id,
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            preview: preview.clone(),
            status: ActionStatus::Pending,
            result_summary: None,
            created_at,
            resolved_at: None,
        })
    }

    pub async fn get(&self, id: &str) -> Result<Option<PendingAction>, CoreError> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM pending_actions WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        row.map(|r| self.action_from_row(r)).transpose()
    }

    /// Actions created in the last [`EXPIRY_HOURS`], oldest first, whatever their status.
    /// The conversation model sees these, so it knows what the user approved or rejected.
    pub async fn recent(&self) -> Result<Vec<PendingAction>, CoreError> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM pending_actions WHERE created_at >= $1 ORDER BY created_at, id"
        )))
        .bind(self.expiry_cutoff().to_rfc3339())
        .fetch_all(&self.db)
        .await?;
        rows.into_iter().map(|r| self.action_from_row(r)).collect()
    }

    /// Actions still awaiting the user, oldest first.
    pub async fn pending(&self) -> Result<Vec<PendingAction>, CoreError> {
        Ok(self
            .recent()
            .await?
            .into_iter()
            .filter(|a| a.status == ActionStatus::Pending)
            .collect())
    }

    /// Records the outcome of a pending action. Errors `ActionNotPending` if it was resolved
    /// meanwhile (e.g. approved twice from two tabs) or has expired.
    pub async fn resolve(
        &self,
        id: &str,
        status: ActionStatus,
        result_summary: Option<&str>,
    ) -> Result<PendingAction, CoreError> {
        let now = self.clock.now();
        let result = sqlx::query(
            "UPDATE pending_actions SET status = $1, result_summary = $2, resolved_at = $3
             WHERE id = $4 AND status = 'pending' AND created_at >= $5",
        )
        .bind(status.as_str())
        .bind(result_summary)
        .bind(now.to_rfc3339())
        .bind(id)
        .bind(self.expiry_cutoff().to_rfc3339())
        .execute(&self.db)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CoreError::ActionNotPending);
        }
        self.get(id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("pending action {id}")))
    }

    /// The action, if it still awaits the user; `ActionNotPending` if it was resolved or has
    /// expired. `CoreApp::approve_action` checks this and resolves it under the app's turn
    /// lock, so one action never runs twice.
    pub async fn require_pending(&self, id: &str) -> Result<PendingAction, CoreError> {
        let Some(action) = self.get(id).await? else {
            return Err(CoreError::NotFound(format!("pending action {id}")));
        };
        if action.status != ActionStatus::Pending {
            return Err(CoreError::ActionNotPending);
        }
        Ok(action)
    }

    fn expiry_cutoff(&self) -> DateTime<Utc> {
        self.clock.now() - Duration::hours(EXPIRY_HOURS)
    }

    fn action_from_row(&self, row: Row) -> Result<PendingAction, CoreError> {
        let (id, tool_name, arguments, preview, status, result_summary, created_at, resolved_at) =
            row;
        let created_at = parse_instant(&created_at)?;
        let mut status = ActionStatus::parse(&status)?;
        if status == ActionStatus::Pending && created_at < self.expiry_cutoff() {
            status = ActionStatus::Expired;
        }
        Ok(PendingAction {
            id,
            tool_name,
            arguments: serde_json::from_str(&arguments)?,
            preview: serde_json::from_str(&preview)?,
            status,
            result_summary,
            created_at,
            resolved_at: resolved_at.as_deref().map(parse_instant).transpose()?,
        })
    }
}

fn parse_instant(s: &str) -> Result<DateTime<Utc>, CoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| CoreError::Validation(format!("bad instant in db {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use serde_json::json;

    async fn store() -> (ApprovalStore, Arc<FixedClock>, crate::db::TestDb) {
        let (db, dir) = crate::db::test_pool().await;
        let clock = Arc::new(FixedClock::new(Utc::now()));
        (ApprovalStore::new(db, clock.clone()), clock, dir)
    }

    #[tokio::test]
    async fn an_action_is_resolved_once() {
        let (store, _clock, _dir) = store().await;
        let action = store
            .create(
                "core_update_allergies",
                &json!({"remove": ["peanut"]}),
                &json!({"kind": "update_allergies"}),
            )
            .await
            .unwrap();
        assert_eq!(store.pending().await.unwrap(), vec![action.clone()]);

        let resolved = store
            .resolve(&action.id, ActionStatus::Rejected, None)
            .await
            .unwrap();
        assert_eq!(resolved.status, ActionStatus::Rejected);
        assert!(store.pending().await.unwrap().is_empty());
        assert!(matches!(
            store
                .resolve(&action.id, ActionStatus::Approved, None)
                .await,
            Err(CoreError::ActionNotPending)
        ));
        assert!(matches!(
            store.require_pending(&action.id).await,
            Err(CoreError::ActionNotPending)
        ));
    }

    #[tokio::test]
    async fn pending_actions_expire() {
        let (store, clock, _dir) = store().await;
        let action = store
            .create("core_add_preference", &json!({}), &json!({}))
            .await
            .unwrap();
        clock.advance(Duration::hours(EXPIRY_HOURS + 1));
        assert!(store.pending().await.unwrap().is_empty());
        assert_eq!(
            store.get(&action.id).await.unwrap().unwrap().status,
            ActionStatus::Expired
        );
        assert!(matches!(
            store.require_pending(&action.id).await,
            Err(CoreError::ActionNotPending)
        ));
        assert!(matches!(
            store
                .resolve(&action.id, ActionStatus::Approved, None)
                .await,
            Err(CoreError::ActionNotPending)
        ));
    }
}
