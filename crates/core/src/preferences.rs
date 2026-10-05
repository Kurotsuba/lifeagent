//! Preferences: dislikes and other durable or time-limited wishes that shape plans. Replaced
//! Phase 1's free-text memory notes (`docs/DECISIONS.md`, "Preferences replace memory notes").
//!
//! Each preference can be limited to a range of logical dates (`valid_from`/`valid_until`, both
//! inclusive, either open), e.g. "only convenience-store food" until the end of a busy week.
//! Without an end date a new preference lasts one month ([`DEFAULT_VALIDITY_MONTHS`]) unless
//! it is marked `permanent`, so stale wishes don't pile up in every prompt.
//! Allergies are not preferences: they stay on the profile as a permanent hard constraint.

use std::sync::Arc;

use chrono::{DateTime, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{CoreError, ValidationRule};

/// Matches the `core_add_preference` tool's stated limit.
pub const MAX_TEXT_LEN: usize = 300;

/// How long a new preference without `valid_until` lasts, counted from its start.
pub const DEFAULT_VALIDITY_MONTHS: u32 = 1;

/// How many of the newest preferences `dislikes_on` looks through.
const MAX_DISLIKES: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceKind {
    /// A food to avoid; `text` is the food's name. The diet plan validator warns on matches.
    Dislike,
    /// Anything else that should shape plans ("weekday lunch is at the company canteen").
    General,
}

impl PreferenceKind {
    fn as_str(self) -> &'static str {
        match self {
            PreferenceKind::Dislike => "dislike",
            PreferenceKind::General => "general",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "dislike" => Ok(PreferenceKind::Dislike),
            "general" => Ok(PreferenceKind::General),
            other => Err(CoreError::Validation(format!(
                "bad preference kind in db: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preference {
    pub id: String,
    pub kind: PreferenceKind,
    pub text: String,
    pub valid_from: Option<NaiveDate>,
    pub valid_until: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

impl Preference {
    /// Whether the preference applies on logical date `date`.
    pub fn is_active_on(&self, date: NaiveDate) -> bool {
        self.valid_from.is_none_or(|from| from <= date)
            && self.valid_until.is_none_or(|until| date <= until)
    }

    /// Whether it still applies on `date` or some later day.
    pub fn is_current_on(&self, date: NaiveDate) -> bool {
        self.valid_until.is_none_or(|until| date <= until)
    }

    /// The dates as a short English suffix for prompts: "", " (until 2026-10-09)", ...
    pub fn validity_label(&self) -> String {
        match (self.valid_from, self.valid_until) {
            (None, None) => String::new(),
            (Some(from), None) => format!(" (from {from})"),
            (None, Some(until)) => format!(" (until {until})"),
            (Some(from), Some(until)) => format!(" ({from} to {until})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewPreference {
    pub kind: PreferenceKind,
    pub text: String,
    #[serde(default)]
    pub valid_from: Option<NaiveDate>,
    #[serde(default)]
    pub valid_until: Option<NaiveDate>,
    /// No end date. Without it, a missing `valid_until` defaults to one month.
    #[serde(default)]
    pub permanent: bool,
}

/// The last day (inclusive) of the default period starting on `start`: one month later, minus
/// a day, so 2026-10-03 gives 2026-11-02. Month ends clamp (01-31 gives 02-27 or 02-28).
pub fn default_valid_until(start: NaiveDate) -> Option<NaiveDate> {
    start
        .checked_add_months(Months::new(DEFAULT_VALIDITY_MONTHS))?
        .pred_opt()
}

type Row = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

#[derive(Clone)]
pub struct PreferenceService {
    db: sqlx::PgPool,
    clock: Arc<dyn Clock>,
}

impl PreferenceService {
    pub fn new(db: sqlx::PgPool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    /// Adds a preference. Rejects empty (after trimming) or over-length text, and a
    /// `valid_from` later than `valid_until`. Unless `permanent`, a missing `valid_until`
    /// becomes the default period from `valid_from`, else from `today` (the logical date).
    pub async fn add(
        &self,
        new: &NewPreference,
        today: NaiveDate,
    ) -> Result<Preference, CoreError> {
        let text = new.text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(ValidationRule::NoteEmpty));
        }
        if text.chars().count() > MAX_TEXT_LEN {
            return Err(CoreError::InvalidInput(ValidationRule::NoteTooLong {
                got: text.chars().count(),
                max: MAX_TEXT_LEN,
            }));
        }
        let valid_until = match (new.valid_until, new.permanent) {
            (Some(until), _) => Some(until),
            (None, true) => None,
            (None, false) => default_valid_until(new.valid_from.unwrap_or(today)),
        };
        if let (Some(from), Some(until)) = (new.valid_from, valid_until) {
            if from > until {
                return Err(CoreError::InvalidInput(
                    ValidationRule::PreferenceDatesReversed {
                        valid_from: from,
                        valid_until: until,
                    },
                ));
            }
        }

        let id = Uuid::now_v7().to_string();
        let created_at = self.clock.now();
        sqlx::query(
            "INSERT INTO preferences (id, kind, text, valid_from, valid_until, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(new.kind.as_str())
        .bind(text)
        .bind(new.valid_from.map(|d| d.to_string()))
        .bind(valid_until.map(|d| d.to_string()))
        .bind(created_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        Ok(Preference {
            id,
            kind: new.kind,
            text: text.to_string(),
            valid_from: new.valid_from,
            valid_until,
            created_at,
        })
    }

    /// One preference that hasn't been removed.
    pub async fn get(&self, id: &str) -> Result<Option<Preference>, CoreError> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, kind, text, valid_from, valid_until, created_at FROM preferences
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        row.map(from_row).transpose()
    }

    /// Newest-first, excluding removed ones, expired ones included (Settings shows them).
    pub async fn list(&self, limit: u32) -> Result<Vec<Preference>, CoreError> {
        // `seq DESC` breaks ties when two share the same `created_at` (e.g. under a
        // `FixedClock` that hasn't advanced between calls).
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, kind, text, valid_from, valid_until, created_at FROM preferences
             WHERE deleted_at IS NULL ORDER BY created_at DESC, seq DESC LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.db)
        .await?;
        rows.into_iter().map(from_row).collect()
    }

    /// Newest-first, those that still apply on `date` or later (the chat context, so the model
    /// sees upcoming ones too and can remove them).
    pub async fn current(&self, date: NaiveDate, limit: u32) -> Result<Vec<Preference>, CoreError> {
        Ok(self
            .list(limit)
            .await?
            .into_iter()
            .filter(|p| p.is_current_on(date))
            .collect())
    }

    /// Newest-first, those that apply on `date` (plans for that date).
    pub async fn active_on(
        &self,
        date: NaiveDate,
        limit: u32,
    ) -> Result<Vec<Preference>, CoreError> {
        Ok(self
            .list(limit)
            .await?
            .into_iter()
            .filter(|p| p.is_active_on(date))
            .collect())
    }

    /// Names of the foods disliked on `date`, oldest first (the order the user added them).
    pub async fn dislikes_on(&self, date: NaiveDate) -> Result<Vec<String>, CoreError> {
        let mut dislikes: Vec<String> = self
            .active_on(date, MAX_DISLIKES)
            .await?
            .into_iter()
            .filter(|p| p.kind == PreferenceKind::Dislike)
            .map(|p| p.text)
            .collect();
        dislikes.reverse();
        Ok(dislikes)
    }

    /// Soft-deletes a preference. Errors `NotFound` if it does not exist or was already
    /// removed.
    pub async fn remove(&self, id: &str) -> Result<(), CoreError> {
        let now = self.clock.now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE preferences SET deleted_at = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&self.db)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound(format!("preference {id}")));
        }
        Ok(())
    }
}

fn from_row(row: Row) -> Result<Preference, CoreError> {
    let (id, kind, text, valid_from, valid_until, created_at) = row;
    let date = |s: Option<String>| -> Result<Option<NaiveDate>, CoreError> {
        s.map(|s| {
            NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .map_err(|e| CoreError::Validation(format!("bad preference date in db: {e}")))
        })
        .transpose()
    };
    Ok(Preference {
        id,
        kind: PreferenceKind::parse(&kind)?,
        text,
        valid_from: date(valid_from)?,
        valid_until: date(valid_until)?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map_err(|e| CoreError::Validation(format!("bad created_at in db: {e}")))?
            .with_timezone(&Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> (PreferenceService, crate::db::TestDb) {
        let (pool, dir) = crate::db::test_pool().await;
        (
            PreferenceService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now()))),
            dir,
        )
    }

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    /// Open-ended, so tests about other things don't depend on the default period.
    fn general(text: &str) -> NewPreference {
        NewPreference {
            kind: PreferenceKind::General,
            text: text.into(),
            valid_from: None,
            valid_until: None,
            permanent: true,
        }
    }

    fn dated(from: Option<u32>, until: Option<u32>) -> NewPreference {
        NewPreference {
            valid_from: from.map(d),
            valid_until: until.map(d),
            permanent: false,
            ..general("只能吃便利店")
        }
    }

    #[tokio::test]
    async fn add_then_list_round_trips() {
        let (service, _dir) = service().await;
        let added = service
            .add(
                &NewPreference {
                    kind: PreferenceKind::Dislike,
                    text: "香菜".into(),
                    valid_from: Some(d(3)),
                    valid_until: Some(d(9)),
                    permanent: false,
                },
                d(3),
            )
            .await
            .unwrap();

        assert_eq!(service.list(50).await.unwrap(), vec![added]);
    }

    #[tokio::test]
    async fn add_rejects_empty_and_over_length_text() {
        let (service, _dir) = service().await;
        assert!(service.add(&general("   "), d(3)).await.is_err());
        assert!(service
            .add(&general(&"a".repeat(MAX_TEXT_LEN + 1)), d(3))
            .await
            .is_err());
        assert!(service
            .add(&general(&"a".repeat(MAX_TEXT_LEN)), d(3))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn add_trims_whitespace() {
        let (service, _dir) = service().await;
        assert_eq!(
            service.add(&general("  香菜  "), d(3)).await.unwrap().text,
            "香菜"
        );
    }

    #[tokio::test]
    async fn add_rejects_reversed_dates_but_allows_a_single_day() {
        let (service, _dir) = service().await;
        assert!(matches!(
            service.add(&dated(Some(9), Some(3)), d(3)).await,
            Err(CoreError::InvalidInput(
                ValidationRule::PreferenceDatesReversed { .. }
            ))
        ));
        assert!(service.add(&dated(Some(5), Some(5)), d(3)).await.is_ok());
    }

    #[tokio::test]
    async fn without_an_end_date_it_lasts_one_month_from_its_start() {
        let (service, _dir) = service().await;
        let from_today = service.add(&dated(None, None), d(3)).await.unwrap();
        assert_eq!(from_today.valid_until, NaiveDate::from_ymd_opt(2026, 11, 2));

        let from_later = service.add(&dated(Some(20), None), d(3)).await.unwrap();
        assert_eq!(
            from_later.valid_until,
            NaiveDate::from_ymd_opt(2026, 11, 19)
        );

        let permanent = service.add(&general("不吃辣"), d(3)).await.unwrap();
        assert_eq!(permanent.valid_until, None);

        // Stored, not only returned.
        let stored = service.list(50).await.unwrap();
        assert_eq!(stored, vec![permanent, from_later, from_today]);
    }

    #[test]
    fn default_valid_until_clamps_at_month_end() {
        let jan31 = NaiveDate::from_ymd_opt(2027, 1, 31).unwrap();
        assert_eq!(
            default_valid_until(jan31),
            NaiveDate::from_ymd_opt(2027, 2, 27)
        );
    }

    #[tokio::test]
    async fn list_is_newest_first_and_respects_limit() {
        let (service, _dir) = service().await;
        let first = service.add(&general("一"), d(3)).await.unwrap();
        let second = service.add(&general("二"), d(3)).await.unwrap();

        assert_eq!(service.list(50).await.unwrap(), vec![second.clone(), first]);
        assert_eq!(service.list(1).await.unwrap(), vec![second]);
    }

    #[tokio::test]
    async fn active_on_includes_both_ends_of_the_range() {
        let (service, _dir) = service().await;
        let week = service.add(&dated(Some(3), Some(9)), d(3)).await.unwrap();
        let open = service.add(&general("不吃辣"), d(3)).await.unwrap();

        assert_eq!(
            service.active_on(d(2), 50).await.unwrap(),
            vec![open.clone()]
        );
        assert_eq!(
            service.active_on(d(3), 50).await.unwrap(),
            vec![open.clone(), week.clone()]
        );
        assert_eq!(
            service.active_on(d(9), 50).await.unwrap(),
            vec![open.clone(), week]
        );
        assert_eq!(service.active_on(d(10), 50).await.unwrap(), vec![open]);
    }

    #[tokio::test]
    async fn dislikes_on_lists_active_dislikes_oldest_first() {
        let (service, _dir) = service().await;
        let dislike = |text: &str, until: Option<u32>| NewPreference {
            kind: PreferenceKind::Dislike,
            text: text.into(),
            valid_from: None,
            valid_until: until.map(d),
            permanent: until.is_none(),
        };
        service.add(&dislike("香菜", None), d(3)).await.unwrap();
        service.add(&general("不吃辣"), d(3)).await.unwrap();
        service.add(&dislike("青椒", Some(5)), d(3)).await.unwrap();

        assert_eq!(
            service.dislikes_on(d(5)).await.unwrap(),
            vec!["香菜", "青椒"]
        );
        assert_eq!(service.dislikes_on(d(6)).await.unwrap(), vec!["香菜"]);
    }

    #[tokio::test]
    async fn current_keeps_upcoming_and_drops_expired() {
        let (service, _dir) = service().await;
        let upcoming = service.add(&dated(Some(20), None), d(3)).await.unwrap();
        service.add(&dated(None, Some(2)), d(3)).await.unwrap();

        assert_eq!(service.current(d(3), 50).await.unwrap(), vec![upcoming]);
    }

    #[tokio::test]
    async fn remove_hides_it_and_a_second_remove_is_not_found() {
        let (service, _dir) = service().await;
        let added = service.add(&general("香菜"), d(3)).await.unwrap();

        service.remove(&added.id).await.unwrap();
        assert_eq!(service.list(50).await.unwrap(), vec![]);
        assert!(matches!(
            service.remove(&added.id).await,
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn validity_label_covers_each_combination() {
        let p = |from: Option<u32>, until: Option<u32>| Preference {
            id: "x".into(),
            kind: PreferenceKind::General,
            text: "x".into(),
            valid_from: from.map(d),
            valid_until: until.map(d),
            created_at: Utc::now(),
        };
        assert_eq!(p(None, None).validity_label(), "");
        assert_eq!(p(Some(3), None).validity_label(), " (from 2026-10-03)");
        assert_eq!(p(None, Some(9)).validity_label(), " (until 2026-10-09)");
        assert_eq!(
            p(Some(3), Some(9)).validity_label(),
            " (2026-10-03 to 2026-10-09)"
        );
    }
}
