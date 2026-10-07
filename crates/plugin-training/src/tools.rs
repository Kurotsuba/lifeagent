//! Training tools: log a session, read the log, set the request for a day, update standing
//! preferences.

use chrono::NaiveDate;
use lifeagent_core::events::EventSource;
use lifeagent_core::plugin::{PluginContext, PluginError, ToolOutput, ToolSummary};
use lifeagent_core::time::parse_local_datetime;
use lifeagent_llm::types::ToolSpec;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::energy::{session_energy, session_intensity};
use crate::log::{self, days_back, LoggedSession, HISTORY_DAYS, SESSION_EVENT};
use crate::model::{Exercise, SessionLogged};
use crate::prefs::{self, TrainingGoal, TrainingPreferences};

/// Longest range `training_get_log` returns.
const MAX_LOG_DAYS: i64 = 31;

pub fn specs() -> Vec<ToolSpec> {
    let exercise = json!({
        "type": "object",
        "required": ["name", "kind"],
        "properties": {
            "name": {"type": "string"},
            "kind": {"type": "string", "enum": ["strength", "cardio", "mobility", "sport", "other"]},
            "intensity": {"type": "string", "enum": ["low", "moderate", "high"]},
            "sets": {"type": "integer"},
            "reps": {"type": "integer"},
            "weight_kg": {"type": "number"},
            "duration_min": {"type": "number"},
            "distance_km": {"type": "number"},
            "note": {"type": "string"}
        }
    });
    vec![
        ToolSpec {
            name: "training_log_session".into(),
            description: "Log a workout or exercise session the user reports having done \
                (gym, running, cycling, swimming, sports, yoga, ...). Split it into \
                exercises with kind and intensity, and copy the numbers the user gave: sets, \
                reps and weight_kg for strength work, duration_min and distance_km for cardio. \
                Don't invent numbers the user didn't give; if nothing tells how long it took, \
                leave durations out and the app assumes typical ones. Set the session's total \
                `duration_min` only if the user states it. Set `performed_at` (local time, \
                YYYY-MM-DDTHH:MM) only if it wasn't just now. Pass the user's words in \
                `raw_text`. The app computes duration and energy; quote the returned numbers. \
                To correct a session, delete it with core_delete_entry and log it again."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["title", "exercises"],
                "properties": {
                    "title": {"type": "string"},
                    "exercises": {"type": "array", "items": exercise},
                    "duration_min": {"type": "number"},
                    "performed_at": {"type": "string"},
                    "rpe": {"type": "integer"},
                    "notes": {"type": "string"},
                    "raw_text": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "training_get_log".into(),
            description: "Get the training sessions logged between `start_date` and \
                `end_date` (YYYY-MM-DD, inclusive; default the last 7 days up to today), with \
                daily totals."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "start_date": {"type": "string"},
                    "end_date": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "training_set_day_preference".into(),
            description: "Save what the user wants from a day's training (e.g. \"legs, 30 \
                min\", \"rest day\", \"running outdoors in the morning\"), for the plan of \
                `date` (YYYY-MM-DD, default tomorrow). Empty `text` clears it. Saving doesn't \
                change an existing plan: to apply it to one, also call core_revise_plan (or \
                core_generate_plan if that day has no plan yet)."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {"type": "string"},
                    "date": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "training_update_preferences".into(),
            description: "Update the user's standing training preferences when they state a \
                lasting change: goal, training days per week (1–7), usual session length in \
                minutes (10–180), equipment/place, or notes such as injuries. Only pass the \
                fields that change."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "enum": ["general_fitness", "strength", "muscle", "endurance", "fat_loss"]
                    },
                    "days_per_week": {"type": "integer"},
                    "session_minutes": {"type": "integer"},
                    "equipment": {"type": "string"},
                    "notes": {"type": "string"}
                }
            }),
        },
    ]
}

pub async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
    match name {
        "training_log_session" => log_session(ctx, args).await,
        "training_get_log" => get_log(ctx, args).await,
        "training_set_day_preference" => set_day_preference(ctx, args).await,
        "training_update_preferences" => update_preferences(ctx, args).await,
        other => Err(PluginError::NotFound(format!("unknown tool {other:?}"))),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, PluginError> {
    serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))
}

fn parse_date(s: &str) -> Result<NaiveDate, PluginError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| PluginError::InvalidArgs(format!("invalid date {s:?}, expected YYYY-MM-DD")))
}

#[derive(Deserialize)]
struct LogSessionArgs {
    title: String,
    exercises: Vec<Exercise>,
    duration_min: Option<f64>,
    performed_at: Option<String>,
    rpe: Option<u8>,
    notes: Option<String>,
    raw_text: Option<String>,
}

async fn log_session(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: LogSessionArgs = parse_args(args)?;
    let title = args.title.trim();
    if title.is_empty() {
        return Err(PluginError::InvalidArgs("title must not be empty".into()));
    }
    if args.exercises.is_empty() {
        return Err(PluginError::InvalidArgs(
            "exercises must not be empty".into(),
        ));
    }
    for e in &args.exercises {
        e.check()
            .map_err(|reason| PluginError::InvalidArgs(format!("{}: {reason}", e.name)))?;
    }
    if args.duration_min.is_some_and(|d| !(d > 0.0 && d <= 600.0)) {
        return Err(PluginError::InvalidArgs(
            "duration_min must be 0–600".into(),
        ));
    }
    if args.rpe.is_some_and(|r| !(1..=10).contains(&r)) {
        return Err(PluginError::InvalidArgs("rpe must be 1–10".into()));
    }
    let performed_at = match &args.performed_at {
        Some(s) => parse_local_datetime(s)?,
        None => ctx.clock.now(),
    };

    let weight = log::latest_weight(ctx).await?;
    let (duration_min, exercise_kcal) = session_energy(&args.exercises, args.duration_min, weight);
    let session = SessionLogged {
        title: title.to_string(),
        raw_text: args.raw_text.unwrap_or_default().trim().to_string(),
        intensity: session_intensity(&args.exercises),
        exercises: args.exercises,
        duration_min,
        exercise_kcal,
        rpe: args.rpe,
        notes: args
            .notes
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
    };
    let payload = serde_json::to_value(&session).map_err(|e| PluginError::Other(e.to_string()))?;
    let event = ctx
        .events
        .append(SESSION_EVENT, performed_at, payload, EventSource::Chat)
        .await?;

    let date = event.logical_date;
    let week = log::sessions_between(ctx, days_back(date, HISTORY_DAYS - 1), date).await?;
    let logged = LoggedSession {
        entry_id: event.id,
        occurred_at: event.occurred_at,
        logical_date: date,
        session,
    };
    let summary = ToolSummary::new("training_session_logged")
        .with("title", &logged.session.title)
        .with("minutes", logged.session.duration_min.round());
    Ok(ToolOutput {
        content: json!({
            "session": logged.view(),
            "energy_note": if exercise_kcal.is_none() {
                "no body weight recorded, so no energy figure"
            } else {
                "exercise_kcal is net energy above rest"
            },
            "last_7_days": {
                "sessions": week.len(),
                "minutes": week.iter().map(|s| s.session.duration_min).sum::<f64>(),
            },
        }),
        summary,
    })
}

#[derive(Deserialize)]
struct GetLogArgs {
    start_date: Option<String>,
    end_date: Option<String>,
}

async fn get_log(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: GetLogArgs = parse_args(args)?;
    let to = match &args.end_date {
        Some(s) => parse_date(s)?,
        None => ctx.today().await?,
    };
    let from = match &args.start_date {
        Some(s) => parse_date(s)?,
        None => days_back(to, HISTORY_DAYS - 1),
    };
    if from > to || (to - from).num_days() >= MAX_LOG_DAYS {
        return Err(PluginError::InvalidArgs(format!(
            "start_date must be on or before end_date, at most {MAX_LOG_DAYS} days apart"
        )));
    }
    let sessions = log::sessions_between(ctx, from, to).await?;
    let days = log::summarize(from, to, &sessions);
    Ok(ToolOutput {
        content: json!({ "start_date": from, "end_date": to, "days": days }),
        summary: ToolSummary::new("training_log_viewed")
            .with("start_date", from)
            .with("end_date", to),
    })
}

#[derive(Deserialize)]
struct DayPreferenceArgs {
    text: String,
    date: Option<String>,
}

async fn set_day_preference(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: DayPreferenceArgs = parse_args(args)?;
    let date = match &args.date {
        Some(s) => parse_date(s)?,
        None => ctx
            .today()
            .await?
            .succ_opt()
            .ok_or_else(|| PluginError::InvalidArgs("date out of range".into()))?,
    };
    prefs::set_day_preference(ctx, date, &args.text).await?;
    let text = args.text.trim();
    Ok(ToolOutput {
        content: json!({ "date": date, "text": text, "saved": true }),
        summary: ToolSummary::new("training_day_preference_set")
            .with("date", date)
            .with("cleared", text.is_empty()),
    })
}

#[derive(Deserialize)]
struct UpdatePreferencesArgs {
    goal: Option<TrainingGoal>,
    days_per_week: Option<u8>,
    session_minutes: Option<u16>,
    equipment: Option<String>,
    notes: Option<String>,
}

async fn update_preferences(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: UpdatePreferencesArgs = parse_args(args)?;
    let current = prefs::get(ctx).await?;
    let updated = TrainingPreferences {
        goal: args.goal.unwrap_or(current.goal),
        days_per_week: args.days_per_week.unwrap_or(current.days_per_week),
        session_minutes: args.session_minutes.unwrap_or(current.session_minutes),
        equipment: args.equipment.unwrap_or(current.equipment),
        notes: args.notes.unwrap_or(current.notes),
    };
    prefs::save(ctx, &updated).await?;
    Ok(ToolOutput {
        content: json!({ "preferences": prefs::get(ctx).await? }),
        summary: ToolSummary::new("training_preferences_updated"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, seed_profile};

    #[tokio::test]
    async fn log_session_computes_duration_and_energy_and_appends_an_event() {
        let env = env().await;
        seed_profile(&env.ctx).await;
        let out = call(
            &env.ctx,
            "training_log_session",
            json!({
                "title": "晨跑",
                "raw_text": "早上跑了 5 公里，30 分钟",
                "exercises": [{"name": "跑步", "kind": "cardio", "duration_min": 30, "distance_km": 5}]
            }),
        )
        .await
        .unwrap();

        // (7 − 1) × 62 × 0.5 = 186.
        assert_eq!(out.content["session"]["duration_min"], 30.0);
        assert_eq!(out.content["session"]["exercise_kcal"], 186.0);
        assert_eq!(out.content["session"]["intensity"], "moderate");
        assert_eq!(out.content["last_7_days"]["sessions"], 1);
        assert_eq!(
            out.summary,
            ToolSummary::new("training_session_logged")
                .with("title", "晨跑")
                .with("minutes", 30.0)
        );
        let today = env.ctx.today().await.unwrap();
        let logged = log::sessions_between(&env.ctx, today, today).await.unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].session.raw_text, "早上跑了 5 公里，30 分钟");
    }

    #[tokio::test]
    async fn log_session_without_weight_has_no_energy() {
        let env = env().await;
        let out = call(
            &env.ctx,
            "training_log_session",
            json!({"title": "力量", "exercises": [
                {"name": "卧推", "kind": "strength", "sets": 3, "reps": 10, "weight_kg": 40}
            ]}),
        )
        .await
        .unwrap();
        assert_eq!(out.content["session"]["duration_min"], 8.0);
        assert_eq!(out.content["session"]["exercise_kcal"], Value::Null);
    }

    #[tokio::test]
    async fn log_session_rejects_bad_arguments() {
        let env = env().await;
        for args in [
            json!({"title": " ", "exercises": [{"name": "跑", "kind": "cardio"}]}),
            json!({"title": "跑", "exercises": []}),
            json!({"title": "跑", "exercises": [{"name": "跑", "kind": "cardio", "sets": 0}]}),
            json!({"title": "跑", "rpe": 11, "exercises": [{"name": "跑", "kind": "cardio"}]}),
        ] {
            assert!(matches!(
                call(&env.ctx, "training_log_session", args).await,
                Err(PluginError::InvalidArgs(_))
            ));
        }
    }

    #[tokio::test]
    async fn day_preference_defaults_to_tomorrow() {
        let env = env().await;
        let tomorrow = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let out = call(
            &env.ctx,
            "training_set_day_preference",
            json!({"text": "休息"}),
        )
        .await
        .unwrap();
        assert_eq!(out.content["date"], tomorrow.to_string());
        assert_eq!(
            prefs::day_preference(&env.ctx, tomorrow)
                .await
                .unwrap()
                .as_deref(),
            Some("休息")
        );
    }

    #[tokio::test]
    async fn update_preferences_changes_only_the_given_fields() {
        let env = env().await;
        call(
            &env.ctx,
            "training_update_preferences",
            json!({"days_per_week": 5}),
        )
        .await
        .unwrap();
        let saved = prefs::get(&env.ctx).await.unwrap();
        assert_eq!(saved.days_per_week, 5);
        assert_eq!(saved.session_minutes, 45);
        assert!(matches!(
            call(
                &env.ctx,
                "training_update_preferences",
                json!({"days_per_week": 9})
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn get_log_defaults_to_the_last_7_days_and_caps_the_range() {
        let env = env().await;
        let today = env.ctx.today().await.unwrap();
        let out = call(&env.ctx, "training_get_log", json!({})).await.unwrap();
        assert_eq!(out.content["end_date"], today.to_string());
        assert_eq!(out.content["days"].as_array().unwrap().len(), 7);
        assert!(matches!(
            call(
                &env.ctx,
                "training_get_log",
                json!({"start_date": "2026-01-01", "end_date": "2026-06-01"})
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
    }
}
