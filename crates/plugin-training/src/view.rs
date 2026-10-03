//! Everything the Training page shows, in one read.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::Serialize;
use serde_json::Value;

use crate::log::{self, days_back, DaySummary, HISTORY_DAYS};
use crate::prefs::{self, TrainingPreferences};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrainingView {
    pub logical_date: NaiveDate,
    pub preferences: TrainingPreferences,
    /// The last 7 days up to today, oldest first.
    pub days: Vec<DaySummary>,
    pub week_sessions: usize,
    pub week_minutes: f64,
    pub week_kcal: f64,
    /// The training section of today's plan.
    pub today_plan: Option<Value>,
    pub tomorrow_date: NaiveDate,
    /// The training section of tomorrow's plan.
    pub tomorrow_plan: Option<Value>,
    /// Whether tomorrow has a plan at all (it may lack a training section if it was generated
    /// while this plugin was off).
    pub tomorrow_has_plan: bool,
    /// The user's request for tomorrow's training.
    pub tomorrow_preference: Option<String>,
}

pub async fn training_view(ctx: &PluginContext) -> Result<TrainingView, PluginError> {
    let today = ctx.today().await?;
    let tomorrow = today
        .succ_opt()
        .ok_or_else(|| PluginError::Other("date out of range".into()))?;
    let from = days_back(today, HISTORY_DAYS - 1);
    let sessions = log::sessions_between(ctx, from, today).await?;
    let days = log::summarize(from, today, &sessions);
    Ok(TrainingView {
        logical_date: today,
        preferences: prefs::get(ctx).await?,
        week_sessions: sessions.len(),
        week_minutes: days.iter().map(|d| d.minutes).sum(),
        week_kcal: days.iter().map(|d| d.kcal).sum(),
        days,
        today_plan: log::planned_section(ctx, today).await?,
        tomorrow_date: tomorrow,
        tomorrow_plan: log::planned_section(ctx, tomorrow).await?,
        tomorrow_has_plan: ctx.plans.current(tomorrow).await?.is_some(),
        tomorrow_preference: prefs::day_preference(ctx, tomorrow).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, log_session};

    #[tokio::test]
    async fn view_totals_the_week_and_reads_tomorrows_request() {
        let env = env().await;
        log_session(&env.ctx, "跑步", 30.0, Some(180.0)).await;
        let today = env.ctx.today().await.unwrap();
        prefs::set_day_preference(&env.ctx, today.succ_opt().unwrap(), "练背")
            .await
            .unwrap();

        let view = training_view(&env.ctx).await.unwrap();

        assert_eq!(view.days.len(), 7);
        assert_eq!(view.week_sessions, 1);
        assert_eq!(view.week_minutes, 30.0);
        assert_eq!(view.week_kcal, 180.0);
        assert_eq!(view.tomorrow_preference.as_deref(), Some("练背"));
        assert!(!view.tomorrow_has_plan);
        assert_eq!(view.tomorrow_plan, None);
    }
}
