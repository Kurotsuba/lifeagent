//! The computed `multimember` plan section (`docs/MULTIMEMBER_SPEC.md` §6.1): who shares each
//! main meal on the plan date, with what the diet plugin needs to plan for them. Built from
//! stored data only; core never shows it to the planner LLM.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PlanSection, PluginContext, PluginError};
use serde::Serialize;
use serde_json::Value;

use crate::model::{MealType, Member, MAX_AGE, MIN_AGE};
use crate::store;

#[derive(Debug, Serialize)]
struct SharedMeal {
    meal_type: MealType,
    guest_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Guest {
    member_id: String,
    name: String,
    sex: lifeagent_core::profile::Sex,
    age: i32,
    height_cm: f64,
    weight_kg: f64,
    activity_level: lifeagent_core::profile::ActivityLevel,
    dietary_pattern: lifeagent_core::profile::DietaryPattern,
    allergies: Vec<String>,
    dislikes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Section {
    meals: Vec<SharedMeal>,
    guests: Vec<Guest>,
}

/// `None` when nobody joins any main meal on `date` (and no member had to be left out), so diet
/// plans for the owner alone, exactly as without this plugin.
pub async fn computed_section(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<Option<PlanSection>, PluginError> {
    let members = store::list_members(ctx).await?;
    let attendance = store::attendance_on(ctx, date).await?;
    let mut warnings = Vec::new();

    let attending = |m: &Member| attendance.values().any(|ids| ids.contains(&m.id));
    let mut guests = Vec::new();
    for m in members.iter().filter(|m| attending(m)) {
        if !m.in_age_range_on(date) {
            warnings.push(format!(
                "“{}”不在 {MIN_AGE}–{MAX_AGE} 岁范围内（{} 岁），未计入共餐",
                m.profile.display_name,
                m.age_on(date)
            ));
            continue;
        }
        let p = &m.profile;
        guests.push(Guest {
            member_id: m.id.clone(),
            name: p.display_name.clone(),
            sex: p.sex,
            age: m.age_on(date),
            height_cm: p.height_cm,
            weight_kg: p.weight_kg,
            activity_level: p.activity_level,
            dietary_pattern: p.dietary_pattern,
            allergies: p.allergies.clone(),
            dislikes: store::dislikes_on(ctx, &m.id, date).await?,
        });
    }

    let meals: Vec<SharedMeal> = attendance
        .into_iter()
        .map(|(meal_type, ids)| SharedMeal {
            meal_type,
            guest_ids: ids
                .into_iter()
                .filter(|id| guests.iter().any(|g| &g.member_id == id))
                .collect(),
        })
        .filter(|m| !m.guest_ids.is_empty())
        .collect();

    if meals.is_empty() && warnings.is_empty() {
        return Ok(None);
    }
    let data: Value = serde_json::to_value(Section { meals, guests })
        .map_err(|e| PluginError::Other(e.to_string()))?;
    Ok(Some(PlanSection {
        data,
        warnings,
        violations: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DefaultSlot, MemberProfile};
    use crate::testutil::{aki, env, today};
    use chrono::Datelike;
    use serde_json::json;

    #[tokio::test]
    async fn none_without_attending_members() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        assert!(computed_section(ctx, day).await.unwrap().is_none());
        // A member who eats nothing with the owner that day.
        store::add_member(ctx, &aki(), Some(&[])).await.unwrap();
        assert!(computed_section(ctx, day).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn lists_shared_meals_and_guest_profiles() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let wd = day.weekday().num_days_from_monday() as u8;
        let aki_m = store::add_member(ctx, &aki(), None).await.unwrap();
        store::add_dislike(ctx, &aki_m.id, "セロリ", None, None)
            .await
            .unwrap();
        let ken = MemberProfile {
            display_name: "Ken".into(),
            allergies: vec![],
            ..aki()
        };
        let ken_m = store::add_member(
            ctx,
            &ken,
            Some(&[DefaultSlot {
                weekday: wd,
                meal_type: MealType::Breakfast,
                present: true,
            }]),
        )
        .await
        .unwrap();

        let section = computed_section(ctx, day).await.unwrap().unwrap();
        assert!(section.warnings.is_empty());
        let data = section.data;
        assert_eq!(
            data["meals"],
            json!([
                {"meal_type": "breakfast", "guest_ids": [ken_m.id]},
                {"meal_type": "dinner", "guest_ids": [aki_m.id]}
            ])
        );
        let g = &data["guests"][0];
        assert_eq!(g["name"], "Aki");
        assert_eq!(g["weight_kg"], 52.0);
        assert_eq!(g["sex"], "female");
        assert_eq!(g["allergies"], json!(["えび"]));
        assert_eq!(g["dislikes"], json!(["セロリ"]));
        assert_eq!(g["age"], aki_m.age_on(day));
    }

    #[tokio::test]
    async fn a_member_who_aged_out_is_left_out_with_a_warning() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        // 70 today, 71 tomorrow.
        let born = day
            .succ_opt()
            .unwrap()
            .checked_sub_months(chrono::Months::new(71 * 12))
            .unwrap();
        let elder = MemberProfile {
            birth_date: born,
            ..aki()
        };
        store::add_member(ctx, &elder, None).await.unwrap();
        assert!(computed_section(ctx, day).await.unwrap().is_some());

        let tomorrow = day.succ_opt().unwrap();
        let section = computed_section(ctx, tomorrow).await.unwrap().unwrap();
        assert_eq!(section.data["meals"], json!([]));
        assert_eq!(section.data["guests"], json!([]));
        assert_eq!(section.warnings.len(), 1);
        assert!(
            section.warnings[0].contains("Aki"),
            "{:?}",
            section.warnings
        );
    }
}
