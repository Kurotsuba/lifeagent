//! Finalize and validate the training section. Violations trigger the planner's retry.
//! Invalid structure, invalid exercises and the hard duration limit block publication if
//! still unmet. Other deviations can remain as warnings, like the diet section's.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PlanNote, PlanSection, PluginContext, PluginError};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::energy::{exercise_minutes, session_energy, session_intensity};
use crate::log::{self, days_back, HISTORY_DAYS};
use crate::model::{Exercise, ExerciseKind, Intensity};
use crate::plan::{HARD_MAX_MINUTES, OVER_USUAL_SHARE};
use crate::prefs;

/// A planned session this much longer than the longest recent one gets a warning.
const MAX_JUMP_SHARE: f64 = 1.5;

#[derive(Deserialize)]
struct PlannedSection {
    rest_day: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    time_of_day: Option<String>,
    #[serde(default)]
    exercises: Vec<Exercise>,
    #[serde(default)]
    tips: Vec<String>,
}

fn invalid(reason: String) -> PlanSection {
    PlanSection {
        data: Value::Null,
        warnings: Vec::new(),
        violations: vec![PlanNote::new(
            "training_invalid_structure",
            format!("The training section has the wrong shape: {reason}"),
        )
        .with("reason", reason)
        .blocking()],
    }
}

pub async fn finalize(
    ctx: &PluginContext,
    date: NaiveDate,
    raw: Value,
) -> Result<PlanSection, PluginError> {
    let planned: PlannedSection = match serde_json::from_value(raw) {
        Ok(p) => p,
        Err(e) => return Ok(invalid(e.to_string())),
    };
    let prefs = prefs::get(ctx).await?;
    let mut violations = Vec::new();
    let mut warnings = Vec::new();

    for e in &planned.exercises {
        if let Err(problem) = e.check() {
            let name = e.name.trim();
            violations.push(
                PlanNote::new(
                    "training_invalid_exercise",
                    format!("\"{name}\": {problem}"),
                )
                .with("name", name)
                .with("problem", problem)
                .blocking(),
            );
        }
    }
    if planned.rest_day {
        if planned
            .exercises
            .iter()
            .any(|e| e.kind != ExerciseKind::Mobility || e.intensity != Intensity::Low)
        {
            violations.push(PlanNote::new(
                "training_rest_day_too_hard",
                "A rest day may only have low-intensity mobility or stretching",
            ));
        }
    } else if planned.exercises.is_empty() {
        violations.push(PlanNote::new(
            "training_no_exercises",
            "A training day needs at least one exercise",
        ));
    }

    let weight = log::latest_weight(ctx).await?;
    let (minutes, kcal) = session_energy(&planned.exercises, None, weight);
    let intensity = session_intensity(&planned.exercises);

    if minutes > HARD_MAX_MINUTES {
        violations.push(
            PlanNote::new(
                "training_too_long",
                format!("The session's {minutes:.0} minutes exceed the {HARD_MAX_MINUTES:.0}-minute limit"),
            )
            .with("minutes", minutes.round())
            .with("max", HARD_MAX_MINUTES.round())
            .blocking(),
        );
    }
    if !planned.rest_day {
        let usual = f64::from(prefs.session_minutes);
        if minutes > usual * OVER_USUAL_SHARE {
            warnings.push(
                PlanNote::new(
                    "training_over_usual",
                    format!(
                        "The session's {minutes:.0} minutes are well over the usual {usual:.0}"
                    ),
                )
                .with("minutes", minutes.round())
                .with("usual", usual.round()),
            );
        }
        let history =
            log::sessions_between(ctx, days_back(date, HISTORY_DAYS), days_back(date, 1)).await?;
        let longest = history
            .iter()
            .map(|s| s.session.duration_min)
            .fold(0.0, f64::max);
        if history.is_empty() {
            if intensity == Intensity::High {
                warnings.push(PlanNote::new(
                    "training_hard_after_break",
                    "No training in the last 7 days; start at low to moderate intensity",
                ));
            }
        } else if minutes > longest * MAX_JUMP_SHARE + 10.0 {
            warnings.push(
                PlanNote::new(
                    "training_big_jump",
                    format!(
                        "Over 1.5× the longest session of the last 7 days ({longest:.0} minutes); build up gradually"
                    ),
                )
                .with("longest", longest.round()),
            );
        }
        let hard_day = |d: NaiveDate| {
            history
                .iter()
                .any(|s| s.logical_date == d && s.session.intensity == Intensity::High)
        };
        if intensity == Intensity::High
            && hard_day(days_back(date, 1))
            && hard_day(days_back(date, 2))
        {
            warnings.push(PlanNote::new(
                "training_third_hard_day",
                "The previous two days were high intensity; lower the intensity or rest",
            ));
        }
    }

    // Empty when the planner gave none: the UI shows its own "rest day" / "training".
    let title = planned.title.trim().to_string();
    let exercises: Vec<Value> = planned
        .exercises
        .iter()
        .map(|e| {
            let mut v = json!(e);
            v["minutes"] = json!(exercise_minutes(e).round());
            v
        })
        .collect();

    Ok(PlanSection {
        data: json!({
            "rest_day": planned.rest_day,
            "title": title,
            "focus": planned.focus,
            "time_of_day": planned.time_of_day,
            "exercises": exercises,
            "duration_min": minutes,
            "exercise_kcal": kcal,
            "intensity": intensity,
            "tips": planned.tips,
        }),
        warnings,
        violations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(notes: &[PlanNote]) -> Vec<&str> {
        notes.iter().map(PlanNote::kind).collect()
    }
    use crate::testutil::{env, log_session_with, seed_profile};

    fn squat(sets: u32, intensity: &str) -> Value {
        json!({"name": "深蹲", "kind": "strength", "intensity": intensity, "sets": sets, "reps": 8})
    }

    #[tokio::test]
    async fn a_sound_session_has_computed_minutes_and_energy() {
        let env = env().await;
        seed_profile(&env.ctx).await;
        let tomorrow = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let raw = json!({
            "rest_day": false, "title": "下肢力量", "time_of_day": "evening",
            "exercises": [
                squat(4, "moderate"),
                {"name": "慢跑", "kind": "cardio", "intensity": "low", "duration_min": 20}
            ]
        });

        let section = finalize(&env.ctx, tomorrow, raw).await.unwrap();

        assert!(section.violations.is_empty());
        // 10 min strength (4 × 2.5) + 20 min cardio = 30 min. At 62 kg:
        // (5 − 1) × 62 × 10/60 = 41.3; (4 − 1) × 62 × 20/60 = 62 → 103 kcal.
        assert_eq!(section.data["duration_min"], 30.0);
        assert_eq!(section.data["exercise_kcal"], 103.0);
        assert_eq!(section.data["exercises"][0]["minutes"], 10.0);
        assert_eq!(section.data["intensity"], "moderate");
        assert_eq!(section.data["time_of_day"], "evening");
    }

    #[tokio::test]
    async fn structure_and_rest_day_rules_are_violations() {
        let env = env().await;
        let day = env.ctx.today().await.unwrap();
        let section = finalize(&env.ctx, day, json!({"exercises": []}))
            .await
            .unwrap();
        assert_eq!(
            kinds(&section.violations),
            vec!["training_invalid_structure"]
        );
        assert!(section.violations[0].blocks_plan());

        let section = finalize(&env.ctx, day, json!({"rest_day": false, "exercises": []}))
            .await
            .unwrap();
        assert_eq!(kinds(&section.violations), vec!["training_no_exercises"]);
        assert!(!section.violations[0].blocks_plan());

        let section = finalize(
            &env.ctx,
            day,
            json!({"rest_day": true, "exercises": [squat(3, "high")]}),
        )
        .await
        .unwrap();
        assert_eq!(
            kinds(&section.violations),
            vec!["training_rest_day_too_hard"]
        );
        let section = finalize(
            &env.ctx,
            day,
            json!({"rest_day": true, "exercises": [
                {"name": "拉伸", "kind": "mobility", "intensity": "low", "duration_min": 10}
            ]}),
        )
        .await
        .unwrap();
        assert!(section.violations.is_empty());
        assert_eq!(section.data["title"], "");
    }

    #[tokio::test]
    async fn too_long_is_a_violation_and_over_usual_a_warning() {
        let env = env().await;
        let day = env.ctx.today().await.unwrap();
        let run = |min: u32| {
            json!({"rest_day": false, "title": "跑", "exercises": [
                {"name": "跑步", "kind": "cardio", "intensity": "moderate", "duration_min": min}
            ]})
        };
        let section = finalize(&env.ctx, day, run(200)).await.unwrap();
        assert_eq!(kinds(&section.violations), vec!["training_too_long"]);
        assert_eq!(section.violations[0].0["max"], 180.0);
        assert!(section.violations[0].blocks_plan());

        let section = finalize(&env.ctx, day, run(60)).await.unwrap();
        assert!(section.violations.is_empty());
        assert_eq!(kinds(&section.warnings), vec!["training_over_usual"]);
        assert_eq!(section.warnings[0].0["usual"], 45.0);
        assert!(!section.warnings[0].blocks_plan());
    }

    #[tokio::test]
    async fn history_based_warnings() {
        let env = env().await;
        let today = env.ctx.today().await.unwrap();
        let tomorrow = today.succ_opt().unwrap();
        let hiit = json!({"rest_day": false, "title": "HIIT", "exercises": [
            {"name": "间歇跑", "kind": "cardio", "intensity": "high", "duration_min": 20}
        ]});

        // No history and high intensity.
        let section = finalize(&env.ctx, tomorrow, hiit.clone()).await.unwrap();
        assert_eq!(kinds(&section.warnings), vec!["training_hard_after_break"]);

        // High intensity on both of the two previous days.
        log_session_with(&env.ctx, today, Intensity::High, 20.0).await;
        log_session_with(&env.ctx, today.pred_opt().unwrap(), Intensity::High, 20.0).await;
        let section = finalize(&env.ctx, tomorrow, hiit).await.unwrap();
        assert_eq!(kinds(&section.warnings), vec!["training_third_hard_day"]);

        // Far longer than the longest recent session (20 min).
        let long = json!({"rest_day": false, "title": "骑行", "exercises": [
            {"name": "骑行", "kind": "cardio", "intensity": "low", "duration_min": 45}
        ]});
        let section = finalize(&env.ctx, tomorrow, long).await.unwrap();
        assert_eq!(
            section.warnings,
            vec![PlanNote::new(
                "training_big_jump",
                "Over 1.5× the longest session of the last 7 days (20 minutes); build up gradually"
            )
            .with("longest", 20.0)]
        );
    }
}
