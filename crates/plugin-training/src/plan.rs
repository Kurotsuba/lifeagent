//! The training section's plan input: standing preferences, the optional request for the day,
//! and the last 7 days' log.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PlanInput, PluginContext, PluginError};
use serde_json::{json, Value};

use crate::log::{self, days_back, LoggedSession, HISTORY_DAYS, SECTION};
use crate::prefs;

/// A planned session longer than this is a violation whatever the preferences say.
pub const HARD_MAX_MINUTES: f64 = 180.0;
/// A planned session more than this share over the usual length gets a warning.
pub const OVER_USUAL_SHARE: f64 = 1.25;

/// The JSON Schema the planner's training section must match.
pub fn output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["rest_day", "title", "exercises"],
        "properties": {
            "rest_day": { "type": "boolean" },
            "title": { "type": "string" },
            "focus": { "type": "string" },
            "time_of_day": { "enum": ["morning", "midday", "afternoon", "evening"] },
            "exercises": { "type": "array", "items": {
                "type": "object",
                "required": ["name", "kind", "intensity"],
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "enum": ["strength", "cardio", "mobility", "sport", "other"] },
                    "intensity": { "enum": ["low", "moderate", "high"] },
                    "sets": { "type": "integer" },
                    "reps": { "type": "integer" },
                    "weight_kg": { "type": "number" },
                    "duration_min": { "type": "number" },
                    "distance_km": { "type": "number" },
                    "note": { "type": "string" }
                } } },
            "tips": { "type": "array", "items": { "type": "string" } }
        }
    })
}

const RULES: &str = "\
Rules for this section:
- Decide between a training day and a rest day. Plan a rest day when the user asks for one, \
when they already trained on as many of the last 6 days as their weekly target, or after 3 or \
more training days in a row.
- The user's request for this day wins over the usual schedule, unless it is unsafe.
- Don't load the same muscle groups hard on consecutive days; vary from the recent log.
- Progress gradually: at most about 10% more time or load than the comparable recent session. \
With no recent log, start at moderate intensity.
- Aim for the usual session length; never exceed the max_session_minutes target.
- Respect injuries and medical notes from the profile, the standing preferences above and the \
user's preferences list.
- Generic exercise names in the user's language. Strength work: sets and reps (weight_kg only \
when the log shows what the user lifts). Cardio and sports: duration_min (distance_km if \
useful). The app computes total time and energy itself.
- A rest day has rest_day true and lists either nothing or only light stretching (kind \
mobility, intensity low).
- \"title\": a short name for the session in the user's language (e.g. \"下肢力量\").";

/// `None` without a profile: like the diet section, nothing is planned before onboarding.
pub async fn plan_input(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<PlanInput>, PluginError> {
    if ctx.profile.get().await?.is_none() {
        tracing::warn!(%date, "no training plan input: profile missing");
        return Ok(None);
    }
    let prefs = prefs::get(ctx).await?;
    let request = prefs::day_preference(ctx, date).await?;
    let from = days_back(date, HISTORY_DAYS);
    let to = days_back(date, 1);
    let history = log::sessions_between(ctx, from, to).await?;

    let mut brief = vec![
        prefs.brief_line(),
        format!(
            "User's request for this day: {}",
            request.as_deref().unwrap_or("none.")
        ),
    ];
    if history.is_empty() {
        brief.push(format!(
            "Training log {from}..{to}: nothing logged. Start conservatively."
        ));
    } else {
        brief.push(format!("Training log {from}..{to}:"));
        brief.extend(
            history
                .iter()
                .map(|s| format!("- {}: {}", s.logical_date, s.label())),
        );
    }
    let recent = trained_days(&history, days_back(date, 6), to);
    brief.push(format!(
        "Training days in the last 6 days: {recent}; consecutive training days up to yesterday: {}.",
        streak(&history, to)
    ));
    brief.push(RULES.into());

    Ok(Some(PlanInput {
        section: SECTION.into(),
        brief: brief.join("\n"),
        targets: json!({
            "usual_session_minutes": prefs.session_minutes,
            "max_session_minutes": HARD_MAX_MINUTES,
            "days_per_week": prefs.days_per_week,
        }),
        output_schema: output_schema(),
    }))
}

/// Distinct days with a session from `from` to `to`.
fn trained_days(history: &[LoggedSession], from: NaiveDate, to: NaiveDate) -> usize {
    let mut days: Vec<NaiveDate> = history
        .iter()
        .map(|s| s.logical_date)
        .filter(|d| (from..=to).contains(d))
        .collect();
    days.dedup();
    days.len()
}

/// Consecutive days with a session, counting back from `last`.
fn streak(history: &[LoggedSession], last: NaiveDate) -> usize {
    let mut n = 0;
    let mut day = last;
    while history.iter().any(|s| s.logical_date == day) {
        n += 1;
        match day.pred_opt() {
            Some(d) => day = d,
            None => break,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, log_session, seed_profile};

    #[tokio::test]
    async fn none_without_a_profile() {
        let env = env().await;
        let today = env.ctx.today().await.unwrap();
        assert!(plan_input(&env.ctx, today).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn brief_has_preferences_request_log_and_streak() {
        let env = env().await;
        seed_profile(&env.ctx).await;
        let today = env.ctx.today().await.unwrap();
        let tomorrow = today.succ_opt().unwrap();
        log_session(&env.ctx, "跑步", 30.0, Some(180.0)).await;
        prefs::set_day_preference(&env.ctx, tomorrow, "上肢，30 分钟")
            .await
            .unwrap();

        let input = plan_input(&env.ctx, tomorrow).await.unwrap().unwrap();

        assert_eq!(input.section, "training");
        assert_eq!(input.targets["usual_session_minutes"], 45);
        assert!(input
            .brief
            .contains("about 3 training days per week, 45 min"));
        assert!(input
            .brief
            .contains("User's request for this day: 上肢，30 分钟"));
        assert!(input.brief.contains(&format!("- {today}: 跑步 30 分钟")));
        assert!(input.brief.contains(
            "Training days in the last 6 days: 1; consecutive training days up to yesterday: 1."
        ));
    }

    #[tokio::test]
    async fn an_empty_log_says_so() {
        let env = env().await;
        seed_profile(&env.ctx).await;
        let tomorrow = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let input = plan_input(&env.ctx, tomorrow).await.unwrap().unwrap();
        assert!(input
            .brief
            .contains("nothing logged. Start conservatively."));
        assert!(input.brief.contains("User's request for this day: none."));
    }
}
