//! The Household page's data (`docs/MULTIMEMBER_SPEC.md` §9): members with their weekly
//! attendance and dislikes, and the one-off changes for the coming week.

use chrono::{Days, NaiveDate};
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::Serialize;

use crate::model::{AttendanceOverride, DefaultSlot, Dislike, MealType, Member};
use crate::store;

/// Days of one-off changes shown, today included.
pub const OVERRIDE_DAYS: u64 = 7;

#[derive(Debug, Clone, Serialize)]
pub struct MemberView {
    #[serde(flatten)]
    pub member: Member,
    pub age: i32,
    /// 21 slots: every weekday × main meal.
    pub attendance: Vec<DefaultSlot>,
    pub dislikes: Vec<Dislike>,
}

/// Whether a member eats one main meal on one day, as resolved for planning.
#[derive(Debug, Clone, Serialize)]
pub struct DaySlot {
    pub member_id: String,
    pub meal_type: MealType,
    pub present: bool,
    /// A one-off change decides it (else the weekly default does).
    pub overridden: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayAttendance {
    pub date: NaiveDate,
    /// Every member × main meal, members in the order they were added.
    pub slots: Vec<DaySlot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HouseholdView {
    pub today: NaiveDate,
    pub members: Vec<MemberView>,
    /// From today through `today + OVERRIDE_DAYS − 1`.
    pub overrides: Vec<AttendanceOverride>,
    /// The same days, resolved: who eats each main meal and why.
    pub week: Vec<DayAttendance>,
}

pub async fn household_view(ctx: &PluginContext) -> Result<HouseholdView, PluginError> {
    let today = ctx.today().await?;
    let mut members = Vec::new();
    for member in store::list_members(ctx).await? {
        members.push(MemberView {
            age: member.age_on(today),
            attendance: store::defaults(ctx, &member.id).await?,
            dislikes: store::dislikes(ctx, &member.id)
                .await?
                .into_iter()
                .filter(|d| d.valid_until.is_none_or(|u| u >= today))
                .collect(),
            member,
        });
    }
    let last = today
        .checked_add_days(Days::new(OVERRIDE_DAYS - 1))
        .unwrap_or(today);
    let overrides = store::overrides_between(ctx, today, last).await?;
    let mut week = Vec::new();
    for offset in 0..OVERRIDE_DAYS {
        let Some(date) = today.checked_add_days(Days::new(offset)) else {
            break;
        };
        let attendance = store::attendance_on(ctx, date).await?;
        let mut slots = Vec::new();
        for m in &members {
            for meal_type in MealType::ALL {
                let o = overrides.iter().find(|o| {
                    o.plan_date == date && o.meal_type == meal_type && o.member_id == m.member.id
                });
                slots.push(DaySlot {
                    member_id: m.member.id.clone(),
                    meal_type,
                    present: attendance
                        .get(&meal_type)
                        .is_some_and(|ids| ids.contains(&m.member.id)),
                    overridden: o.is_some(),
                    note: o.and_then(|o| o.note.clone()),
                });
            }
        }
        week.push(DayAttendance { date, slots });
    }
    Ok(HouseholdView {
        today,
        members,
        overrides,
        week,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MealType;
    use crate::testutil::{aki, env, today};

    #[tokio::test]
    async fn view_has_members_attendance_dislikes_and_this_weeks_overrides() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let m = store::add_member(ctx, &aki(), None).await.unwrap();
        store::add_dislike(ctx, &m.id, "セロリ", None, None)
            .await
            .unwrap();
        // Expired dislikes are hidden.
        store::add_dislike(ctx, &m.id, "納豆", None, day.pred_opt())
            .await
            .unwrap();
        store::set_override(ctx, &m.id, day, MealType::Lunch, true, None)
            .await
            .unwrap();
        let far = day.checked_add_days(Days::new(OVERRIDE_DAYS)).unwrap();
        store::set_override(ctx, &m.id, far, MealType::Lunch, true, None)
            .await
            .unwrap();

        let view = household_view(ctx).await.unwrap();
        assert_eq!(view.today, day);
        assert_eq!(view.members.len(), 1);
        let mv = &view.members[0];
        assert_eq!(mv.attendance.len(), 21);
        assert_eq!(mv.dislikes.len(), 1);
        assert_eq!(view.overrides.len(), 1);
        assert_eq!(view.week.len(), OVERRIDE_DAYS as usize);
        let today_slots = &view.week[0].slots;
        assert_eq!(today_slots.len(), 3);
        let lunch = today_slots
            .iter()
            .find(|s| s.meal_type == MealType::Lunch)
            .unwrap();
        assert!(lunch.present && lunch.overridden);
        let dinner = today_slots
            .iter()
            .find(|s| s.meal_type == MealType::Dinner)
            .unwrap();
        assert!(dinner.present && !dinner.overridden, "the weekly default");
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["members"][0]["display_name"], "Aki");
    }
}
