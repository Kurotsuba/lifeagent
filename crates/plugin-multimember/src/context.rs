//! What the chat model and the Today page see about members (`docs/MULTIMEMBER_SPEC.md` §8).
//! The snippet never includes weight or height: those only matter to the planner, which reads
//! them from the plan section.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde_json::{json, Value};

use crate::model::{enum_str, DefaultSlot, MealType, Member};
use crate::store;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "daily", "never", or the weekdays ("Mon, Sat") a member usually eats `meal`.
fn usual_days(slots: &[DefaultSlot], meal: MealType) -> String {
    let days: Vec<&str> = slots
        .iter()
        .filter(|s| s.meal_type == meal && s.present)
        .filter_map(|s| WEEKDAYS.get(usize::from(s.weekday)).copied())
        .collect();
    match days.len() {
        0 => "never".into(),
        7 => "daily".into(),
        _ => days.join(", "),
    }
}

/// A member for tool results: profile, age, usual attendance and current dislikes.
pub async fn member_json(
    ctx: &PluginContext,
    m: &Member,
    today: NaiveDate,
) -> Result<Value, PluginError> {
    let slots = store::defaults(ctx, &m.id).await?;
    let usual: BTreeMap<&str, String> = MealType::ALL
        .into_iter()
        .map(|meal| (meal.as_str(), usual_days(&slots, meal)))
        .collect();
    let dislikes: Vec<Value> = store::dislikes(ctx, &m.id)
        .await?
        .into_iter()
        .filter(|d| d.valid_until.is_none_or(|u| u >= today))
        .map(|d| {
            json!({
                "id": d.id, "text": d.text,
                "valid_from": d.valid_from, "valid_until": d.valid_until
            })
        })
        .collect();
    let mut value = json!(m);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("age".into(), json!(m.age_on(today)));
        obj.insert("usual_attendance".into(), json!(usual));
        obj.insert("dislikes".into(), json!(dislikes));
    }
    Ok(value)
}

/// "Aki (34, female, omnivore; allergies: えび; dislikes: セロリ)".
fn member_label(m: &Member, date: NaiveDate, dislikes: &[String]) -> String {
    let p = &m.profile;
    let mut parts = vec![format!(
        "{}, {}, {}",
        m.age_on(date),
        enum_str(&p.sex),
        enum_str(&p.dietary_pattern)
    )];
    if !p.allergies.is_empty() {
        parts.push(format!("allergies: {}", p.allergies.join(", ")));
    }
    if !dislikes.is_empty() {
        parts.push(format!("dislikes: {}", dislikes.join(", ")));
    }
    format!("{} ({})", p.display_name, parts.join("; "))
}

/// "dinner: Aki, Ken; lunch: Aki", or "nobody joins" when no member attends.
fn attendance_line(
    attendance: &BTreeMap<MealType, Vec<String>>,
    names: &BTreeMap<&str, &str>,
) -> String {
    if attendance.is_empty() {
        return "nobody joins".into();
    }
    attendance
        .iter()
        .map(|(meal, ids)| {
            let who: Vec<&str> = ids
                .iter()
                .filter_map(|id| names.get(id.as_str()).copied())
                .collect();
            format!("{}: {}", meal.as_str(), who.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub async fn context_snippet(ctx: &PluginContext, date: NaiveDate) -> Result<String, PluginError> {
    let members = store::list_members(ctx).await?;
    if members.is_empty() {
        return Ok("Other members: none yet (the owner eats alone).".into());
    }
    let mut lines = vec![
        "Other members who share some of the owner's main meals (no goals; planned to maintain \
         weight). Don't assume how they are related to the owner:"
            .to_string(),
    ];
    for m in &members {
        let dislikes = store::dislikes_on(ctx, &m.id, date).await?;
        let slots = store::defaults(ctx, &m.id).await?;
        let usual: Vec<String> = MealType::ALL
            .into_iter()
            .map(|meal| format!("{} {}", meal.as_str(), usual_days(&slots, meal)))
            .collect();
        lines.push(format!(
            "- {}; usually: {}",
            member_label(m, date, &dislikes),
            usual.join(", ")
        ));
    }
    let names: BTreeMap<&str, &str> = members
        .iter()
        .map(|m| (m.id.as_str(), m.profile.display_name.as_str()))
        .collect();
    lines.push(format!(
        "Today ({date}): {}",
        attendance_line(&store::attendance_on(ctx, date).await?, &names)
    ));
    if let Some(tomorrow) = date.succ_opt() {
        lines.push(format!(
            "Tomorrow ({tomorrow}): {}",
            attendance_line(&store::attendance_on(ctx, tomorrow).await?, &names)
        ));
    }
    Ok(lines.join("\n"))
}

/// Who eats which main meal on `date`: `{"meals": [{"meal_type", "members": [{id, name}]}]}`.
pub async fn today_card(ctx: &PluginContext, date: NaiveDate) -> Result<Value, PluginError> {
    let members = store::list_members(ctx).await?;
    let attendance = store::attendance_on(ctx, date).await?;
    let meals: Vec<Value> = attendance
        .iter()
        .map(|(meal, ids)| {
            let who: Vec<Value> = ids
                .iter()
                .filter_map(|id| members.iter().find(|m| &m.id == id))
                .map(|m| json!({"id": m.id, "name": m.profile.display_name}))
                .collect();
            json!({"meal_type": meal, "members": who})
        })
        .collect();
    Ok(json!({ "meals": meals }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MemberProfile;
    use crate::testutil::{aki, env, today};

    #[tokio::test]
    async fn snippet_lists_members_and_attendance_without_body_measurements() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        assert!(context_snippet(ctx, day)
            .await
            .unwrap()
            .contains("none yet"));

        let m = store::add_member(ctx, &aki(), None).await.unwrap();
        store::add_dislike(ctx, &m.id, "セロリ", None, None)
            .await
            .unwrap();
        let snippet = context_snippet(ctx, day).await.unwrap();
        assert!(snippet.contains("Aki ("), "{snippet}");
        assert!(snippet.contains("allergies: えび"), "{snippet}");
        assert!(snippet.contains("dislikes: セロリ"), "{snippet}");
        assert!(snippet.contains("dinner daily"), "{snippet}");
        assert!(snippet.contains("Today") && snippet.contains("dinner: Aki"));
        assert!(
            !snippet.contains("52") && !snippet.contains("160"),
            "{snippet}"
        );
    }

    #[tokio::test]
    async fn today_card_lists_who_eats_each_meal() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let m = store::add_member(ctx, &aki(), None).await.unwrap();
        let ken = MemberProfile {
            display_name: "Ken".into(),
            ..aki()
        };
        store::add_member(ctx, &ken, None).await.unwrap();
        store::set_override(ctx, &m.id, day, MealType::Breakfast, true, None)
            .await
            .unwrap();
        let card = today_card(ctx, day).await.unwrap();
        assert_eq!(card["meals"][0]["meal_type"], "breakfast");
        assert_eq!(card["meals"][0]["members"][0]["name"], "Aki");
        assert_eq!(card["meals"][1]["meal_type"], "dinner");
        assert_eq!(card["meals"][1]["members"].as_array().unwrap().len(), 2);
    }
}
