//! `Event`, `EventStore` and the `AppEvent` bus, per `docs/PHASE1_SPEC.md` §7.5.
//!
//! `EventStore` persists events and publishes each committed change on an internal
//! `EventChange` channel. The dispatcher `CoreApp::build` starts delivers appended events to
//! every plugin's `on_event`, then emits `AppEvent::DataChanged` to shells.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::plan::PlanTrigger;
use crate::settings::Settings;
use crate::time::logical_date;

/// Where an event's data came from — recorded for audit/debugging, not currently branched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    Chat,
    Ui,
    System,
}

impl EventSource {
    fn as_str(&self) -> &'static str {
        match self {
            EventSource::Chat => "chat",
            EventSource::Ui => "ui",
            EventSource::System => "system",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "chat" => Ok(EventSource::Chat),
            "ui" => Ok(EventSource::Ui),
            "system" => Ok(EventSource::System),
            other => Err(CoreError::Validation(format!(
                "unknown event source: {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub logical_date: NaiveDate,
    pub payload: Value,
    pub source: EventSource,
}

/// Broadcast to shells (and, from M3, to the plugin dispatcher) whenever stored data changes.
#[derive(Debug, Clone)]
pub enum AppEvent {
    DataChanged {
        event_type: String,
        logical_date: NaiveDate,
    },
    PlanReady {
        date: NaiveDate,
        version: u32,
        /// Lets shells notify only for scheduled plans (§10.1).
        trigger: PlanTrigger,
    },
    PlanFailed {
        date: NaiveDate,
        error: String,
        /// `CoreError::code()` of the failure — a stable classification (e.g. `"not_configured"`,
        /// `"auth"`) a shell can use to pick a specific user-facing message, without needing to
        /// parse `error`'s English `Display` text (§14 M8: per-cause notification text).
        code: &'static str,
    },
}

/// A committed change to the event store, published on the internal channel the dispatcher
/// reads. Deletions are kept distinct because `Event` itself carries no "deleted" marker.
#[derive(Debug, Clone, PartialEq)]
pub enum EventChange {
    Appended(Event),
    Deleted(Event),
}

impl EventChange {
    pub fn event(&self) -> &Event {
        match self {
            EventChange::Appended(e) | EventChange::Deleted(e) => e,
        }
    }
}

/// Channel capacity for the `AppEvent` bus: generous enough that a burst of appends between
/// a shell's poll cycles never drops an event; a lagging receiver just misses the oldest.
const BUS_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct EventStore {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
    settings: Settings,
    bus: broadcast::Sender<AppEvent>,
    changes: broadcast::Sender<EventChange>,
}

impl EventStore {
    pub fn new(db: SqlitePool, clock: Arc<dyn Clock>, settings: Settings) -> Self {
        let (bus, _) = broadcast::channel(BUS_CAPACITY);
        let (changes, _) = broadcast::channel(BUS_CAPACITY);
        Self {
            db,
            clock,
            settings,
            bus,
            changes,
        }
    }

    /// Shell-facing `AppEvent`s.
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.bus.subscribe()
    }

    /// Committed appends and deletes, for the dispatcher.
    pub fn subscribe_changes(&self) -> broadcast::Receiver<EventChange> {
        self.changes.subscribe()
    }

    /// Emits an `AppEvent` to shells. Having no subscriber is not an error.
    pub fn publish(&self, event: AppEvent) {
        let _ = self.bus.send(event);
    }

    pub async fn append(
        &self,
        event_type: &str,
        occurred_at: DateTime<Utc>,
        payload: Value,
        source: EventSource,
    ) -> Result<Event, CoreError> {
        let boundary = self.settings.day_boundary().await?;
        let logical = logical_date(occurred_at, boundary);
        let id = Uuid::now_v7().to_string();
        let now = self.clock.now().to_rfc3339();

        sqlx::query(
            "INSERT INTO events
                (id, event_type, occurred_at, logical_date, payload_json, source,
                 created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(event_type)
        .bind(occurred_at.to_rfc3339())
        .bind(logical.to_string())
        .bind(serde_json::to_string(&payload)?)
        .bind(source.as_str())
        .bind(&now)
        .bind(&now)
        .execute(&self.db)
        .await?;

        let event = Event {
            id,
            event_type: event_type.to_string(),
            occurred_at,
            logical_date: logical,
            payload,
            source,
        };
        // No dispatcher running (e.g. a store used standalone in tests) is not an error.
        let _ = self.changes.send(EventChange::Appended(event.clone()));
        Ok(event)
    }

    pub async fn list(
        &self,
        event_type: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Event>, CoreError> {
        let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, event_type, occurred_at, logical_date, payload_json, source
             FROM events
             WHERE event_type = ? AND logical_date >= ? AND logical_date <= ? AND deleted_at IS NULL
             ORDER BY occurred_at ASC, rowid ASC",
        )
        .bind(event_type)
        .bind(from.to_string())
        .bind(to.to_string())
        .fetch_all(&self.db)
        .await?;

        rows.into_iter().map(Self::row_to_event).collect()
    }

    pub async fn latest(&self, event_type: &str) -> Result<Option<Event>, CoreError> {
        let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, event_type, occurred_at, logical_date, payload_json, source
             FROM events
             WHERE event_type = ? AND deleted_at IS NULL
             ORDER BY occurred_at DESC, rowid DESC LIMIT 1",
        )
        .bind(event_type)
        .fetch_optional(&self.db)
        .await?;

        row.map(Self::row_to_event).transpose()
    }

    /// A live (not deleted) event by id.
    pub async fn get(&self, id: &str) -> Result<Option<Event>, CoreError> {
        let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, event_type, occurred_at, logical_date, payload_json, source
             FROM events WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;

        row.map(Self::row_to_event).transpose()
    }

    pub async fn soft_delete(&self, id: &str) -> Result<Event, CoreError> {
        let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, event_type, occurred_at, logical_date, payload_json, source
             FROM events WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;

        let event = row
            .map(Self::row_to_event)
            .transpose()?
            .ok_or_else(|| CoreError::NotFound(format!("event {id}")))?;

        // `deleted_at IS NULL` + the row count make a concurrent second delete of the same
        // event a `NotFound` instead of a second `Deleted` change.
        let now = self.clock.now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE events SET deleted_at = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.db)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound(format!("event {id}")));
        }

        let _ = self.changes.send(EventChange::Deleted(event.clone()));
        Ok(event)
    }

    fn row_to_event(
        row: (String, String, String, String, String, String),
    ) -> Result<Event, CoreError> {
        let (id, event_type, occurred_at, logical_date, payload_json, source) = row;
        Ok(Event {
            id,
            event_type,
            occurred_at: DateTime::parse_from_rfc3339(&occurred_at)
                .map_err(|e| CoreError::Validation(format!("bad occurred_at in db: {e}")))?
                .with_timezone(&Utc),
            logical_date: logical_date
                .parse()
                .map_err(|e| CoreError::Validation(format!("bad logical_date in db: {e}")))?,
            payload: serde_json::from_str(&payload_json)?,
            source: EventSource::parse(&source)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    async fn store() -> (EventStore, tempfile::TempDir) {
        let (pool, dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        let settings = Settings::new(pool.clone());
        (EventStore::new(pool, clock, settings), dir)
    }

    #[tokio::test]
    async fn append_then_latest_round_trips() {
        let (store, _dir) = store().await;
        let occurred_at = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let payload = serde_json::json!({"kg": 70.5});

        let appended = store
            .append(
                "body.weight_logged",
                occurred_at,
                payload.clone(),
                EventSource::Chat,
            )
            .await
            .unwrap();

        let latest = store.latest("body.weight_logged").await.unwrap().unwrap();
        assert_eq!(latest, appended);
        assert_eq!(latest.payload, payload);
        assert_eq!(latest.source, EventSource::Chat);
    }

    #[tokio::test]
    async fn latest_is_none_for_unknown_event_type() {
        let (store, _dir) = store().await;
        assert_eq!(store.latest("does.not.exist").await.unwrap(), None);
    }

    #[tokio::test]
    async fn logical_date_uses_the_day_boundary_setting() {
        let (pool, _dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        let settings = Settings::new(pool.clone());
        settings
            .set_day_boundary(
                chrono::NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
                Utc::now(),
            )
            .await
            .unwrap();
        let store = EventStore::new(pool, clock, settings);

        // 03:00 UTC is treated as local for this test's purposes via a UTC-boundary machine;
        // what matters is that append() actually consults settings.day_boundary(), which the
        // logical_date module's own tests already prove correct in isolation.
        let occurred_at = Utc.with_ymd_and_hms(2026, 9, 23, 3, 0, 0).unwrap();
        let appended = store
            .append(
                "diet.meal_logged",
                occurred_at,
                serde_json::json!({}),
                EventSource::Chat,
            )
            .await
            .unwrap();

        let expected = logical_date(
            occurred_at,
            chrono::NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
        );
        assert_eq!(appended.logical_date, expected);
    }

    #[tokio::test]
    async fn list_filters_by_type_and_date_range() {
        let (store, _dir) = store().await;
        let day1 = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
        let day3 = Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0).unwrap();

        store
            .append(
                "diet.meal_logged",
                day1,
                serde_json::json!({"n": 1}),
                EventSource::Chat,
            )
            .await
            .unwrap();
        store
            .append(
                "diet.meal_logged",
                day2,
                serde_json::json!({"n": 2}),
                EventSource::Chat,
            )
            .await
            .unwrap();
        store
            .append(
                "diet.meal_logged",
                day3,
                serde_json::json!({"n": 3}),
                EventSource::Chat,
            )
            .await
            .unwrap();
        store
            .append(
                "body.weight_logged",
                day2,
                serde_json::json!({"n": 99}),
                EventSource::Chat,
            )
            .await
            .unwrap();

        let got = store
            .list(
                "diet.meal_logged",
                NaiveDate::from_ymd_opt(2026, 9, 21).unwrap(),
                NaiveDate::from_ymd_opt(2026, 9, 23).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].payload, serde_json::json!({"n": 2}));
    }

    #[tokio::test]
    async fn soft_delete_excludes_the_event_from_list_and_latest() {
        let (store, _dir) = store().await;
        let occurred_at = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let appended = store
            .append(
                "diet.meal_logged",
                occurred_at,
                serde_json::json!({}),
                EventSource::Chat,
            )
            .await
            .unwrap();

        let deleted = store.soft_delete(&appended.id).await.unwrap();
        assert_eq!(deleted.id, appended.id);

        assert_eq!(store.latest("diet.meal_logged").await.unwrap(), None);
        let listed = store
            .list(
                "diet.meal_logged",
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2100, 1, 1).unwrap(),
            )
            .await
            .unwrap();
        assert!(listed.is_empty());
    }

    #[tokio::test]
    async fn soft_delete_of_missing_event_is_not_found() {
        let (store, _dir) = store().await;
        assert!(matches!(
            store.soft_delete("does-not-exist").await,
            Err(CoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn soft_delete_twice_is_not_found_the_second_time() {
        let (store, _dir) = store().await;
        let occurred_at = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        let appended = store
            .append(
                "diet.meal_logged",
                occurred_at,
                serde_json::json!({}),
                EventSource::Chat,
            )
            .await
            .unwrap();

        store.soft_delete(&appended.id).await.unwrap();
        assert!(matches!(
            store.soft_delete(&appended.id).await,
            Err(CoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn append_and_soft_delete_publish_changes() {
        let (store, _dir) = store().await;
        let mut rx = store.subscribe_changes();
        let occurred_at = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();

        let appended = store
            .append(
                "diet.meal_logged",
                occurred_at,
                serde_json::json!({}),
                EventSource::Chat,
            )
            .await
            .unwrap();
        store.soft_delete(&appended.id).await.unwrap();

        assert_eq!(
            rx.try_recv().unwrap(),
            EventChange::Appended(appended.clone())
        );
        assert_eq!(rx.try_recv().unwrap(), EventChange::Deleted(appended));
    }

    #[tokio::test]
    async fn publish_reaches_app_event_subscribers() {
        let (store, _dir) = store().await;
        let mut rx = store.subscribe();
        let date = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();

        store.publish(AppEvent::DataChanged {
            event_type: "diet.meal_logged".into(),
            logical_date: date,
        });

        match rx.try_recv().unwrap() {
            AppEvent::DataChanged { event_type, .. } => assert_eq!(event_type, "diet.meal_logged"),
            other => panic!("unexpected AppEvent: {other:?}"),
        }
    }
}
