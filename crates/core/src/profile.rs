//! `Profile` and `ProfileService`, per `docs/PHASE1_SPEC.md` §7.4.
//!
//! Weight is deliberately not part of the profile — it is the latest `body.weight_logged`
//! event (added by the diet plugin's tools in M3/M4), so it changes without rewriting the
//! profile record.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::error::CoreError;
use crate::time::age_years;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLevel {
    Sedentary,
    Light,
    Moderate,
    Active,
    VeryActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DietaryPattern {
    Omnivore,
    Vegetarian,
    Vegan,
    Pescatarian,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub display_name: Option<String>,
    pub sex: Sex,
    pub birth_date: NaiveDate,
    pub height_cm: f64,
    pub activity_level: ActivityLevel,
    pub dietary_pattern: DietaryPattern,
    #[serde(default)]
    pub allergies: Vec<String>,
    #[serde(default)]
    pub dislikes: Vec<String>,
    pub medical_notes: Option<String>,
    pub meals_per_day: u8,
}

/// Validates a profile against the bounds in §7.4 (height 120–230 cm, age 16–100,
/// meals_per_day 3..=5). Weight is validated separately, where it is logged
/// (`core_log_weight`, M3), since it is not part of the profile.
pub fn validate(profile: &Profile, today: NaiveDate) -> Result<(), CoreError> {
    if !(120.0..=230.0).contains(&profile.height_cm) {
        return Err(CoreError::Validation(format!(
            "height_cm must be within 120..=230, got {}",
            profile.height_cm
        )));
    }

    let age = age_years(profile.birth_date, today);
    if !(16..=100).contains(&age) {
        return Err(CoreError::Validation(format!(
            "age must be within 16..=100, got {age} (birth_date {})",
            profile.birth_date
        )));
    }

    if !(3..=5).contains(&profile.meals_per_day) {
        return Err(CoreError::Validation(format!(
            "meals_per_day must be within 3..=5, got {}",
            profile.meals_per_day
        )));
    }

    Ok(())
}

#[derive(Clone)]
pub struct ProfileService {
    db: SqlitePool,
    clock: std::sync::Arc<dyn Clock>,
}

impl ProfileService {
    pub fn new(db: SqlitePool, clock: std::sync::Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    pub async fn get(&self) -> Result<Option<Profile>, CoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT data_json FROM profile WHERE id = 1")
            .fetch_optional(&self.db)
            .await?;
        match row {
            Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// Validates, then upserts the single profile row.
    pub async fn set(&self, profile: &Profile) -> Result<(), CoreError> {
        let today = crate::time::logical_date_from_local(
            self.clock.now().with_timezone(&chrono::Local).naive_local(),
            // Profile validation only needs a calendar date, not the day-boundary shift;
            // a midnight boundary makes `logical_date_from_local` a no-op date conversion.
            chrono::NaiveTime::from_hms_opt(0, 0, 0).expect("valid midnight"),
        );
        validate(profile, today)?;

        let json = serde_json::to_string(profile)?;
        let now: DateTime<Utc> = self.clock.now();
        sqlx::query(
            "INSERT INTO profile (id, data_json, updated_at) VALUES (1, ?, ?)
             ON CONFLICT(id) DO UPDATE SET data_json = excluded.data_json, updated_at = excluded.updated_at",
        )
        .bind(json)
        .bind(now.to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sample(height_cm: f64, birth_date: NaiveDate, meals_per_day: u8) -> Profile {
        Profile {
            display_name: Some("测试".into()),
            sex: Sex::Female,
            birth_date,
            height_cm,
            activity_level: ActivityLevel::Moderate,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec!["虾".into()],
            dislikes: vec!["香菜".into()],
            medical_notes: None,
            meals_per_day,
        }
    }

    const TODAY: NaiveDate = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();

    #[test]
    fn validate_accepts_a_reasonable_profile() {
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap(); // age 30
        assert!(validate(&sample(165.0, birth, 3), TODAY).is_ok());
    }

    #[test]
    fn validate_rejects_height_below_range() {
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();
        assert!(validate(&sample(119.9, birth, 3), TODAY).is_err());
    }

    #[test]
    fn validate_rejects_height_above_range() {
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();
        assert!(validate(&sample(230.1, birth, 3), TODAY).is_err());
    }

    #[test]
    fn validate_accepts_height_boundaries() {
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();
        assert!(validate(&sample(120.0, birth, 3), TODAY).is_ok());
        assert!(validate(&sample(230.0, birth, 3), TODAY).is_ok());
    }

    #[test]
    fn validate_rejects_too_young() {
        let birth = NaiveDate::from_ymd_opt(2015, 1, 1).unwrap(); // age 11
        assert!(validate(&sample(165.0, birth, 3), TODAY).is_err());
    }

    #[test]
    fn validate_rejects_too_old() {
        let birth = NaiveDate::from_ymd_opt(1900, 1, 1).unwrap(); // age 126
        assert!(validate(&sample(165.0, birth, 3), TODAY).is_err());
    }

    #[test]
    fn validate_rejects_meals_per_day_out_of_range() {
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();
        assert!(validate(&sample(165.0, birth, 2), TODAY).is_err());
        assert!(validate(&sample(165.0, birth, 6), TODAY).is_err());
    }

    #[test]
    fn validate_accepts_meals_per_day_boundaries() {
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();
        assert!(validate(&sample(165.0, birth, 3), TODAY).is_ok());
        assert!(validate(&sample(165.0, birth, 5), TODAY).is_ok());
    }

    #[tokio::test]
    async fn get_is_none_before_any_profile_is_set() {
        let (pool, _dir) = crate::db::test_pool().await;
        let service =
            ProfileService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now())));
        assert_eq!(service.get().await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let (pool, _dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        let service = ProfileService::new(pool, clock);
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();
        let profile = sample(165.0, birth, 4);

        service.set(&profile).await.unwrap();
        assert_eq!(service.get().await.unwrap(), Some(profile));
    }

    #[tokio::test]
    async fn set_rejects_invalid_profile_and_does_not_store_it() {
        let (pool, _dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        let service = ProfileService::new(pool, clock);
        let birth = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(); // too young

        assert!(service.set(&sample(165.0, birth, 3)).await.is_err());
        assert_eq!(service.get().await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_twice_overwrites_the_single_row() {
        let (pool, _dir) = crate::db::test_pool().await;
        let clock = Arc::new(crate::clock::FixedClock::new(Utc::now()));
        let service = ProfileService::new(pool, clock);
        let birth = NaiveDate::from_ymd_opt(1996, 5, 1).unwrap();

        service.set(&sample(165.0, birth, 3)).await.unwrap();
        service.set(&sample(170.0, birth, 4)).await.unwrap();

        let got = service.get().await.unwrap().unwrap();
        assert_eq!(got.height_cm, 170.0);
        assert_eq!(got.meals_per_day, 4);
    }
}
