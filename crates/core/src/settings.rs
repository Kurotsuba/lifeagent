//! Typed settings accessors over the generic key-value `settings` table, per
//! `docs/PHASE1_SPEC.md` §7.2.
//!
//! The table itself is a plain `key -> JSON` store, so a typed accessor for a value not yet
//! defined elsewhere in the workspace (`models: ModelsConfig`, owned by `lifeagent-llm`,
//! lands in M2) is just `get_json::<ModelsConfig>("models")` away — no change needed here
//! when that type exists.

use chrono::{DateTime, Utc};
use lifeagent_llm::router::ModelsConfig;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::{CoreError, ValidationRule};
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

    /// Rejects a `plan_time` earlier than `day_boundary` (§7.2): the scheduler's catch-up rule
    /// relies on `plan_time ≥ day_boundary` (§7.13).
    pub async fn set_plan_time(&self, t: NaiveTime, now: DateTime<Utc>) -> Result<(), CoreError> {
        check_plan_time(t, self.day_boundary().await?)?;
        self.set_json("plan_time", &format_hhmm(t), now).await
    }

    pub async fn day_boundary(&self) -> Result<NaiveTime, CoreError> {
        match self.get_json::<String>("day_boundary").await? {
            Some(s) => parse_hhmm(&s),
            None => Ok(parse_hhmm("04:00").expect("default day_boundary is valid")),
        }
    }

    /// Rejects a boundary later than the current `plan_time`; see [`Settings::set_plan_time`].
    pub async fn set_day_boundary(
        &self,
        t: NaiveTime,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        check_plan_time(self.plan_time().await?, t)?;
        self.set_json("day_boundary", &format_hhmm(t), now).await
    }

    /// Validates and stores `plan_time` and `day_boundary` together, against each other rather
    /// than against whatever was stored before. A caller changing both at once (§10.2's
    /// `save_settings`) can't use `set_plan_time` then `set_day_boundary` safely: each checks
    /// only the *other's currently-stored* value, so e.g. moving both several hours earlier can
    /// be rejected by whichever setter runs first even though the final pair is valid. This
    /// checks the new pair directly, so there's no such order dependency.
    pub async fn set_plan_time_and_day_boundary(
        &self,
        plan_time: NaiveTime,
        day_boundary: NaiveTime,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        check_plan_time(plan_time, day_boundary)?;
        self.set_json("plan_time", &format_hhmm(plan_time), now)
            .await?;
        self.set_json("day_boundary", &format_hhmm(day_boundary), now)
            .await
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

    /// `None` until onboarding configures a provider (§7.2: "none (onboarding required)").
    pub async fn models(&self) -> Result<Option<ModelsConfig>, CoreError> {
        self.get_json("models").await
    }

    pub async fn set_models(
        &self,
        config: &ModelsConfig,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.set_json("models", config, now).await
    }
}

fn check_plan_time(plan_time: NaiveTime, day_boundary: NaiveTime) -> Result<(), CoreError> {
    if plan_time < day_boundary {
        return Err(CoreError::InvalidInput(
            ValidationRule::PlanTimeBeforeDayBoundary {
                plan_time,
                day_boundary,
            },
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[tokio::test]
    async fn plan_time_earlier_than_day_boundary_is_rejected_from_either_setter() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);

        // Defaults are 21:00 / 04:00; 03:00 is before the boundary.
        assert!(settings
            .set_plan_time(parse_hhmm("03:00").unwrap(), now())
            .await
            .is_err());
        // A boundary after the 21:00 plan_time.
        assert!(settings
            .set_day_boundary(parse_hhmm("22:00").unwrap(), now())
            .await
            .is_err());
        // Nothing was stored by either rejected call.
        assert_eq!(
            settings.plan_time().await.unwrap(),
            parse_hhmm("21:00").unwrap()
        );
        assert_eq!(
            settings.day_boundary().await.unwrap(),
            parse_hhmm("04:00").unwrap()
        );
    }

    #[tokio::test]
    async fn plan_time_equal_to_day_boundary_is_allowed() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);
        settings
            .set_plan_time(parse_hhmm("04:00").unwrap(), now())
            .await
            .unwrap();
        assert_eq!(
            settings.plan_time().await.unwrap(),
            parse_hhmm("04:00").unwrap()
        );
    }

    #[tokio::test]
    async fn set_plan_time_and_day_boundary_together_rejects_an_invalid_new_pair() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);

        assert!(settings
            .set_plan_time_and_day_boundary(
                parse_hhmm("03:00").unwrap(),
                parse_hhmm("04:00").unwrap(),
                now()
            )
            .await
            .is_err());
        // Neither value was written.
        assert_eq!(
            settings.plan_time().await.unwrap(),
            parse_hhmm("21:00").unwrap()
        );
        assert_eq!(
            settings.day_boundary().await.unwrap(),
            parse_hhmm("04:00").unwrap()
        );
    }

    /// The order-dependency hazard `set_plan_time_and_day_boundary` exists to avoid: starting at
    /// the defaults (21:00 / 04:00), moving both several hours earlier to 03:00 / 02:00 is a
    /// valid final pair, but calling `set_plan_time` then `set_day_boundary` separately would
    /// reject at the first step (03:00 is earlier than the *old*, still-04:00, day_boundary).
    #[tokio::test]
    async fn set_plan_time_and_day_boundary_together_avoids_the_single_setter_order_hazard() {
        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);

        assert!(settings
            .set_plan_time(parse_hhmm("03:00").unwrap(), now())
            .await
            .is_err());

        settings
            .set_plan_time_and_day_boundary(
                parse_hhmm("03:00").unwrap(),
                parse_hhmm("02:00").unwrap(),
                now(),
            )
            .await
            .unwrap();

        assert_eq!(
            settings.plan_time().await.unwrap(),
            parse_hhmm("03:00").unwrap()
        );
        assert_eq!(
            settings.day_boundary().await.unwrap(),
            parse_hhmm("02:00").unwrap()
        );
    }

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
        assert_eq!(settings.models().await.unwrap(), None);
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
    async fn models_round_trips_through_set_and_get() {
        use lifeagent_llm::router::{ProviderConfig, ProviderKind, RoleBinding};

        let (pool, _dir) = crate::db::test_pool().await;
        let settings = Settings::new(pool);

        let binding = RoleBinding {
            provider_id: "openai".into(),
            model: "gpt-test".into(),
            temperature: None,
            effort: None,
        };
        let config = ModelsConfig {
            providers: vec![ProviderConfig {
                id: "openai".into(),
                kind: ProviderKind::OpenAiCompatible,
                base_url: "https://api.openai.com/v1".into(),
            }],
            parser: binding.clone(),
            planner: binding.clone(),
            conversation: binding,
        };

        settings.set_models(&config, now()).await.unwrap();
        assert_eq!(settings.models().await.unwrap(), Some(config));
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
