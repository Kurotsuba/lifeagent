//! The diet section's plan input, per `docs/PHASE1_SPEC.md` §8.9.

use chrono::{Days, NaiveDate};
use lifeagent_core::plugin::{PlanInput, PluginContext, PluginError};
use serde_json::{json, Value};

use crate::intake;

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
/// are no targets to plan against.
pub async fn plan_input(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<PlanInput>, PluginError> {
    let (Some(targets), Some(profile)) = (
        intake::targets_for(ctx, date).await?,
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
        assert!(plan_input(&env.ctx, today).await.unwrap().is_none());
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

        let input = plan_input(&env.ctx, tomorrow).await.unwrap().unwrap();

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
    }

    #[test]
    fn days_before_is_oldest_first() {
        let d = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let days: Vec<String> = days_before(d, 3).map(|d| d.to_string()).collect();
        assert_eq!(days, vec!["2026-09-22", "2026-09-23", "2026-09-24"]);
    }
}
