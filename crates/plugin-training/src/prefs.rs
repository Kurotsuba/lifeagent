//! The user's standing training preferences (a core setting) and per-day requests (the
//! `training_day_preferences` table) that the plan for that day follows.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::{Deserialize, Serialize};

const SETTINGS_KEY: &str = "training.preferences";
pub const MAX_TEXT_CHARS: usize = 500;

/// Creates the per-day request table (plugin migration 1).
pub const MIGRATION_1: &str = "CREATE TABLE training_day_preferences (
    plan_date TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingGoal {
    #[default]
    GeneralFitness,
    Strength,
    Muscle,
    Endurance,
    FatLoss,
}

impl TrainingGoal {
    fn prompt_name(self) -> &'static str {
        match self {
            TrainingGoal::GeneralFitness => "general fitness",
            TrainingGoal::Strength => "strength",
            TrainingGoal::Muscle => "muscle gain",
            TrainingGoal::Endurance => "endurance",
            TrainingGoal::FatLoss => "fat loss",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingPreferences {
    pub goal: TrainingGoal,
    /// Training days the user aims for per week, 1–7.
    pub days_per_week: u8,
    /// Usual session length, 10–180 minutes.
    pub session_minutes: u16,
    /// Equipment and place ("gym", "home, dumbbells", ...).
    #[serde(default)]
    pub equipment: String,
    /// Anything else the planner should respect (injuries, favorite sports, times).
    #[serde(default)]
    pub notes: String,
}

impl Default for TrainingPreferences {
    fn default() -> Self {
        Self {
            goal: TrainingGoal::GeneralFitness,
            days_per_week: 3,
            session_minutes: 45,
            equipment: String::new(),
            notes: String::new(),
        }
    }
}

impl TrainingPreferences {
    pub fn check(&self) -> Result<(), PluginError> {
        if !(1..=7).contains(&self.days_per_week) {
            return Err(PluginError::InvalidArgs("days_per_week must be 1–7".into()));
        }
        if !(10..=180).contains(&self.session_minutes) {
            return Err(PluginError::InvalidArgs(
                "session_minutes must be 10–180".into(),
            ));
        }
        for text in [&self.equipment, &self.notes] {
            check_text_len(text)?;
        }
        Ok(())
    }

    /// One line for the planner's brief.
    pub fn brief_line(&self) -> String {
        let or_unspecified = |s: &str| {
            if s.trim().is_empty() {
                "not specified".to_string()
            } else {
                s.trim().to_string()
            }
        };
        format!(
            "Standing preferences: goal {}; about {} training days per week, {} min per session; \
             equipment/place: {}; notes: {}.",
            self.goal.prompt_name(),
            self.days_per_week,
            self.session_minutes,
            or_unspecified(&self.equipment),
            or_unspecified(&self.notes),
        )
    }
}

fn check_text_len(text: &str) -> Result<(), PluginError> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(PluginError::InvalidArgs(format!(
            "text must be at most {MAX_TEXT_CHARS} characters"
        )));
    }
    Ok(())
}

/// The saved preferences, or the defaults if the user never saved any.
pub async fn get(ctx: &PluginContext) -> Result<TrainingPreferences, PluginError> {
    Ok(ctx
        .settings
        .get_json::<TrainingPreferences>(SETTINGS_KEY)
        .await?
        .unwrap_or_default())
}

pub async fn save(ctx: &PluginContext, prefs: &TrainingPreferences) -> Result<(), PluginError> {
    prefs.check()?;
    let prefs = TrainingPreferences {
        equipment: prefs.equipment.trim().to_string(),
        notes: prefs.notes.trim().to_string(),
        ..prefs.clone()
    };
    ctx.settings
        .set_json(SETTINGS_KEY, &prefs, ctx.clock.now())
        .await?;
    Ok(())
}

/// The user's request for `date`'s training ("legs, 30 min", "rest day"), if any.
pub async fn day_preference(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<String>, PluginError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT text FROM training_day_preferences WHERE plan_date = $1")
            .bind(date.to_string())
            .fetch_optional(&ctx.db)
            .await?;
    Ok(row.map(|(text,)| text))
}

/// Saves the request for `date`; blank text clears it.
pub async fn set_day_preference(
    ctx: &PluginContext,
    date: NaiveDate,
    text: &str,
) -> Result<(), PluginError> {
    let text = text.trim();
    check_text_len(text)?;
    if text.is_empty() {
        sqlx::query("DELETE FROM training_day_preferences WHERE plan_date = $1")
            .bind(date.to_string())
            .execute(&ctx.db)
            .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO training_day_preferences (plan_date, text, updated_at) VALUES ($1, $2, $3)
         ON CONFLICT (plan_date) DO UPDATE SET text = EXCLUDED.text, updated_at = EXCLUDED.updated_at",
    )
    .bind(date.to_string())
    .bind(text)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env;

    #[tokio::test]
    async fn preferences_default_then_round_trip_trimmed() {
        let env = env().await;
        assert_eq!(get(&env.ctx).await.unwrap(), TrainingPreferences::default());
        let prefs = TrainingPreferences {
            goal: TrainingGoal::Strength,
            days_per_week: 4,
            session_minutes: 60,
            equipment: " 健身房 ".into(),
            notes: "左膝旧伤".into(),
        };
        save(&env.ctx, &prefs).await.unwrap();
        let saved = get(&env.ctx).await.unwrap();
        assert_eq!(saved.equipment, "健身房");
        assert_eq!(saved.goal, TrainingGoal::Strength);
    }

    #[tokio::test]
    async fn out_of_range_preferences_are_rejected() {
        let env = env().await;
        for prefs in [
            TrainingPreferences {
                days_per_week: 0,
                ..Default::default()
            },
            TrainingPreferences {
                session_minutes: 300,
                ..Default::default()
            },
            TrainingPreferences {
                notes: "长".repeat(MAX_TEXT_CHARS + 1),
                ..Default::default()
            },
        ] {
            assert!(matches!(
                save(&env.ctx, &prefs).await,
                Err(PluginError::InvalidArgs(_))
            ));
        }
    }

    #[tokio::test]
    async fn day_preference_upserts_and_blank_clears() {
        let env = env().await;
        let day = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        assert_eq!(day_preference(&env.ctx, day).await.unwrap(), None);
        set_day_preference(&env.ctx, day, "练腿").await.unwrap();
        set_day_preference(&env.ctx, day, " 跑步 30 分钟 ")
            .await
            .unwrap();
        assert_eq!(
            day_preference(&env.ctx, day).await.unwrap().as_deref(),
            Some("跑步 30 分钟")
        );
        set_day_preference(&env.ctx, day, "  ").await.unwrap();
        assert_eq!(day_preference(&env.ctx, day).await.unwrap(), None);
    }
}
