//! Typed settings accessors over the generic key-value `settings` table, per
//! `docs/PHASE1_SPEC.md` §7.2.
//!
//! The table itself is a plain `key -> JSON` store, so a typed accessor for a value not yet
//! defined elsewhere in the workspace (`models: ModelsConfig`, owned by `lifeagent-llm`,
//! lands in M2) is just `get_json::<ModelsConfig>("models")` away — no change needed here
//! when that type exists.

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::CoreError;
use crate::time::{format_hhmm, parse_hhmm};
use chrono::NaiveTime;

#[derive(Clone)]
pub struct Settings {
    db: SqlitePool,
}

impl Settings {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    /// Reads a setting's raw JSON value, or `None` if it has never been set.
    pub async fn get_raw(&self, key: &str) -> Result<Option<Value>, CoreError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value_json FROM settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.db)
                .await?;
        match row {
            Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// Writes a setting's value, overwriting any previous one.
    pub async fn set_raw(
        &self,
        key: &str,
        value: Value,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let json = serde_json::to_string(&value)?;
        sqlx::query(
            "INSERT INTO settings (key, value_json, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(json)
        .bind(now.to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Generic typed read: deserializes the stored JSON as `T`, or `None` if unset.
    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, CoreError> {
        match self.get_raw(key).await? {
            Some(v) => Ok(Some(serde_json::from_value(v)?)),
            None => Ok(None),
        }
    }

    /// Generic typed write: serializes `value` as JSON and stores it under `key`.
    pub async fn set_json<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.set_raw(key, serde_json::to_value(value)?, now).await
    }

    pub async fn plan_time(&self) -> Result<NaiveTime, CoreError> {
        match self.get_json::<String>("plan_time").await? {
            Some(s) => parse_hhmm(&s),
            None => Ok(parse_hhmm("21:00").expect("default plan_time is valid")),
        }
    }

    pub async fn set_plan_time(&self, t: NaiveTime, now: DateTime<Utc>) -> Result<(), CoreError> {
        self.set_json("plan_time", &format_hhmm(t), now).await
    }

    pub async fn day_boundary(&self) -> Result<NaiveTime, CoreError> {
        match self.get_json::<String>("day_boundary").await? {
            Some(s) => parse_hhmm(&s),
            None => Ok(parse_hhmm("04:00").expect("default day_boundary is valid")),
        }
    }

    pub async fn set_day_boundary(
        &self,
        t: NaiveTime,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.set_json("day_boundary", &format_hhmm(t), now).await
    }

    pub async fn notifications_enabled(&self) -> Result<bool, CoreError> {
        Ok(self
            .get_json::<bool>("notifications_enabled")
            .await?
            .unwrap_or(true))
    }

    pub async fn set_notifications_enabled(
        &self,
        on: bool,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.set_json("notifications_enabled", &on, now).await
    }

    pub async fn autostart(&self) -> Result<bool, CoreError> {
        Ok(self.get_json::<bool>("autostart").await?.unwrap_or(false))
    }

    pub async fn set_autostart(&self, on: bool, now: DateTime<Utc>) -> Result<(), CoreError> {
        self.set_json("autostart", &on, now).await
    }

    pub async fn onboarding_complete(&self) -> Result<bool, CoreError> {
        Ok(self
            .get_json::<bool>("onboarding_complete")
            .await?
            .unwrap_or(false))
    }

    pub async fn set_onboarding_complete(
        &self,
        on: bool,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.set_json("onboarding_complete", &on, now).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn defaults_match_spec_when_unset() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);

        assert_eq!(
            settings.plan_time().await.unwrap(),
            parse_hhmm("21:00").unwrap()
        );
        assert_eq!(
            settings.day_boundary().await.unwrap(),
            parse_hhmm("04:00").unwrap()
        );
        assert!(settings.notifications_enabled().await.unwrap());
        assert!(!settings.autostart().await.unwrap());
        assert!(!settings.onboarding_complete().await.unwrap());
    }

    #[tokio::test]
    async fn set_then_get_round_trips_for_every_typed_setting() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);
        let t = now();

        settings
            .set_plan_time(parse_hhmm("22:30").unwrap(), t)
            .await
            .unwrap();
        assert_eq!(
            settings.plan_time().await.unwrap(),
            parse_hhmm("22:30").unwrap()
        );

        settings
            .set_day_boundary(parse_hhmm("03:00").unwrap(), t)
            .await
            .unwrap();
        assert_eq!(
            settings.day_boundary().await.unwrap(),
            parse_hhmm("03:00").unwrap()
        );

        settings.set_notifications_enabled(false, t).await.unwrap();
        assert!(!settings.notifications_enabled().await.unwrap());

        settings.set_autostart(true, t).await.unwrap();
        assert!(settings.autostart().await.unwrap());

        settings.set_onboarding_complete(true, t).await.unwrap();
        assert!(settings.onboarding_complete().await.unwrap());
    }

    #[tokio::test]
    async fn set_raw_overwrites_previous_value() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);

        settings
            .set_raw("k", serde_json::json!(1), now())
            .await
            .unwrap();
        settings
            .set_raw("k", serde_json::json!(2), now())
            .await
            .unwrap();

        assert_eq!(
            settings.get_raw("k").await.unwrap(),
            Some(serde_json::json!(2))
        );
    }

    #[tokio::test]
    async fn get_raw_is_none_when_never_set() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);
        assert_eq!(settings.get_raw("never-set").await.unwrap(), None);
    }

    #[tokio::test]
    async fn generic_json_round_trip_for_arbitrary_type() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Sample {
            a: u32,
            b: String,
        }
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);
        let value = Sample {
            a: 7,
            b: "hi".into(),
        };

        settings.set_json("sample", &value, now()).await.unwrap();
        let round_tripped: Sample = settings.get_json("sample").await.unwrap().unwrap();
        assert_eq!(round_tripped, value);
    }
}
