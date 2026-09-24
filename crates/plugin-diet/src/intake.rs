//! Daily intake: logged meals, totals vs targets, the today card, the context snippet and
//! plan-vs-actual, per `docs/PHASE1_SPEC.md` §8.6–§8.8 and §8.11.

use std::collections::HashSet;

use chrono::{DateTime, Local, NaiveDate, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_core::time::age_years;
use serde::Serialize;
use serde_json::{json, Value};

use crate::model::{round1, Macros, MealLogged, MealType};
use crate::targets::{self, NutritionTargets};

pub const MEAL_EVENT: &str = "diet.meal_logged";

/// A logged meal: the stored `MealLogged` payload plus its event metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedMeal {
    pub entry_id: String,
    pub occurred_at: DateTime<Utc>,
    pub meal: MealLogged,
}

impl LoggedMeal {
    /// `{entry_id, meal_type, time, items, total}`, as the today card and `diet_get_intake`
    /// list meals.
    pub fn to_json(&self) -> Value {
        json!({
            "entry_id": self.entry_id,
            "meal_type": self.meal.meal_type,
            "time": self.occurred_at.with_timezone(&Local).format("%H:%M").to_string(),
            "items": self.meal.items.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
            "total": self.meal.total.to_json(),
        })
    }
}

/// Planned vs logged energy and protein for one day (§8.11).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Adherence {
    pub planned_kcal: f64,
    pub actual_kcal: f64,
    pub planned_protein: f64,
    pub actual_protein: f64,
}

pub async fn today(ctx: &PluginContext) -> Result<NaiveDate, PluginError> {
    Ok(ctx.today().await?)
}

pub async fn meals_on(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Vec<LoggedMeal>, PluginError> {
    ctx.events
        .list(MEAL_EVENT, date, date)
        .await?
        .into_iter()
        .map(|event| {
            let meal = serde_json::from_value(event.payload).map_err(|e| {
                PluginError::Other(format!("bad {MEAL_EVENT} payload for {}: {e}", event.id))
            })?;
            Ok(LoggedMeal {
                entry_id: event.id,
                occurred_at: event.occurred_at,
                meal,
            })
        })
        .collect()
}

pub fn total(meals: &[LoggedMeal]) -> Macros {
    meals.iter().map(|m| m.meal.total).sum()
}

/// Targets for `date` from the profile, latest weight and active goal, or `None` if the
/// profile or a weight is missing (§8.6).
pub async fn targets_for(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<NutritionTargets>, PluginError> {
    let Some(profile) = ctx.profile.get().await? else {
        return Ok(None);
    };
    let Some(weight) = ctx
        .events
        .latest("body.weight_logged")
        .await?
        .and_then(|e| e.payload.get("kg").and_then(Value::as_f64))
    else {
        return Ok(None);
    };
    let goal = ctx.goals.get_active_weight_goal().await?;
    let age = age_years(profile.birth_date, date);
    Ok(Some(targets::compute(&profile, weight, age, goal.as_ref())))
}

/// The diet section of the current plan for `date`, if any.
async fn planned_section(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<Value>, PluginError> {
    Ok(ctx
        .plans
        .current(date)
        .await?
        .and_then(|plan| plan.content.sections.get("diet").cloned()))
}

/// §8.11: the current plan version for `date` vs logged totals, or `None` without a plan
/// that has a diet total.
pub async fn adherence(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<Adherence>, PluginError> {
    let Some(section) = planned_section(ctx, date).await? else {
        return Ok(None);
    };
    let planned = |field: &str| {
        section
            .get("total")
            .and_then(|t| t.get(field))
            .and_then(Value::as_f64)
    };
    let (Some(planned_kcal), Some(planned_protein)) = (planned("kcal"), planned("protein_g"))
    else {
        return Ok(None);
    };
    let actual = total(&meals_on(ctx, date).await?);
    Ok(Some(Adherence {
        planned_kcal: planned_kcal.round(),
        actual_kcal: actual.kcal.round(),
        planned_protein: round1(planned_protein),
        actual_protein: round1(actual.protein_g),
    }))
}

/// `{total, target, remaining}` for a day's meals.
fn totals_json(meals_total: &Macros, target: Option<&NutritionTargets>) -> (Value, Value, Value) {
    let target_json = target.map_or(Value::Null, |t| t.as_macros().to_json());
    let remaining = target.map_or(Value::Null, |t| (t.as_macros() - *meals_total).to_json());
    (meals_total.to_json(), target_json, remaining)
}

/// §8.8 today card: `{date, total, target, meals}`.
pub async fn today_card(ctx: &PluginContext, date: NaiveDate) -> Result<Value, PluginError> {
    let meals = meals_on(ctx, date).await?;
    let target = targets_for(ctx, date).await?;
    let (total, target, _) = totals_json(&total(&meals), target.as_ref());
    Ok(json!({
        "date": date.to_string(),
        "total": total,
        "target": target,
        "meals": meals.iter().map(LoggedMeal::to_json).collect::<Vec<_>>(),
    }))
}

/// `diet_get_intake` content: `{date, total, target, remaining, meals}`, plus `adherence` for
/// past dates (§8.5, §8.11).
pub async fn intake_json(ctx: &PluginContext, date: NaiveDate) -> Result<Value, PluginError> {
    let meals = meals_on(ctx, date).await?;
    let target = targets_for(ctx, date).await?;
    let (total, target, remaining) = totals_json(&total(&meals), target.as_ref());
    let mut out = json!({
        "date": date.to_string(),
        "total": total,
        "target": target,
        "remaining": remaining,
        "meals": meals.iter().map(LoggedMeal::to_json).collect::<Vec<_>>(),
    });
    if date < today(ctx).await? {
        out["adherence"] = json!(adherence(ctx, date).await?);
    }
    Ok(out)
}

/// §8.7 context snippet: today's totals vs targets, logged meals with entry ids (so the model
/// can delete one), and planned meals not yet eaten.
pub async fn context_snippet(ctx: &PluginContext, date: NaiveDate) -> Result<String, PluginError> {
    let meals = meals_on(ctx, date).await?;
    let t = total(&meals);
    let target = targets_for(ctx, date).await?;

    let totals_line = match target {
        Some(g) => format!(
            "今日摄入：{:.0} / {:.0} kcal；蛋白质 {:.0} / {:.0} g；碳水 {:.0} / {:.0} g；脂肪 {:.0} / {:.0} g",
            t.kcal, g.kcal, t.protein_g, g.protein_g, t.carbs_g, g.carbs_g, t.fat_g, g.fat_g
        ),
        None => format!(
            "今日摄入：{:.0} kcal；蛋白质 {:.0} g；碳水 {:.0} g；脂肪 {:.0} g（暂无目标：缺少档案或体重）",
            t.kcal, t.protein_g, t.carbs_g, t.fat_g
        ),
    };

    let logged = if meals.is_empty() {
        "无".to_string()
    } else {
        meals
            .iter()
            .map(|m| {
                let names: Vec<&str> = m.meal.items.iter().map(|i| i.name.as_str()).collect();
                format!(
                    "{}(entry {}) {} {:.0} kcal",
                    m.meal.meal_type.zh_label(),
                    m.entry_id,
                    names.join("、"),
                    m.meal.total.kcal
                )
            })
            .collect::<Vec<_>>()
            .join("；")
    };

    let mut lines = vec![totals_line, format!("已记录：{logged}")];
    let not_eaten = planned_not_eaten(ctx, date, &meals).await?;
    if !not_eaten.is_empty() {
        lines.push(format!("今日计划中尚未吃的餐：{}", not_eaten.join("；")));
    }
    Ok(lines.join("\n"))
}

/// Planned meals whose meal type has no logged meal yet, as "晚餐 鸡胸肉糙米饭".
async fn planned_not_eaten(
    ctx: &PluginContext,
    date: NaiveDate,
    meals: &[LoggedMeal],
) -> Result<Vec<String>, PluginError> {
    let Some(section) = planned_section(ctx, date).await? else {
        return Ok(Vec::new());
    };
    let logged: HashSet<MealType> = meals.iter().map(|m| m.meal.meal_type).collect();
    let planned = section
        .get("meals")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(planned
        .iter()
        .filter_map(|meal| {
            let meal_type = MealType::parse(meal.get("meal_type")?.as_str()?)?;
            let title = meal.get("title")?.as_str()?;
            (!logged.contains(&meal_type)).then(|| format!("{} {title}", meal_type.zh_label()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, log_meal_event, seed_person};
    use lifeagent_core::plan::{PlanContent, PlanTrigger};
    use std::collections::BTreeMap;

    fn diet_plan(meals: Value, kcal: f64, protein_g: f64) -> PlanContent {
        PlanContent {
            summary: "测试计划".into(),
            sections: BTreeMap::from([(
                "diet".to_string(),
                json!({ "meals": meals, "total": { "kcal": kcal, "protein_g": protein_g } }),
            )]),
            disclaimer: None,
        }
    }

    #[tokio::test]
    async fn today_card_totals_meals_and_has_null_target_without_a_profile() {
        let env = env().await;
        let day = today(&env.ctx).await.unwrap();
        log_meal_event(&env.ctx, MealType::Breakfast, 420.0, 15.0).await;
        log_meal_event(&env.ctx, MealType::Lunch, 900.0, 40.0).await;

        let card = today_card(&env.ctx, day).await.unwrap();

        assert_eq!(card["total"]["kcal"], 1320.0);
        assert_eq!(card["total"]["protein_g"], 55.0);
        assert_eq!(card["target"], Value::Null);
        assert_eq!(card["meals"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn intake_includes_target_and_remaining_once_profile_and_weight_exist() {
        let env = env().await;
        seed_person(&env.ctx).await;
        let day = today(&env.ctx).await.unwrap();
        log_meal_event(&env.ctx, MealType::Lunch, 500.0, 30.0).await;

        let intake = intake_json(&env.ctx, day).await.unwrap();

        let target_kcal = intake["target"]["kcal"].as_f64().unwrap();
        assert_eq!(
            intake["remaining"]["kcal"].as_f64().unwrap(),
            target_kcal - 500.0
        );
        // No adherence for today — only past dates carry it.
        assert!(intake.get("adherence").is_none());
    }

    #[tokio::test]
    async fn context_snippet_lists_meals_with_entry_ids_and_uneaten_planned_meals() {
        let env = env().await;
        let day = today(&env.ctx).await.unwrap();
        let entry = log_meal_event(&env.ctx, MealType::Breakfast, 420.0, 15.0).await;
        env.ctx
            .plans
            .insert(
                day,
                PlanTrigger::Manual,
                None,
                &diet_plan(
                    json!([
                        {"meal_type": "breakfast", "title": "燕麦牛奶"},
                        {"meal_type": "dinner", "title": "鸡胸肉糙米饭"}
                    ]),
                    2000.0,
                    120.0,
                ),
                &[],
            )
            .await
            .unwrap();

        let snippet = context_snippet(&env.ctx, day).await.unwrap();

        assert!(snippet.contains("今日摄入：420 kcal"));
        assert!(snippet.contains(&format!("早餐(entry {entry})")));
        // Breakfast was logged, so only dinner remains.
        assert!(snippet.ends_with("今日计划中尚未吃的餐：晚餐 鸡胸肉糙米饭"));
    }

    #[tokio::test]
    async fn context_snippet_without_meals_or_plan() {
        let env = env().await;
        let day = today(&env.ctx).await.unwrap();

        let snippet = context_snippet(&env.ctx, day).await.unwrap();

        assert!(snippet.contains("已记录：无"));
        assert!(!snippet.contains("今日计划"));
    }

    #[tokio::test]
    async fn adherence_compares_the_plan_total_with_logged_meals() {
        let env = env().await;
        let day = today(&env.ctx).await.unwrap();
        log_meal_event(&env.ctx, MealType::Lunch, 700.0, 35.0).await;
        assert_eq!(adherence(&env.ctx, day).await.unwrap(), None);

        env.ctx
            .plans
            .insert(
                day,
                PlanTrigger::Manual,
                None,
                &diet_plan(json!([]), 2000.0, 120.0),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(
            adherence(&env.ctx, day).await.unwrap(),
            Some(Adherence {
                planned_kcal: 2000.0,
                actual_kcal: 700.0,
                planned_protein: 120.0,
                actual_protein: 35.0,
            })
        );
    }

    #[tokio::test]
    async fn intake_for_a_past_date_carries_adherence() {
        let env = env().await;
        let day = today(&env.ctx).await.unwrap();
        let yesterday = day.pred_opt().unwrap();

        let intake = intake_json(&env.ctx, yesterday).await.unwrap();

        // No plan for yesterday: present, but null.
        assert_eq!(intake["adherence"], Value::Null);
    }
}
