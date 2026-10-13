//! The diet plugin's proactive follow-ups (`lifeagent_core::follow_ups`, `docs/DECISIONS.md`
//! "Proactive follow-ups"). Both checks are plain code over logged meals, the plan and the
//! targets; no model decides whether to ask.
//!
//! - `diet_meal_missed:<meal>`: today's plan has the meal, its check time has passed, and no
//!   meal of that type is logged. Answers: log it as planned (`diet_log_planned`, numbers from
//!   the food table, no model call), reply in chat, or skipped (`diet_skipped`).
//! - `diet_day_review`: at the evening check time, once every main meal is logged or skipped,
//!   the day's energy or protein is far off its target. Answers: adjust tomorrow's plan
//!   (`core_adjust_tomorrow`, with an instruction built here) or not now.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use lifeagent_core::error::{CoreError, ValidationRule};
use lifeagent_core::events::EventSource;
use lifeagent_core::follow_ups::{
    FollowUp, FollowUpCandidate, ANSWER_ADJUST_TOMORROW, ANSWER_DISMISS, ANSWER_REPLY,
};
use lifeagent_core::plugin::{PluginContext, PluginError, ToolSummary};
use lifeagent_core::time::parse_hhmm;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::intake::{self, MEAL_EVENT};
use crate::model::{round1, ItemKind, MealItem, MealLogged, MealType};
use crate::plan::SECTION;
use crate::resolver::{resolve_many, FoodQuery};

pub const MEAL_MISSED: &str = "diet_meal_missed";
pub const DAY_REVIEW: &str = "diet_day_review";
pub const DAY_OFF_TARGET: &str = "diet_day_off_target";
pub const ANSWER_LOG_PLANNED: &str = "diet_log_planned";
pub const ANSWER_SKIPPED: &str = "diet_skipped";

const MAIN_MEALS: [MealType; 3] = [MealType::Breakfast, MealType::Lunch, MealType::Dinner];

/// The day's energy is off when it misses its target by this share or [`KCAL_OFF_MIN`],
/// whichever is larger.
const KCAL_OFF_SHARE: f64 = 0.15;
const KCAL_OFF_MIN: f64 = 300.0;
/// Protein is short when under its target by this share or [`PROTEIN_SHORT_MIN`] g, whichever is
/// larger.
const PROTEIN_SHORT_SHARE: f64 = 0.2;
const PROTEIN_SHORT_MIN: f64 = 20.0;

/// The settings key the check times are stored under.
const TIMES_KEY: &str = "diet_follow_up_times";

/// Local times on the logical day when each check runs, `HH:MM`. Defaults: a planned meal is
/// asked about well after its usual time, and the evening review comes after dinner and after
/// the default `plan_time` (21:00), so tomorrow's plan exists to adjust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUpTimes {
    pub breakfast: String,
    pub lunch: String,
    pub dinner: String,
    pub evening: String,
}

impl Default for FollowUpTimes {
    fn default() -> Self {
        Self {
            breakfast: "10:30".into(),
            lunch: "14:30".into(),
            dinner: "20:30".into(),
            evening: "21:30".into(),
        }
    }
}

impl FollowUpTimes {
    fn meal(&self, meal: MealType) -> &str {
        match meal {
            MealType::Breakfast => &self.breakfast,
            MealType::Lunch => &self.lunch,
            MealType::Dinner | MealType::Snack => &self.dinner,
        }
    }

    /// Every time parses, and none is before `day_boundary`: such a time falls on the next
    /// calendar morning, which the checks (comparing on the logical day itself) would read as
    /// the start of the same day. Like `plan_time`'s rule (`docs/PHASE1_SPEC.md` §7.2).
    pub fn validate(&self, day_boundary: NaiveTime) -> Result<(), CoreError> {
        for t in [&self.breakfast, &self.lunch, &self.dinner, &self.evening] {
            let time = parse_hhmm(t)?;
            if time < day_boundary {
                return Err(CoreError::InvalidInput(
                    ValidationRule::CheckTimeBeforeDayBoundary { time, day_boundary },
                ));
            }
        }
        Ok(())
    }
}

fn parse_time(s: &str) -> Result<NaiveTime, PluginError> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .map_err(|e| PluginError::InvalidArgs(format!("invalid time {s:?}: {e}")))
}

pub async fn times(ctx: &PluginContext) -> Result<FollowUpTimes, PluginError> {
    Ok(ctx
        .settings
        .get_json::<FollowUpTimes>(TIMES_KEY)
        .await?
        .unwrap_or_default())
}

/// Errors with `CoreError::InvalidInput` for a time before the day boundary, so the UI can word
/// it.
pub async fn save_times(ctx: &PluginContext, times: &FollowUpTimes) -> Result<(), CoreError> {
    times.validate(ctx.settings.day_boundary().await?)?;
    ctx.settings
        .set_json(TIMES_KEY, times, ctx.clock.now())
        .await?;
    Ok(())
}

/// Whether `time` has passed on logical day `date`. Like the scheduler's `is_due`, compared on
/// `date` itself, never the calendar date of `now_local`.
fn passed(now_local: NaiveDateTime, date: NaiveDate, time: &str) -> Result<bool, PluginError> {
    Ok(now_local >= date.and_time(parse_time(time)?))
}

fn meal_key(meal: MealType) -> String {
    format!("{MEAL_MISSED}:{}", meal.as_str())
}

/// Whether the user answered `meal`'s follow-up today with "skipped".
fn skipped(raised: &[FollowUp], meal: MealType) -> bool {
    let key = meal_key(meal);
    raised
        .iter()
        .any(|f| f.key == key && f.answer.as_deref() == Some(ANSWER_SKIPPED))
}

/// The planned meals of today's diet section, as stored.
async fn planned_meals(ctx: &PluginContext, date: NaiveDate) -> Result<Vec<Value>, PluginError> {
    Ok(ctx
        .plans
        .current(date)
        .await?
        .and_then(|plan| plan.content.sections.get(SECTION).cloned())
        .and_then(|section| section.get("meals").and_then(Value::as_array).cloned())
        .unwrap_or_default())
}

fn meal_type_of(meal: &Value) -> Option<MealType> {
    meal.get("meal_type")
        .and_then(Value::as_str)
        .and_then(MealType::parse)
}

pub async fn check(
    ctx: &PluginContext,
    date: NaiveDate,
    now_local: NaiveDateTime,
    raised: &[FollowUp],
) -> Result<Vec<FollowUpCandidate>, PluginError> {
    let times = times(ctx).await?;
    let logged = intake::meals_on(ctx, date).await?;
    let is_logged = |meal: MealType| logged.iter().any(|m| m.meal.meal_type == meal);
    let mut out = Vec::new();

    let planned = planned_meals(ctx, date).await?;
    for meal in MAIN_MEALS {
        if is_logged(meal) || skipped(raised, meal) || !passed(now_local, date, times.meal(meal))? {
            continue;
        }
        let Some(plan_meal) = planned.iter().find(|m| meal_type_of(m) == Some(meal)) else {
            continue;
        };
        let title = plan_meal.get("title").and_then(Value::as_str).unwrap_or("");
        let kcal = plan_meal
            .get("total")
            .and_then(|t| t.get("kcal"))
            .and_then(Value::as_f64);
        out.push(FollowUpCandidate {
            key: meal_key(meal),
            kind: MEAL_MISSED.into(),
            params: json!({
                "meal_type": meal,
                "title": title,
                "kcal": kcal,
                "check_time": times.meal(meal),
            }),
            answers: vec![
                ANSWER_LOG_PLANNED.into(),
                ANSWER_REPLY.into(),
                ANSWER_SKIPPED.into(),
            ],
            reason: format!(
                "planned {} not logged by {}",
                meal.as_str(),
                times.meal(meal)
            ),
            instruction: None,
        });
    }

    if passed(now_local, date, &times.evening)?
        && !logged.is_empty()
        && MAIN_MEALS
            .iter()
            .all(|&m| is_logged(m) || skipped(raised, m))
    {
        if let Some(target) = intake::targets_for(ctx, date).await? {
            let total = intake::total(&logged);
            let has_plan_tomorrow = match date.succ_opt() {
                Some(tomorrow) => ctx.plans.current(tomorrow).await?.is_some(),
                None => false,
            };
            out.extend(day_review(
                date,
                total.kcal,
                total.protein_g,
                target.kcal,
                target.protein_g,
                has_plan_tomorrow,
            ));
        }
    }
    Ok(out)
}

/// The evening review for a day that ate `kcal`/`protein` against its targets: a candidate when
/// either is far off, with the planner instruction for adjusting tomorrow.
fn day_review(
    date: NaiveDate,
    kcal: f64,
    protein: f64,
    target_kcal: f64,
    target_protein: f64,
    has_plan_tomorrow: bool,
) -> Option<FollowUpCandidate> {
    let kcal_diff = (kcal - target_kcal).round();
    let protein_diff = round1(protein - target_protein);
    let kcal_off = kcal_diff.abs() >= (target_kcal * KCAL_OFF_SHARE).max(KCAL_OFF_MIN);
    let protein_short =
        -protein_diff >= (target_protein * PROTEIN_SHORT_SHARE).max(PROTEIN_SHORT_MIN);
    if !kcal_off && !protein_short {
        return None;
    }

    let mut issues = Vec::new();
    let mut asks = Vec::new();
    if kcal_off && kcal_diff > 0.0 {
        issues.push("kcal_over");
        asks.push(
            "choose lighter, lower-fat dishes with plenty of vegetables, so the day feels \
             satisfying without going over its energy target",
        );
    } else if kcal_off {
        issues.push("kcal_under");
        asks.push("make every meal substantial enough that the day reaches its energy target");
    }
    if protein_short {
        issues.push("protein_under");
        asks.push("put a clear protein source in every meal");
    }
    let instruction = format!(
        "Today ({date}) the user ate {kcal:.0} kcal against a target of {target_kcal:.0} kcal \
         ({kcal_diff:+.0}) and {protein:.0} g protein against {target_protein:.0} g \
         ({protein_diff:+.0} g), as computed by the app. Adjust tomorrow's diet to rebalance \
         gently: stay within tomorrow's own targets (never cut below them to make up for today), \
         and {}. Keep everything else as it was.",
        asks.join("; and ")
    );
    Some(FollowUpCandidate {
        key: DAY_REVIEW.into(),
        kind: DAY_OFF_TARGET.into(),
        params: json!({
            "issues": issues,
            "kcal": kcal.round(),
            "target_kcal": target_kcal.round(),
            "kcal_diff": kcal_diff,
            "protein_g": round1(protein),
            "target_protein_g": round1(target_protein),
            "protein_diff_g": protein_diff,
            "has_plan_tomorrow": has_plan_tomorrow,
        }),
        answers: vec![ANSWER_ADJUST_TOMORROW.into(), ANSWER_DISMISS.into()],
        reason: format!(
            "day off target: {kcal_diff:+.0} kcal, {protein_diff:+.0} g protein ({})",
            issues.join(", ")
        ),
        instruction: Some(instruction),
    })
}

pub async fn answer(
    ctx: &PluginContext,
    follow_up: &FollowUp,
    answer: &str,
) -> Result<ToolSummary, PluginError> {
    let meal = follow_up
        .key
        .strip_prefix(&format!("{MEAL_MISSED}:"))
        .and_then(MealType::parse)
        .ok_or_else(|| {
            PluginError::InvalidArgs(format!("{answer:?} does not answer {:?}", follow_up.key))
        })?;
    match answer {
        ANSWER_LOG_PLANNED => log_planned(ctx, follow_up.logical_date, meal).await,
        ANSWER_SKIPPED => Ok(ToolSummary::new("diet_meal_skipped").with("meal_type", meal)),
        other => Err(PluginError::InvalidArgs(format!(
            "unknown follow-up answer {other:?}"
        ))),
    }
}

/// Logs `meal` of `date`'s current plan as eaten, now: the planned foods and grams, with
/// nutrition from the food table like the plan's own numbers. No model call unless a planned
/// food has since gone from the table.
async fn log_planned(
    ctx: &PluginContext,
    date: NaiveDate,
    meal: MealType,
) -> Result<ToolSummary, PluginError> {
    let planned = planned_meals(ctx, date).await?;
    let plan_meal = planned
        .iter()
        .find(|m| meal_type_of(m) == Some(meal))
        .ok_or_else(|| PluginError::NotFound(format!("planned {} on {date}", meal.as_str())))?;
    let title = plan_meal
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(meal.as_str())
        .to_string();
    let items: Vec<(String, f64)> = plan_meal
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| {
                    Some((
                        i.get("name")?.as_str()?.to_string(),
                        i.get("grams")?.as_f64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() {
        return Err(PluginError::NotFound(format!(
            "items of the planned {}",
            meal.as_str()
        )));
    }

    let queries: Vec<FoodQuery> = items.iter().map(|(name, _)| FoodQuery::new(name)).collect();
    let foods = resolve_many(ctx, &queries).await?;
    let items: Vec<MealItem> = foods
        .into_iter()
        .zip(items)
        .map(|(food, (_, grams))| MealItem {
            macros: food.per_100g.for_grams(grams),
            name: food.name,
            kind: ItemKind::Food,
            brand: None,
            grams: Some(grams),
            servings: None,
            food_id: Some(food.id),
            dish_id: None,
            source: food.source_kind,
            source_id: Some(food.source_id),
            source_name: Some(food.source_name),
            eaten_out: false,
        })
        .collect();
    let logged = MealLogged {
        meal_type: meal,
        raw_text: title,
        total: items.iter().map(|i| i.macros).sum(),
        items,
    };
    let payload = serde_json::to_value(&logged).map_err(|e| PluginError::Other(e.to_string()))?;
    ctx.events
        .append(MEAL_EVENT, ctx.clock.now(), payload, EventSource::Ui)
        .await?;
    Ok(ToolSummary::new("diet_meal_logged")
        .with("meal_type", meal)
        .with("kcal", logged.total.kcal.round()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{self, local_instant, seed_person, RICE};
    use lifeagent_core::follow_ups::FollowUpStatus;
    use lifeagent_core::plan::{PlanContent, PlanTrigger};
    use std::collections::BTreeMap;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 24).unwrap()
    }

    fn at(h: u32, m: u32) -> NaiveDateTime {
        day().and_hms_opt(h, m, 0).unwrap()
    }

    async fn save_plan(ctx: &PluginContext) {
        let meal = |meal_type: &str, title: &str| {
            json!({
                "meal_type": meal_type, "title": title,
                "items": [{"name": "熟米饭", "grams": 200.0}],
                "total": {"kcal": 232.0},
            })
        };
        let mut sections = BTreeMap::new();
        sections.insert(
            SECTION.to_string(),
            json!({"meals": [
                meal("breakfast", "米饭早餐"),
                meal("lunch", "米饭午餐"),
                meal("dinner", "米饭晚餐"),
            ]}),
        );
        ctx.plans
            .insert(
                day(),
                PlanTrigger::Schedule,
                None,
                &PlanContent {
                    summary: String::new(),
                    sections,
                    disclaimer: None,
                },
                &[],
            )
            .await
            .unwrap();
    }

    fn follow_up(key: &str, answer: Option<&str>) -> FollowUp {
        FollowUp {
            id: "f".into(),
            logical_date: day(),
            key: key.into(),
            kind: MEAL_MISSED.into(),
            params: json!({}),
            answers: vec![],
            reason: String::new(),
            instruction: None,
            status: if answer.is_some() {
                FollowUpStatus::Answered
            } else {
                FollowUpStatus::Open
            },
            answer: answer.map(Into::into),
            result: None,
            run_id: None,
            created_at: chrono::Utc::now(),
            resolved_at: None,
        }
    }

    #[tokio::test]
    async fn asks_about_planned_meals_not_logged_by_their_check_time() {
        let env = testutil::env().await;
        let ctx = &env.ctx;
        save_plan(ctx).await;
        env.clock.set(local_instant(15, 0));

        let found = check(ctx, day(), at(15, 0), &[]).await.unwrap();
        let keys: Vec<&str> = found.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            ["diet_meal_missed:breakfast", "diet_meal_missed:lunch"]
        );
        assert_eq!(found[1].params["title"], "米饭午餐");
        assert_eq!(found[1].answers[0], ANSWER_LOG_PLANNED);

        // Logged breakfast, and lunch answered "skipped": nothing left to ask before dinner.
        testutil::log_meal_event(ctx, MealType::Breakfast, 300.0, 10.0).await;
        let raised = [follow_up("diet_meal_missed:lunch", Some(ANSWER_SKIPPED))];
        assert!(check(ctx, day(), at(15, 0), &raised)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn no_meal_follow_ups_without_a_plan() {
        let env = testutil::env().await;
        assert!(check(&env.ctx, day(), at(23, 0), &[])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn logs_the_planned_meal_with_table_numbers() {
        let env = testutil::env().await;
        let ctx = &env.ctx;
        testutil::add_dataset(&env, "test", &[("熟米饭", RICE)]).await;
        save_plan(ctx).await;
        env.clock.set(local_instant(15, 0));

        let summary = answer(
            ctx,
            &follow_up("diet_meal_missed:lunch", None),
            ANSWER_LOG_PLANNED,
        )
        .await
        .unwrap();
        assert_eq!(summary.kind(), "diet_meal_logged");
        assert_eq!(summary.0["kcal"], (RICE.kcal * 2.0).round());
        let meals = intake::meals_on(ctx, day()).await.unwrap();
        assert_eq!(meals.len(), 1);
        assert_eq!(meals[0].meal.meal_type, MealType::Lunch);
        assert_eq!(meals[0].meal.raw_text, "米饭午餐");
        assert_eq!(meals[0].meal.items[0].grams, Some(200.0));
        // The model was never called.
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn evening_review_once_main_meals_are_in_and_the_day_is_off() {
        let env = testutil::env().await;
        let ctx = &env.ctx;
        seed_person(ctx).await;
        env.clock.set(local_instant(22, 0));
        for meal in MAIN_MEALS {
            testutil::log_meal_event(ctx, meal, 900.0, 15.0).await;
        }
        let found = check(ctx, day(), at(22, 0), &[]).await.unwrap();
        assert_eq!(found.len(), 1);
        let review = &found[0];
        assert_eq!(review.key, DAY_REVIEW);
        assert_eq!(
            review.params["issues"],
            json!(["kcal_over", "protein_under"])
        );
        assert_eq!(review.answers, [ANSWER_ADJUST_TOMORROW, ANSWER_DISMISS]);
        let instruction = review.instruction.as_deref().unwrap();
        assert!(instruction.contains("2700 kcal"));
        assert!(instruction.contains("protein source"));

        // Before the evening check time, nothing.
        assert!(check(ctx, day(), at(21, 0), &[]).await.unwrap().is_empty());
    }

    #[test]
    fn day_review_thresholds() {
        // 200 kcal over a 1800 target and 10 g short of 100: within both limits.
        assert!(day_review(day(), 2000.0, 90.0, 1800.0, 100.0, true).is_none());
        // 400 kcal under: off.
        let under = day_review(day(), 1400.0, 95.0, 1800.0, 100.0, false).unwrap();
        assert_eq!(under.params["issues"], json!(["kcal_under"]));
        assert_eq!(under.params["has_plan_tomorrow"], false);
        // 25 g short of 100 g protein: short.
        let short = day_review(day(), 1800.0, 75.0, 1800.0, 100.0, true).unwrap();
        assert_eq!(short.params["issues"], json!(["protein_under"]));
    }

    #[tokio::test]
    async fn check_times_round_trip_and_reject_bad_input() {
        let env = testutil::env().await;
        let ctx = &env.ctx;
        assert_eq!(times(ctx).await.unwrap(), FollowUpTimes::default());
        let mut t = FollowUpTimes {
            lunch: "13:45".into(),
            ..FollowUpTimes::default()
        };
        save_times(ctx, &t).await.unwrap();
        assert_eq!(times(ctx).await.unwrap().lunch, "13:45");
        t.dinner = "25:00".into();
        assert!(save_times(ctx, &t).await.is_err());
        // 01:00 is before the default 04:00 day boundary: the next calendar morning.
        t.dinner = "01:00".into();
        assert!(matches!(
            save_times(ctx, &t).await,
            Err(CoreError::InvalidInput(
                ValidationRule::CheckTimeBeforeDayBoundary { .. }
            ))
        ));
        assert_eq!(times(ctx).await.unwrap().dinner, "20:30");
    }
}
