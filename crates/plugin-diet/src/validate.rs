//! Finalize and validate the diet plan section, per `docs/PHASE1_SPEC.md` §8.10.
//!
//! Violations trigger one planner retry and, if still unmet, are shown to the user as
//! warnings; warnings are only shown. Both are therefore written in zh-CN.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PlanDraft, PlanSection, PluginContext, PluginError};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::intake;
use crate::model::{ItemKind, Macros, MealItem, MealType};
use crate::resolver::{resolve_many, FoodQuery};
use crate::shared_meals::{self, SharedMeals};

const KCAL_TOLERANCE: f64 = 0.08;
const MIN_PROTEIN_SHARE: f64 = 0.9;
const MACRO_TOLERANCE: f64 = 0.2;
const MIN_FIBER_SHARE: f64 = 0.7;
const MIN_ITEM_GRAMS: f64 = 1.0;
const MAX_ITEM_GRAMS: f64 = 1500.0;

#[derive(Deserialize)]
struct PlannedSection {
    meals: Vec<PlannedMeal>,
    #[serde(default)]
    tips: Vec<String>,
}

#[derive(Deserialize)]
struct PlannedMeal {
    meal_type: MealType,
    title: String,
    items: Vec<PlannedItem>,
    #[serde(default)]
    prep_note: Option<String>,
}

#[derive(Deserialize)]
struct PlannedItem {
    name: String,
    grams: f64,
}

fn invalid(reason: String) -> PlanSection {
    PlanSection {
        data: Value::Null,
        warnings: Vec::new(),
        violations: vec![format!("结构无效：{reason}")],
    }
}

/// Case-insensitive substring match in either direction (§8.10), so "虾" flags "虾仁" and
/// "花生油" flags "花生".
fn matches_term(name: &str, term: &str) -> bool {
    let name = name.trim().to_lowercase();
    let term = term.trim().to_lowercase();
    !name.is_empty() && !term.is_empty() && (name.contains(&term) || term.contains(&name))
}

fn outside(actual: f64, target: f64, tolerance: f64) -> bool {
    (actual - target).abs() > tolerance * target
}

/// `prior` is the day's already-planned training, if any: the targets checked against are
/// raised for it exactly as `plan::plan_input` raised the ones the planner was given.
pub async fn finalize(
    ctx: &PluginContext,
    date: NaiveDate,
    raw: Value,
    prior: &PlanDraft,
) -> Result<PlanSection, PluginError> {
    let planned: PlannedSection = match serde_json::from_value(raw) {
        Ok(p) => p,
        Err(e) => return Ok(invalid(e.to_string())),
    };
    if planned
        .meals
        .iter()
        .flat_map(|m| &m.items)
        .any(|i| i.name.trim().is_empty())
    {
        return Ok(invalid("有食物没有名称".into()));
    }
    let exercise_kcal = intake::planned_exercise_kcal(prior);
    let targets = intake::targets_with_exercise(ctx, date, exercise_kcal)
        .await?
        .ok_or_else(|| PluginError::Other("no diet targets: profile or weight missing".into()))?;
    let profile = ctx
        .profile
        .get()
        .await?
        .ok_or_else(|| PluginError::Other("no profile".into()))?;
    let dislikes = ctx.preferences.dislikes_on(date).await?;
    let sharing = SharedMeals::from_draft(prior, targets.kcal);

    let names: Vec<FoodQuery> = planned
        .meals
        .iter()
        .flat_map(|m| m.items.iter().map(|i| FoodQuery::new(i.name.trim())))
        .collect();
    let mut foods = resolve_many(ctx, &names).await?.into_iter();

    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    let mut day_total = Macros::default();
    let mut meals_json = Vec::new();

    for meal in &planned.meals {
        let mut meal_total = Macros::default();
        let mut items_json = Vec::new();
        let shared = sharing
            .as_ref()
            .and_then(|h| Some((h, h.meal(meal.meal_type)?)));
        let table = shared.map(|(h, shared)| {
            let guests = h.guests_at(shared);
            let pattern = shared_meals::strictest(
                std::iter::once(profile.dietary_pattern)
                    .chain(guests.iter().map(|g| g.guest.dietary_pattern)),
            );
            (guests, pattern)
        });
        let meal_label = format!("{}“{}”", meal.meal_type.zh_label(), meal.title);
        for item in &meal.items {
            let food = foods
                .next()
                .ok_or_else(|| PluginError::Other("resolver returned too few foods".into()))?;
            let planned_name = item.name.trim();
            let names = [planned_name, food.name.as_str()];

            if !(MIN_ITEM_GRAMS..=MAX_ITEM_GRAMS).contains(&item.grams) {
                violations.push(format!(
                    "“{planned_name}”的份量 {} g 超出 {MIN_ITEM_GRAMS}–{MAX_ITEM_GRAMS} g",
                    item.grams
                ));
            }
            for allergy in &profile.allergies {
                if names.iter().any(|n| matches_term(n, allergy)) {
                    violations.push(format!(
                        "{}“{}”中的“{planned_name}”含过敏原“{allergy}”",
                        meal.meal_type.zh_label(),
                        meal.title
                    ));
                }
            }
            for dislike in &dislikes {
                if names.iter().any(|n| matches_term(n, dislike)) {
                    warnings.push(format!("“{planned_name}”含不喜欢的“{dislike}”"));
                }
            }
            if let Some((guests, pattern)) = &table {
                let (v, w) = shared_meals::check_item(
                    guests,
                    *pattern,
                    &meal_label,
                    planned_name,
                    &names,
                    matches_term,
                );
                violations.extend(v);
                warnings.extend(w);
            }

            let entry = MealItem {
                macros: food.per_100g.for_grams(item.grams),
                name: food.name,
                kind: ItemKind::Food,
                brand: None,
                grams: Some(item.grams),
                servings: None,
                food_id: Some(food.id),
                dish_id: None,
                source: food.source_kind,
                source_id: Some(food.source_id),
                source_name: Some(food.source_name),
            };
            meal_total += entry.macros;
            items_json.push(entry.to_json());
        }
        day_total += meal_total;
        let mut meal_json = json!({
            "meal_type": meal.meal_type,
            "title": meal.title,
            "prep_note": meal.prep_note,
            "items": items_json,
            "total": meal_total.to_json(),
        });
        if let (Some((h, shared)), Some(obj)) = (shared, meal_json.as_object_mut()) {
            obj.insert("servings".into(), h.servings_json(shared, meal_total));
            obj.insert("portions".into(), json!(h.portions(shared)));
        }
        meals_json.push(meal_json);
    }

    let main_meals = planned
        .meals
        .iter()
        .filter(|m| m.meal_type != MealType::Snack)
        .count();
    let max_meals = usize::from(profile.meals_per_day) + 1;
    if main_meals != 3 {
        violations.push(format!(
            "正餐（早餐、午餐、晚餐）应为 3 顿，实际 {main_meals} 顿"
        ));
    }
    if !(3..=max_meals).contains(&planned.meals.len()) {
        violations.push(format!(
            "总餐数应在 3–{max_meals} 之间，实际 {}",
            planned.meals.len()
        ));
    }

    let t = targets;
    if outside(day_total.kcal, t.kcal, KCAL_TOLERANCE) {
        violations.push(format!(
            "全天热量 {:.0} kcal 超出目标 {:.0} kcal 的 ±8%（{:.0}–{:.0}）",
            day_total.kcal,
            t.kcal,
            t.kcal * (1.0 - KCAL_TOLERANCE),
            t.kcal * (1.0 + KCAL_TOLERANCE)
        ));
    }
    if day_total.protein_g < MIN_PROTEIN_SHARE * t.protein_g {
        violations.push(format!(
            "蛋白质 {:.0} g 低于目标 {:.0} g 的 90%",
            day_total.protein_g, t.protein_g
        ));
    }
    if outside(day_total.fat_g, t.fat_g, MACRO_TOLERANCE) {
        warnings.push(format!(
            "脂肪 {:.0} g 偏离目标 {:.0} g 超过 20%",
            day_total.fat_g, t.fat_g
        ));
    }
    if outside(day_total.carbs_g, t.carbs_g, MACRO_TOLERANCE) {
        warnings.push(format!(
            "碳水 {:.0} g 偏离目标 {:.0} g 超过 20%",
            day_total.carbs_g, t.carbs_g
        ));
    }
    if day_total.fiber_g < MIN_FIBER_SHARE * t.fiber_g {
        warnings.push(format!(
            "膳食纤维 {:.0} g 低于目标 {:.0} g 的 70%",
            day_total.fiber_g, t.fiber_g
        ));
    }

    let mut data = json!({
        "targets": t.as_macros().to_json(),
        "exercise_kcal_added": crate::targets::exercise_credit(exercise_kcal),
        "total": day_total.to_json(),
        "meals": meals_json,
        "tips": planned.tips,
    });
    if let (Some(h), Some(obj)) = (&sharing, data.as_object_mut()) {
        obj.insert("guest_estimates".into(), h.estimates_json());
        warnings.extend(shared_meals::clamp_warnings(h));
    }
    Ok(PlanSection {
        data,
        warnings,
        violations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::upsert_manual_food;
    use crate::testutil::{env, seed_person, RICE};
    use lifeagent_core::preferences::{NewPreference, PreferenceKind};
    use lifeagent_core::profile::Profile;
    use lifeagent_core::test_support::TestEnv;

    /// Seeds the person and a food whose per-100 g values are exactly a tenth of the day's
    /// targets, so 1,000 g of it hits every target. Returns the env and the plan date.
    async fn setup() -> (TestEnv, NaiveDate) {
        let env = env().await;
        seed_person(&env.ctx).await;
        let date = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let t = intake::targets_for(&env.ctx, date).await.unwrap().unwrap();
        let tenth = Macros {
            kcal: t.kcal / 10.0,
            protein_g: t.protein_g / 10.0,
            carbs_g: t.carbs_g / 10.0,
            fat_g: t.fat_g / 10.0,
            fiber_g: t.fiber_g / 10.0,
        };
        upsert_manual_food(&env.ctx, "均衡餐", tenth).await.unwrap();
        upsert_manual_food(
            &env.ctx,
            "低纤维均衡餐",
            Macros {
                fiber_g: 0.0,
                ..tenth
            },
        )
        .await
        .unwrap();
        upsert_manual_food(&env.ctx, "熟米饭", RICE).await.unwrap();
        (env, date)
    }

    fn meal(meal_type: &str, items: Value) -> Value {
        json!({"meal_type": meal_type, "title": format!("{meal_type}套餐"), "items": items})
    }

    /// Breakfast 300 g + lunch 400 g + dinner 300 g of 均衡餐: exactly on target.
    fn on_target() -> Value {
        json!({
            "meals": [
                meal("breakfast", json!([{"name": "均衡餐", "grams": 300}])),
                meal("lunch", json!([{"name": "均衡餐", "grams": 400}])),
                meal("dinner", json!([{"name": "均衡餐", "grams": 300}])),
            ],
            "tips": ["多喝水"]
        })
    }

    async fn set_profile(env: &TestEnv, edit: impl FnOnce(&mut Profile)) {
        let mut profile = env.ctx.profile.get().await.unwrap().unwrap();
        edit(&mut profile);
        env.ctx.profile.set(&profile).await.unwrap();
    }

    #[tokio::test]
    async fn on_target_plan_has_no_violations_or_warnings() {
        let (env, date) = setup().await;

        let section = finalize(&env.ctx, date, on_target(), &PlanDraft::new())
            .await
            .unwrap();

        assert_eq!(section.violations, Vec::<String>::new());
        assert_eq!(section.warnings, Vec::<String>::new());
        let t = intake::targets_for(&env.ctx, date).await.unwrap().unwrap();
        assert_eq!(section.data["total"]["kcal"], t.kcal.round());
        assert_eq!(
            section.data["meals"][1]["total"]["kcal"],
            (t.kcal * 0.4).round()
        );
        assert_eq!(section.data["meals"][0]["items"][0]["estimated"], false);
        assert_eq!(section.data["tips"], json!(["多喝水"]));
    }

    #[tokio::test]
    async fn planned_training_raises_the_kcal_target_checked_against() {
        let (env, date) = setup().await;
        let prior = PlanDraft::from([(
            "training".to_string(),
            json!({"rest_day": false, "exercise_kcal": 1000.0}),
        )]);

        // On target for the base day is now ~20% short of the raised target.
        let section = finalize(&env.ctx, date, on_target(), &prior).await.unwrap();

        assert_eq!(section.data["exercise_kcal_added"], 500.0);
        let base = intake::targets_for(&env.ctx, date).await.unwrap().unwrap();
        assert_eq!(section.data["targets"]["kcal"], base.kcal + 500.0);
        assert!(section.violations.iter().any(|v| v.starts_with("全天热量")));
    }

    #[tokio::test]
    async fn invalid_structure_is_a_violation() {
        let (env, date) = setup().await;
        let section = finalize(
            &env.ctx,
            date,
            json!({"meals": "不是数组"}),
            &PlanDraft::new(),
        )
        .await
        .unwrap();
        assert!(section.violations[0].starts_with("结构无效："));
        assert_eq!(section.data, Value::Null);
    }

    #[tokio::test]
    async fn kcal_outside_8_percent_is_a_violation() {
        let (env, date) = setup().await;
        let mut plan = on_target();
        // 1,000 g → 800 g: 80% of target.
        plan["meals"][2]["items"][0]["grams"] = json!(100);

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section.violations.iter().any(|v| v.starts_with("全天热量")));
    }

    #[tokio::test]
    async fn low_protein_is_a_violation_even_with_kcal_on_target() {
        let (env, date) = setup().await;
        let t = intake::targets_for(&env.ctx, date).await.unwrap().unwrap();
        let mut plan = on_target();
        // Swap dinner's 300 g of 均衡餐 for rice with the same energy but far less protein.
        let rice_grams = (t.kcal * 0.3) / RICE.kcal * 100.0;
        plan["meals"][2]["items"] = json!([{"name": "熟米饭", "grams": rice_grams}]);

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(!section.violations.iter().any(|v| v.starts_with("全天热量")));
        assert!(section.violations.iter().any(|v| v.starts_with("蛋白质")));
    }

    #[tokio::test]
    async fn meal_counts_are_checked() {
        let (env, date) = setup().await;
        let mut plan = on_target();
        // Dinner becomes a snack: 2 main meals.
        plan["meals"][2]["meal_type"] = json!("snack");

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section.violations.iter().any(|v| v.starts_with("正餐")));

        // meals_per_day 3 allows up to 4 meals in total; 5 is too many.
        let mut plan = on_target();
        let snack = meal("snack", json!([{"name": "均衡餐", "grams": 1}]));
        plan["meals"]
            .as_array_mut()
            .unwrap()
            .extend([snack.clone(), snack]);
        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();
        assert!(section.violations.iter().any(|v| v.starts_with("总餐数")));
    }

    #[tokio::test]
    async fn allergy_match_in_either_direction_is_a_violation() {
        let (env, date) = setup().await;
        set_profile(&env, |p| p.allergies = vec!["虾".into()]).await;
        upsert_manual_food(&env.ctx, "虾仁", RICE).await.unwrap();
        let mut plan = on_target();
        plan["meals"][1]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "虾仁", "grams": 1}));

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section
            .violations
            .iter()
            .any(|v| v.contains("含过敏原“虾”")));
        assert!(matches_term("花生油", "花生"));
        assert!(matches_term("花生", "花生油"));
        assert!(matches_term("Shrimp", "shrimp"));
        assert!(!matches_term("米饭", "虾"));
    }

    #[tokio::test]
    async fn item_grams_outside_1_to_1500_is_a_violation() {
        let (env, date) = setup().await;
        let mut plan = on_target();
        plan["meals"][1]["items"][0]["grams"] = json!(1600);

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section.violations.iter().any(|v| v.contains("1600 g 超出")));
    }

    #[tokio::test]
    async fn disliked_food_is_a_warning_not_a_violation() {
        let (env, date) = setup().await;
        env.ctx
            .preferences
            .add(
                &NewPreference {
                    kind: PreferenceKind::Dislike,
                    text: "香菜".into(),
                    valid_from: None,
                    valid_until: None,
                    permanent: true,
                },
                date,
            )
            .await
            .unwrap();
        upsert_manual_food(&env.ctx, "香菜", RICE).await.unwrap();
        let mut plan = on_target();
        plan["meals"][1]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "香菜", "grams": 1}));

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section.violations.is_empty());
        assert_eq!(section.warnings, vec!["“香菜”含不喜欢的“香菜”"]);
    }

    #[tokio::test]
    async fn low_fiber_is_a_warning() {
        let (env, date) = setup().await;
        let mut plan = on_target();
        for i in 0..3 {
            plan["meals"][i]["items"][0]["name"] = json!("低纤维均衡餐");
        }

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section.violations.is_empty());
        assert_eq!(section.warnings.len(), 1);
        assert!(section.warnings[0].starts_with("膳食纤维"));
    }

    #[tokio::test]
    async fn fat_and_carbs_off_by_more_than_20_percent_are_warnings() {
        let (env, date) = setup().await;
        let t = intake::targets_for(&env.ctx, date).await.unwrap().unwrap();
        // A food with the same energy per 100 g, but fat up 50% and carbs down to match.
        let extra_fat = t.fat_g / 10.0 * 0.5;
        upsert_manual_food(
            &env.ctx,
            "高脂餐",
            Macros {
                kcal: t.kcal / 10.0,
                protein_g: t.protein_g / 10.0,
                carbs_g: t.carbs_g / 10.0 - extra_fat * 9.0 / 4.0,
                fat_g: t.fat_g / 10.0 + extra_fat,
                fiber_g: t.fiber_g / 10.0,
            },
        )
        .await
        .unwrap();
        let mut plan = on_target();
        for i in 0..3 {
            plan["meals"][i]["items"][0]["name"] = json!("高脂餐");
        }

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert!(section.violations.is_empty());
        assert!(section.warnings.iter().any(|w| w.starts_with("脂肪")));
        assert!(section.warnings.iter().any(|w| w.starts_with("碳水")));
    }

    #[tokio::test]
    async fn estimated_foods_are_marked_not_warned() {
        let (env, date) = setup().await;
        env.mock.push_closure(crate::testutil::estimate_responder);
        let mut plan = on_target();
        plan["meals"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "鸡蛋", "grams": 1}));

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert_eq!(section.data["meals"][0]["items"][1]["estimated"], true);
        assert!(section.warnings.is_empty());
    }

    #[tokio::test]
    async fn a_dataset_only_food_name_is_matched_instead_of_estimated() {
        let (env, date) = setup().await;
        crate::testutil::add_dataset(&env, "表", &[("鸡蛋(煮)", crate::testutil::EGG)]).await;
        env.mock.push_response(crate::testutil::json_response(
            json!({"matches": [{"food": 0, "candidate": 0}]}),
        ));
        let mut plan = on_target();
        plan["meals"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "鸡蛋", "grams": 1}));

        let section = finalize(&env.ctx, date, plan, &PlanDraft::new())
            .await
            .unwrap();

        assert_eq!(section.data["meals"][0]["items"][1]["estimated"], false);
        // One match call and no estimate.
        assert_eq!(env.mock.received_requests().len(), 1);
    }
}
