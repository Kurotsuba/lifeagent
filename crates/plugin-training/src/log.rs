//! Logged sessions: reading them back, per-day summaries, the context snippet and today card.

use std::collections::BTreeMap;

use chrono::{DateTime, Days, Local, NaiveDate, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::Serialize;
use serde_json::{json, Value};

use crate::model::{Exercise, SessionLogged};
use crate::prefs;

pub const SESSION_EVENT: &str = "training.session_logged";
/// This plugin's plan section id (also its plugin id).
pub const SECTION: &str = "training";
/// Days of history the plan and the Training page look back over.
pub const HISTORY_DAYS: u64 = 7;

#[derive(Debug, Clone, PartialEq)]
pub struct LoggedSession {
    pub entry_id: String,
    pub occurred_at: DateTime<Utc>,
    pub logical_date: NaiveDate,
    pub session: SessionLogged,
}

/// A logged session as the UI and tool results show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionView {
    pub entry_id: String,
    pub date: NaiveDate,
    /// Local "HH:MM".
    pub time: String,
    #[serde(flatten)]
    pub session: SessionLogged,
}

impl LoggedSession {
    pub fn view(&self) -> SessionView {
        SessionView {
            entry_id: self.entry_id.clone(),
            date: self.logical_date,
            time: self
                .occurred_at
                .with_timezone(&Local)
                .format("%H:%M")
                .to_string(),
            session: self.session.clone(),
        }
    }

    /// "下肢力量 50 分钟 高强度（深蹲 4×8 60kg、硬拉 3×5 80kg）".
    pub fn label(&self) -> String {
        let s = &self.session;
        let exercises: Vec<String> = s.exercises.iter().map(Exercise::label).collect();
        format!(
            "{} {:.0} 分钟 {}（{}）",
            s.title,
            s.duration_min,
            s.intensity.zh_label(),
            exercises.join("、")
        )
    }
}

/// One day's sessions and totals.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DaySummary {
    pub date: NaiveDate,
    pub sessions: Vec<SessionView>,
    pub minutes: f64,
    /// Sum over the sessions that have an energy figure.
    pub kcal: f64,
}

pub async fn sessions_between(
    ctx: &PluginContext,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<LoggedSession>, PluginError> {
    ctx.events
        .list(SESSION_EVENT, from, to)
        .await?
        .into_iter()
        .map(|event| {
            let session = serde_json::from_value(event.payload).map_err(|e| {
                PluginError::Other(format!("bad {SESSION_EVENT} payload for {}: {e}", event.id))
            })?;
            Ok(LoggedSession {
                entry_id: event.id,
                occurred_at: event.occurred_at,
                logical_date: event.logical_date,
                session,
            })
        })
        .collect()
}

/// `date − n .. date − 1`'s first day (or `date` itself if out of range).
pub fn days_back(date: NaiveDate, n: u64) -> NaiveDate {
    date.checked_sub_days(Days::new(n)).unwrap_or(date)
}

/// One summary per day from `from` to `to`, oldest first, days without sessions included.
pub fn summarize(from: NaiveDate, to: NaiveDate, sessions: &[LoggedSession]) -> Vec<DaySummary> {
    let mut by_day: BTreeMap<NaiveDate, Vec<&LoggedSession>> = BTreeMap::new();
    for s in sessions {
        by_day.entry(s.logical_date).or_default().push(s);
    }
    from.iter_days()
        .take_while(|d| *d <= to)
        .map(|date| {
            let day = by_day.remove(&date).unwrap_or_default();
            DaySummary {
                date,
                minutes: day.iter().map(|s| s.session.duration_min).sum(),
                kcal: day.iter().filter_map(|s| s.session.exercise_kcal).sum(),
                sessions: day.iter().map(|s| s.view()).collect(),
            }
        })
        .collect()
}

/// The latest logged body weight, for energy figures.
pub async fn latest_weight(ctx: &PluginContext) -> Result<Option<f64>, PluginError> {
    Ok(ctx
        .events
        .latest("body.weight_logged")
        .await?
        .and_then(|e| e.payload.get("kg").and_then(Value::as_f64)))
}

/// The training section of the current plan for `date`, if it has a valid one.
pub async fn planned_section(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<Value>, PluginError> {
    Ok(ctx
        .plans
        .current(date)
        .await?
        .and_then(|plan| plan.content.sections.get(SECTION).cloned())
        .filter(Value::is_object))
}

/// "下肢力量（傍晚，50 分钟）：深蹲 4×8、…" or "休息日", from a finalized plan section.
pub fn plan_label(section: &Value) -> String {
    if section.get("rest_day").and_then(Value::as_bool) == Some(true) {
        return "休息日".into();
    }
    let title = section
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("训练");
    let minutes = section
        .get("duration_min")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let exercises: Vec<String> = section
        .get("exercises")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| serde_json::from_value::<Exercise>(e.clone()).ok())
                .map(|e| e.label())
                .collect()
        })
        .unwrap_or_default();
    format!("{title}（{minutes:.0} 分钟）：{}", exercises.join("、"))
}

/// Today's sessions (with entry ids, so the model can delete one), the last 7 days' totals,
/// today's and tomorrow's planned training, and tomorrow's request.
pub async fn context_snippet(ctx: &PluginContext, date: NaiveDate) -> Result<String, PluginError> {
    let from = days_back(date, HISTORY_DAYS - 1);
    let sessions = sessions_between(ctx, from, date).await?;
    let today: Vec<String> = sessions
        .iter()
        .filter(|s| s.logical_date == date)
        .map(|s| format!("(entry {}) {}", s.entry_id, s.label()))
        .collect();
    let minutes: f64 = sessions.iter().map(|s| s.session.duration_min).sum();
    let mut lines = vec![
        format!(
            "今日训练：{}",
            if today.is_empty() {
                "无".to_string()
            } else {
                today.join("；")
            }
        ),
        format!("近 7 天：{} 次，共 {minutes:.0} 分钟", sessions.len()),
    ];
    if let Some(section) = planned_section(ctx, date).await? {
        lines.push(format!("今日计划训练：{}", plan_label(&section)));
    }
    if let Some(tomorrow) = date.succ_opt() {
        match planned_section(ctx, tomorrow).await? {
            Some(section) => lines.push(format!("明日计划训练：{}", plan_label(&section))),
            None => lines.push("明日尚无训练计划".into()),
        }
        if let Some(text) = prefs::day_preference(ctx, tomorrow).await? {
            lines.push(format!("明日训练偏好：{text}"));
        }
    }
    Ok(lines.join("\n"))
}

/// `{date, sessions, minutes, kcal}` for the day.
pub async fn today_card(ctx: &PluginContext, date: NaiveDate) -> Result<Value, PluginError> {
    let sessions = sessions_between(ctx, date, date).await?;
    let day = summarize(date, date, &sessions)
        .pop()
        .ok_or_else(|| PluginError::Other("empty day summary".into()))?;
    Ok(json!(day))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, log_session};

    #[tokio::test]
    async fn summarize_fills_empty_days_and_totals_each_day() {
        let env = env().await;
        let today = env.ctx.today().await.unwrap();
        log_session(&env.ctx, "跑步", 30.0, Some(180.0)).await;
        log_session(&env.ctx, "拉伸", 10.0, None).await;
        let from = days_back(today, 2);
        let sessions = sessions_between(&env.ctx, from, today).await.unwrap();

        let days = summarize(from, today, &sessions);

        assert_eq!(days.len(), 3);
        assert!(days[0].sessions.is_empty());
        assert_eq!(days[2].date, today);
        assert_eq!(days[2].sessions.len(), 2);
        assert_eq!(days[2].minutes, 40.0);
        assert_eq!(days[2].kcal, 180.0);
    }

    #[tokio::test]
    async fn context_snippet_lists_todays_sessions_with_entry_ids() {
        let env = env().await;
        let today = env.ctx.today().await.unwrap();
        let id = log_session(&env.ctx, "跑步", 30.0, Some(180.0)).await;
        prefs::set_day_preference(&env.ctx, today.succ_opt().unwrap(), "练上肢")
            .await
            .unwrap();

        let snippet = context_snippet(&env.ctx, today).await.unwrap();

        assert!(snippet.contains(&format!("今日训练：(entry {id}) 跑步 30 分钟")));
        assert!(snippet.contains("近 7 天：1 次，共 30 分钟"));
        assert!(snippet.contains("明日尚无训练计划"));
        assert!(snippet.contains("明日训练偏好：练上肢"));
    }

    #[test]
    fn plan_label_for_rest_and_training_days() {
        assert_eq!(plan_label(&json!({"rest_day": true})), "休息日");
        let section = json!({
            "rest_day": false, "title": "下肢力量", "duration_min": 50.0,
            "exercises": [{"name": "深蹲", "kind": "strength", "sets": 4, "reps": 8}]
        });
        assert_eq!(plan_label(&section), "下肢力量（50 分钟）：深蹲 4×8");
    }
}
