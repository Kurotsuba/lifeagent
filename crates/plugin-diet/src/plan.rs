//! The diet section's plan input, per `docs/PHASE1_SPEC.md` §8.9.

use chrono::{Days, NaiveDate};
use lifeagent_core::plugin::{PlanDraft, PlanInput, PluginContext, PluginError};
use serde_json::{json, Value};

use crate::intake::{self, TRAINING_SECTION};
use crate::shared_meals::{self, SharedMeals};
use crate::targets;

pub const SECTION: &str = "diet";

/// Days of plan-vs-actual in the brief (`D−3..D−1`).
const ADHERENCE_DAYS: u64 = 3;
/// Days of planned dish titles in the brief, to encourage variety (`D−5..D−1`).
const RECENT_TITLE_DAYS: u64 = 5;

/// The JSON Schema the planner's diet section must match (§8.9).
pub fn output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["meals"],
        "properties": {
            "meals": { "type": "array", "items": {
                "type": "object",
                "required": ["meal_type", "title", "items"],
                "properties": {
                    "meal_type": { "enum": ["breakfast", "lunch", "dinner", "snack"] },
                    "title": { "type": "string" },
                    "items": { "type": "array", "items": {
                        "type": "object", "required": ["name", "grams"],
                        "properties": {
                            "name": { "type": "string" },
                            "grams": { "type": "number" }
                        } } },
                    "prep_note": { "type": "string" }
                } } },
            "tips": { "type": "array", "items": { "type": "string" } }
        }
    })
}

/// §8.9. `None` (with a warning logged) when the profile or a weight is missing, since there
/// are no targets to plan against. `prior` holds the day's training section when the training
/// plugin planned one: the targets are raised for it and the brief says when it is.
pub async fn plan_input(
    ctx: &PluginContext,
    date: NaiveDate,
    prior: &PlanDraft,
) -> Result<Option<PlanInput>, PluginError> {
    let exercise_kcal = intake::planned_exercise_kcal(prior);
    let (Some(targets), Some(profile)) = (
        intake::targets_with_exercise(ctx, date, exercise_kcal).await?,
        ctx.profile.get().await?,
    ) else {
        tracing::warn!(%date, "no diet plan input: profile or weight missing");
        return Ok(None);
    };

    let max_snacks = profile.meals_per_day.saturating_sub(2);
    let mut brief = vec![
        format!(
            "Meals: exactly 3 main meals (breakfast, lunch, dinner) plus 0–{max_snacks} snacks."
        ),
        format!(
            "Dietary pattern: {}.",
            format!("{:?}", profile.dietary_pattern).to_lowercase()
        ),
        format!(
            "Allergies (hard constraint, never include): {}.",
            list_or_none(&profile.allergies)
        ),
        format!(
            "Dislikes (avoid): {}.",
            list_or_none(&ctx.preferences.dislikes_on(date).await?)
        ),
    ];
    if let Some(line) = training_line(prior, exercise_kcal) {
        brief.push(line);
    }
    if let Some(sharing) = SharedMeals::from_draft(prior, targets.kcal) {
        brief.extend(shared_meals::brief_lines(
            &sharing,
            &profile.allergies,
            profile.dietary_pattern,
        ));
    }

    let mut adherence_lines = Vec::new();
    for day in days_before(date, ADHERENCE_DAYS) {
        if let Some(a) = intake::adherence(ctx, day).await? {
            adherence_lines.push(format!(
                "- {day}: planned {:.0} kcal / {:.0} g protein; ate {:.0} kcal / {:.0} g protein",
                a.planned_kcal, a.planned_protein, a.actual_kcal, a.actual_protein
            ));
        }
    }
    if adherence_lines.is_empty() {
        brief.push("Recent plan vs actual: no data.".into());
    } else {
        brief.push("Recent plan vs actual:".into());
        brief.extend(adherence_lines);
    }

    let mut titles = Vec::new();
    for day in days_before(date, RECENT_TITLE_DAYS) {
        titles.extend(planned_titles(ctx, day).await?);
    }
    brief.push(format!(
        "Dishes planned in the last {RECENT_TITLE_DAYS} days (vary from these): {}.",
        list_or_none(&titles)
    ));

    Ok(Some(PlanInput {
        section: SECTION.into(),
        brief: brief.join("\n"),
        targets: targets.as_macros().to_json(),
        output_schema: output_schema(),
    }))
}

/// The brief's line about the day's training, from the already-planned training section, or
/// `None` when the training plugin isn't planning (disabled, or its section is invalid).
fn training_line(prior: &PlanDraft, exercise_kcal: f64) -> Option<String> {
    let section = prior.get(TRAINING_SECTION).filter(|s| s.is_object())?;
    if section.get("rest_day").and_then(Value::as_bool) == Some(true) {
        return Some(
            "Training that day: rest day. Keep meals evenly spread; no pre- or post-workout \
             meals needed."
                .into(),
        );
    }
    let title = section.get("title").and_then(Value::as_str).unwrap_or("");
    let mut line = format!("Training that day (already planned): {title}");
    if let Some(minutes) = section.get("duration_min").and_then(Value::as_f64) {
        line.push_str(&format!(", {minutes:.0} min"));
    }
    if let Some(time) = section.get("time_of_day").and_then(Value::as_str) {
        line.push_str(&format!(", in the {time}"));
    }
    line.push('.');
    let credit = targets::exercise_credit(exercise_kcal);
    if credit > 0.0 {
        line.push_str(&format!(
            " The targets above already include {credit:.0} kcal extra (as carbs) for it."
        ));
    }
    line.push_str(
        " Time the meals around the session: a carb-containing meal or snack 1–3 h before, \
         and a protein-rich meal within 2 h after.",
    );
    Some(line)
}

/// `date − n .. date − 1`, oldest first.
fn days_before(date: NaiveDate, n: u64) -> impl Iterator<Item = NaiveDate> {
    (1..=n)
        .rev()
        .filter_map(move |k| date.checked_sub_days(Days::new(k)))
}

async fn planned_titles(ctx: &PluginContext, date: NaiveDate) -> Result<Vec<String>, PluginError> {
    let Some(plan) = ctx.plans.current(date).await? else {
        return Ok(Vec::new());
    };
    Ok(plan
        .content
        .sections
        .get(SECTION)
        .and_then(|s| s.get("meals"))
        .and_then(Value::as_array)
        .map(|meals| {
            meals
                .iter()
                .filter_map(|m| m.get("title")?.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".into()
    } else {
        items.join("、")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MealType;
    use crate::testutil::{env, log_meal_event, seed_person};
    use lifeagent_core::plan::{PlanContent, PlanTrigger};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn none_without_profile_or_weight() {
        let env = env().await;
        let today = env.ctx.today().await.unwrap();
        assert!(plan_input(&env.ctx, today, &PlanDraft::new())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn brief_has_constraints_adherence_and_recent_titles() {
        let env = env().await;
        seed_person(&env.ctx).await;
        let today = env.ctx.today().await.unwrap();
        let tomorrow = today.succ_opt().unwrap();
        // Today has a plan and a logged meal: adherence for today, one of D−3..D−1.
        env.ctx
            .plans
            .insert(
                today,
                PlanTrigger::Schedule,
                None,
                &PlanContent {
                    summary: "s".into(),
                    sections: BTreeMap::from([(
                        "diet".to_string(),
                        json!({
                            "meals": [{"meal_type": "lunch", "title": "番茄炒蛋饭"}],
                            "total": {"kcal": 1800, "protein_g": 100}
                        }),
                    )]),
                    disclaimer: None,
                },
                &[],
            )
            .await
            .unwrap();
        log_meal_event(&env.ctx, MealType::Lunch, 600.0, 30.0).await;

        let input = plan_input(&env.ctx, tomorrow, &PlanDraft::new())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(input.section, "diet");
        assert!(input.targets["kcal"].as_f64().unwrap() > 0.0);
        assert!(input.brief.contains("plus 0–1 snacks"));
        assert!(input
            .brief
            .contains("Allergies (hard constraint, never include): none."));
        assert!(input.brief.contains(&format!(
            "- {today}: planned 1800 kcal / 100 g protein; ate 600 kcal / 30 g protein"
        )));
        assert!(input.brief.contains("(vary from these): 番茄炒蛋饭."));
        assert_eq!(input.output_schema, output_schema());
        assert!(!input.brief.contains("Training that day"));
    }

    #[tokio::test]
    async fn a_planned_session_raises_the_targets_and_is_in_the_brief() {
        let env = env().await;
        seed_person(&env.ctx).await;
        let tomorrow = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let base = plan_input(&env.ctx, tomorrow, &PlanDraft::new())
            .await
            .unwrap()
            .unwrap();
        let prior = PlanDraft::from([(
            "training".to_string(),
            json!({
                "rest_day": false, "title": "下肢力量", "time_of_day": "evening",
                "duration_min": 50.0, "exercise_kcal": 300.0
            }),
        )]);

        let input = plan_input(&env.ctx, tomorrow, &prior)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            input.targets["kcal"].as_f64().unwrap(),
            base.targets["kcal"].as_f64().unwrap() + 150.0
        );
        assert!(input.brief.contains(
            "Training that day (already planned): 下肢力量, 50 min, in the evening. The targets \
             above already include 150 kcal extra (as carbs) for it."
        ));

        let rest = PlanDraft::from([("training".to_string(), json!({"rest_day": true}))]);
        let input = plan_input(&env.ctx, tomorrow, &rest)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(input.targets, base.targets);
        assert!(input.brief.contains("Training that day: rest day."));
    }

    #[test]
    fn days_before_is_oldest_first() {
        let d = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let days: Vec<String> = days_before(d, 3).map(|d| d.to_string()).collect();
        assert_eq!(days, vec!["2026-09-22", "2026-09-23", "2026-09-24"]);
    }
}
