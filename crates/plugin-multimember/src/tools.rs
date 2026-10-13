//! Multimember tools (`docs/MULTIMEMBER_SPEC.md` §7). The chat always speaks as the owner; these
//! let the owner tell the assistant about the other people they share meals with.

use chrono::{Datelike, NaiveDate};
use lifeagent_core::plugin::{PluginContext, PluginError, ToolOutput, ToolSummary};
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Sex};
use lifeagent_llm::types::ToolSpec;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::context::member_json;
use crate::model::{DefaultSlot, MealType, MemberProfile, MemberRule};
use crate::store;

const NOT_FOR_LOGGING: &str = "These tools never log food or weight for anyone, and members \
     can't have goals: the app plans shared meals that keep them at their current weight.";

pub fn specs() -> Vec<ToolSpec> {
    let profile_props = json!({
        "display_name": {"type": "string"},
        "sex": {"type": "string", "enum": ["male", "female"]},
        "birth_date": {
            "type": "string",
            "description": "Birth year and month, YYYY-MM. Ask only for the year and month, \
                never the day."
        },
        "height_cm": {"type": "number"},
        "weight_kg": {"type": "number"},
        "activity_level": {
            "type": "string",
            "enum": ["sedentary", "light", "moderate", "active", "very_active"]
        },
        "dietary_pattern": {
            "type": "string",
            "enum": ["omnivore", "vegetarian", "vegan", "pescatarian"]
        },
        "allergies": {"type": "array", "items": {"type": "string"}}
    });
    let slot = json!({
        "type": "object",
        "required": ["weekday", "meal_type", "present"],
        "properties": {
            "weekday": {"type": "integer", "description": "0 = Monday … 6 = Sunday"},
            "meal_type": {"type": "string", "enum": ["breakfast", "lunch", "dinner"]},
            "present": {"type": "boolean"}
        }
    });
    let member = json!({"type": "string", "description": "The member's id or exact name"});
    let mut add_props = profile_props.clone();
    if let Some(props) = add_props.as_object_mut() {
        props.insert("attendance".into(), json!({"type": "array", "items": slot}));
    }
    let mut update_props = profile_props;
    if let Some(props) = update_props.as_object_mut() {
        props.insert("member".into(), member.clone());
    }
    vec![
        ToolSpec {
            name: "multimember_list_members".into(),
            description: format!(
                "List the other members who share meals with the owner, with their profile, \
                 usual meal attendance and dislikes. {NOT_FOR_LOGGING}"
            ),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "multimember_add_member".into(),
            description: format!(
                "Add a member: another person who regularly shares meals with the owner. Don't \
                 assume or ask how they are related to the owner. Members must be aged 16–70; ask for every required field you \
                 don't know rather than guessing. `attendance` sets which main meals they \
                 usually eat with the owner; without it, dinner every day. Waits for the owner's \
                 approval in the app. {NOT_FOR_LOGGING}"
            ),
            parameters: json!({
                "type": "object",
                "required": ["display_name", "sex", "birth_date", "height_cm", "weight_kg",
                             "activity_level", "dietary_pattern"],
                "properties": add_props
            }),
        },
        ToolSpec {
            name: "multimember_update_member".into(),
            description: format!(
                "Change a member's profile (name, weight, allergies, ...). Only pass the fields \
                 that change; `allergies` replaces the whole list, and a change to it waits for \
                 the owner's approval in the app. {NOT_FOR_LOGGING}"
            ),
            parameters: json!({
                "type": "object",
                "required": ["member"],
                "properties": update_props
            }),
        },
        ToolSpec {
            name: "multimember_archive_member".into(),
            description: "Remove a member who no longer shares meals. Past plans keep their name. \
                Waits for the owner's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["member"],
                "properties": {"member": member}
            }),
        },
        ToolSpec {
            name: "multimember_set_attendance".into(),
            description: "Set whether a member eats a main meal with the owner. With `date` \
                (YYYY-MM-DD, today or later) it is a one-off change (\"Aki is out for dinner on \
                Friday\"), with an optional short `note`. With `weekdays` (0 = Monday … 6 = \
                Sunday) it changes their usual week. Pass one of the two. Saving doesn't change \
                an existing plan: to apply it, also call core_revise_plan (or \
                core_generate_plan if that day has no plan yet)."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["member", "meal_type", "present"],
                "properties": {
                    "member": member,
                    "meal_type": {"type": "string", "enum": ["breakfast", "lunch", "dinner"]},
                    "present": {"type": "boolean"},
                    "date": {"type": "string"},
                    "weekdays": {"type": "array", "items": {"type": "integer"}},
                    "note": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "multimember_clear_attendance_override".into(),
            description: "Undo a one-off attendance change for a date and meal, so the member's \
                usual week applies again."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["member", "date", "meal_type"],
                "properties": {
                    "member": member,
                    "date": {"type": "string"},
                    "meal_type": {"type": "string", "enum": ["breakfast", "lunch", "dinner"]}
                }
            }),
        },
        ToolSpec {
            name: "multimember_add_dislike".into(),
            description: "Save a food a member dislikes, so shared meals avoid it. Optional \
                `valid_from`/`valid_until` (YYYY-MM-DD) limit it to a period."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["member", "text"],
                "properties": {
                    "member": member,
                    "text": {"type": "string"},
                    "valid_from": {"type": "string"},
                    "valid_until": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "multimember_remove_dislike".into(),
            description: "Remove a member's dislike by its id (from multimember_list_members)."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["id"],
                "properties": {"id": {"type": "string"}}
            }),
        },
    ]
}

/// Adding a member (the preview shows their allergies, so a member saved without the ones the
/// owner named is caught), removing one, and changing a member's allergies (hard constraints for
/// shared meals) wait for the owner's approval (`lifeagent_core::approvals`). An unknown member,
/// or a profile the tool would refuse, runs and fails without asking.
pub async fn approval_preview(
    ctx: &PluginContext,
    name: &str,
    args: &Value,
) -> Result<Option<Value>, PluginError> {
    match name {
        "multimember_add_member" => {
            let a: AddArgs = parse_args(args.clone())?;
            let Ok(profile) = a.profile.normalized(ctx.today().await?) else {
                return Ok(None);
            };
            Ok(Some(json!({
                "kind": "add_member",
                "name": profile.display_name,
                "allergies": profile.allergies,
            })))
        }
        "multimember_archive_member" => {
            let a: MemberArg = parse_args(args.clone())?;
            let Ok(m) = store::resolve(ctx, &a.member).await else {
                return Ok(None);
            };
            Ok(Some(json!({
                "kind": "archive_member",
                "name": m.profile.display_name,
            })))
        }
        "multimember_update_member" => {
            let a: UpdateArgs = parse_args(args.clone())?;
            let Some(allergies) = a.allergies else {
                return Ok(None);
            };
            let Ok(m) = store::resolve(ctx, &a.member).await else {
                return Ok(None);
            };
            let allergies: Vec<String> = allergies.iter().map(|t| t.trim().to_string()).collect();
            if allergies == m.profile.allergies {
                return Ok(None);
            }
            Ok(Some(json!({
                "kind": "update_member_allergies",
                "name": m.profile.display_name,
                "allergies": allergies,
            })))
        }
        _ => Ok(None),
    }
}

pub async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
    match name {
        "multimember_list_members" => list_members(ctx).await,
        "multimember_add_member" => add_member(ctx, args).await,
        "multimember_update_member" => update_member(ctx, args).await,
        "multimember_archive_member" => archive_member(ctx, args).await,
        "multimember_set_attendance" => set_attendance(ctx, args).await,
        "multimember_clear_attendance_override" => clear_override(ctx, args).await,
        "multimember_add_dislike" => add_dislike(ctx, args).await,
        "multimember_remove_dislike" => remove_dislike(ctx, args).await,
        other => Err(PluginError::NotFound(format!("unknown tool {other:?}"))),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(mut args: Value) -> Result<T, PluginError> {
    birth_month_to_date(&mut args);
    serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))
}

/// The assistant asks for a member's birth year and month only, like the web form; a `YYYY-MM`
/// `birth_date` is stored as the 1st of that month. A full `YYYY-MM-DD` is still accepted.
fn birth_month_to_date(args: &mut Value) {
    if let Some(v) = args.get_mut("birth_date") {
        if let Some(s) = v.as_str() {
            let s = s.trim();
            if NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").is_ok() {
                *v = json!(format!("{s}-01"));
            }
        }
    }
}

fn parse_date(s: &str) -> Result<NaiveDate, PluginError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| PluginError::InvalidArgs(format!("invalid date {s:?}, expected YYYY-MM-DD")))
}

fn parse_opt_date(s: Option<&str>) -> Result<Option<NaiveDate>, PluginError> {
    s.map(parse_date).transpose()
}

async fn list_members(ctx: &PluginContext) -> Result<ToolOutput, PluginError> {
    let today = ctx.today().await?;
    let members = store::list_members(ctx).await?;
    let mut out = Vec::with_capacity(members.len());
    for m in &members {
        out.push(member_json(ctx, m, today).await?);
    }
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_members_viewed").with("count", members.len()),
        content: json!({ "members": out }),
    })
}

#[derive(Deserialize)]
struct AddArgs {
    #[serde(flatten)]
    profile: MemberProfile,
    #[serde(default)]
    attendance: Option<Vec<DefaultSlot>>,
}

async fn add_member(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: AddArgs = parse_args(args)?;
    let m = store::add_member(ctx, &a.profile, a.attendance.as_deref()).await?;
    let today = ctx.today().await?;
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_member_added").with("name", &m.profile.display_name),
        content: json!({ "member": member_json(ctx, &m, today).await? }),
    })
}

#[derive(Deserialize)]
struct UpdateArgs {
    member: String,
    display_name: Option<String>,
    sex: Option<Sex>,
    birth_date: Option<NaiveDate>,
    height_cm: Option<f64>,
    weight_kg: Option<f64>,
    activity_level: Option<ActivityLevel>,
    dietary_pattern: Option<DietaryPattern>,
    allergies: Option<Vec<String>>,
}

async fn update_member(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: UpdateArgs = parse_args(args)?;
    let m = store::resolve(ctx, &a.member).await?;
    let p = m.profile;
    let profile = MemberProfile {
        display_name: a.display_name.unwrap_or(p.display_name),
        sex: a.sex.unwrap_or(p.sex),
        // Like the web form, a year and month that don't change keep the stored day.
        birth_date: a
            .birth_date
            .filter(|d| (d.year(), d.month()) != (p.birth_date.year(), p.birth_date.month()))
            .unwrap_or(p.birth_date),
        height_cm: a.height_cm.unwrap_or(p.height_cm),
        weight_kg: a.weight_kg.unwrap_or(p.weight_kg),
        activity_level: a.activity_level.unwrap_or(p.activity_level),
        dietary_pattern: a.dietary_pattern.unwrap_or(p.dietary_pattern),
        allergies: a.allergies.unwrap_or(p.allergies),
    };
    let m = store::update_member(ctx, &m.id, &profile).await?;
    let today = ctx.today().await?;
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_member_updated")
            .with("name", &m.profile.display_name),
        content: json!({ "member": member_json(ctx, &m, today).await? }),
    })
}

#[derive(Deserialize)]
struct MemberArg {
    member: String,
}

async fn archive_member(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: MemberArg = parse_args(args)?;
    let m = store::resolve(ctx, &a.member).await?;
    store::archive_member(ctx, &m.id).await?;
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_member_archived")
            .with("name", &m.profile.display_name),
        content: json!({ "archived": m.id }),
    })
}

#[derive(Deserialize)]
struct AttendanceArgs {
    member: String,
    meal_type: MealType,
    present: bool,
    date: Option<String>,
    weekdays: Option<Vec<u8>>,
    note: Option<String>,
}

async fn set_attendance(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: AttendanceArgs = parse_args(args)?;
    let m = store::resolve(ctx, &a.member).await?;
    let name = &m.profile.display_name;
    match (a.date.as_deref(), a.weekdays) {
        (Some(date), None) => {
            let date = parse_date(date)?;
            store::set_override(ctx, &m.id, date, a.meal_type, a.present, a.note.as_deref())
                .await?;
            Ok(ToolOutput {
                summary: ToolSummary::new("multimember_attendance_set")
                    .with("name", name)
                    .with("meal_type", a.meal_type)
                    .with("present", a.present)
                    .with("date", Some(date)),
                content: json!({
                    "member_id": m.id, "date": date, "meal_type": a.meal_type,
                    "present": a.present
                }),
            })
        }
        (None, Some(weekdays)) if !weekdays.is_empty() => {
            if weekdays.iter().any(|&w| w > 6) {
                return Err(PluginError::from(crate::model::MemberError::from(
                    MemberRule::AttendanceInvalid,
                )));
            }
            for weekday in &weekdays {
                let slot = DefaultSlot {
                    weekday: *weekday,
                    meal_type: a.meal_type,
                    present: a.present,
                };
                store::set_default(ctx, &m.id, &slot).await?;
            }
            Ok(ToolOutput {
                summary: ToolSummary::new("multimember_attendance_set")
                    .with("name", name)
                    .with("meal_type", a.meal_type)
                    .with("present", a.present)
                    .with("date", None::<NaiveDate>),
                content: json!({
                    "member_id": m.id, "weekdays": weekdays, "meal_type": a.meal_type,
                    "present": a.present
                }),
            })
        }
        _ => Err(PluginError::InvalidArgs(
            "pass either `date` or a non-empty `weekdays`, not both".into(),
        )),
    }
}

#[derive(Deserialize)]
struct ClearArgs {
    member: String,
    date: String,
    meal_type: MealType,
}

async fn clear_override(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: ClearArgs = parse_args(args)?;
    let m = store::resolve(ctx, &a.member).await?;
    let date = parse_date(&a.date)?;
    store::clear_override(ctx, &m.id, date, a.meal_type).await?;
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_attendance_cleared")
            .with("name", &m.profile.display_name)
            .with("meal_type", a.meal_type)
            .with("date", date),
        content: json!({ "member_id": m.id, "date": date, "meal_type": a.meal_type }),
    })
}

#[derive(Deserialize)]
struct DislikeArgs {
    member: String,
    text: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

async fn add_dislike(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: DislikeArgs = parse_args(args)?;
    let m = store::resolve(ctx, &a.member).await?;
    let d = store::add_dislike(
        ctx,
        &m.id,
        &a.text,
        parse_opt_date(a.valid_from.as_deref())?,
        parse_opt_date(a.valid_until.as_deref())?,
    )
    .await?;
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_dislike_added")
            .with("name", &m.profile.display_name)
            .with("text", &d.text),
        content: json!({ "dislike": d }),
    })
}

#[derive(Deserialize)]
struct IdArg {
    id: String,
}

async fn remove_dislike(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let a: IdArg = parse_args(args)?;
    store::remove_dislike(ctx, &a.id).await?;
    Ok(ToolOutput {
        summary: ToolSummary::new("multimember_dislike_removed"),
        content: json!({ "removed": a.id }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, today};

    fn aki_args() -> Value {
        json!({
            "display_name": "Aki", "sex": "female", "birth_date": "1992-05-10",
            "height_cm": 160, "weight_kg": 52, "activity_level": "light",
            "dietary_pattern": "omnivore", "allergies": ["えび"]
        })
    }

    #[tokio::test]
    async fn adding_or_removing_a_member_or_changing_their_allergies_needs_approval() {
        let env = env().await;
        let ctx = &env.ctx;
        let preview = |args: Value| async move {
            approval_preview(ctx, args["tool"].as_str().unwrap(), &args["args"])
                .await
                .unwrap()
        };
        let mut no_allergies = aki_args();
        no_allergies["display_name"] = json!(" Ken ");
        no_allergies["allergies"] = json!([" "]);
        assert_eq!(
            preview(json!({"tool": "multimember_add_member", "args": no_allergies})).await,
            Some(json!({"kind": "add_member", "name": "Ken", "allergies": []}))
        );
        // A profile the tool refuses (too young) runs and fails instead of asking.
        let mut child = aki_args();
        child["birth_date"] = json!(format!("{}-01-01", today(ctx).await.year() - 5));
        assert_eq!(
            preview(json!({"tool": "multimember_add_member", "args": child})).await,
            None
        );

        call(ctx, "multimember_add_member", aki_args())
            .await
            .unwrap();

        assert_eq!(
            preview(json!({"tool": "multimember_archive_member", "args": {"member": "aki"}})).await,
            Some(json!({"kind": "archive_member", "name": "Aki"}))
        );
        assert_eq!(
            preview(json!({"tool": "multimember_update_member",
                           "args": {"member": "Aki", "allergies": []}}))
            .await,
            Some(json!({"kind": "update_member_allergies", "name": "Aki", "allergies": []}))
        );
        // Same allergies, other fields, or an unknown member: no approval.
        for args in [
            json!({"member": "Aki", "allergies": ["えび"]}),
            json!({"member": "Aki", "weight_kg": 53}),
            json!({"member": "Nobody", "allergies": []}),
        ] {
            assert_eq!(
                preview(json!({"tool": "multimember_update_member", "args": args})).await,
                None
            );
        }
        assert_eq!(
            preview(json!({"tool": "multimember_list_members", "args": {}})).await,
            None
        );
    }

    #[tokio::test]
    async fn add_update_list_and_archive_through_tools() {
        let env = env().await;
        let ctx = &env.ctx;
        let out = call(ctx, "multimember_add_member", aki_args())
            .await
            .unwrap();
        assert_eq!(out.content["member"]["display_name"], "Aki");

        call(
            ctx,
            "multimember_update_member",
            json!({"member": "aki", "weight_kg": 53.5}),
        )
        .await
        .unwrap();
        let list = call(ctx, "multimember_list_members", json!({}))
            .await
            .unwrap();
        let m = &list.content["members"][0];
        assert_eq!(m["weight_kg"], 53.5);
        assert_eq!(m["allergies"], json!(["えび"]), "unchanged fields kept");
        assert_eq!(m["usual_attendance"]["dinner"], "daily");

        call(ctx, "multimember_archive_member", json!({"member": "Aki"}))
            .await
            .unwrap();
        let list = call(ctx, "multimember_list_members", json!({}))
            .await
            .unwrap();
        assert_eq!(list.content["members"], json!([]));
    }

    #[tokio::test]
    async fn birth_date_is_taken_as_year_and_month() {
        let env = env().await;
        let ctx = &env.ctx;
        let mut aki = aki_args();
        aki["birth_date"] = json!("1992-05");
        let out = call(ctx, "multimember_add_member", aki).await.unwrap();
        let birth = |m: &Value| m["birth_date"].as_str().map(str::to_string);
        assert_eq!(birth(&out.content["member"]).as_deref(), Some("1992-05-01"));

        // Same year and month keeps the stored day; a new month moves to its 1st.
        let id = out.content["member"]["id"].as_str().unwrap().to_string();
        let m = store::resolve(ctx, &id).await.unwrap();
        let mut p = m.profile.clone();
        p.birth_date = NaiveDate::from_ymd_opt(1992, 5, 10).unwrap();
        store::update_member(ctx, &id, &p).await.unwrap();
        let updated = |args: Value| async move {
            let out = call(ctx, "multimember_update_member", args).await.unwrap();
            birth(&out.content["member"])
        };
        assert_eq!(
            updated(json!({"member": "Aki", "birth_date": "1992-05"}))
                .await
                .as_deref(),
            Some("1992-05-10")
        );
        assert_eq!(
            updated(json!({"member": "Aki", "birth_date": "1992-06"}))
                .await
                .as_deref(),
            Some("1992-06-01")
        );
    }

    #[tokio::test]
    async fn rule_violations_and_unknown_members_are_fixable_errors() {
        let env = env().await;
        let ctx = &env.ctx;
        let mut too_old = aki_args();
        too_old["birth_date"] = json!("1940-01-01");
        match call(ctx, "multimember_add_member", too_old).await {
            Err(PluginError::InvalidArgs(msg)) => assert!(msg.contains("16–70"), "{msg}"),
            other => panic!("{:?}", other.map(|o| o.content)),
        }
        call(ctx, "multimember_add_member", aki_args())
            .await
            .unwrap();
        match call(
            ctx,
            "multimember_add_dislike",
            json!({"member": "Ken", "text": "セロリ"}),
        )
        .await
        {
            Err(PluginError::NotFound(msg)) => assert!(msg.contains("Aki"), "{msg}"),
            other => panic!("{:?}", other.map(|o| o.content)),
        }
    }

    #[tokio::test]
    async fn set_attendance_takes_a_date_or_weekdays() {
        let env = env().await;
        let ctx = &env.ctx;
        call(ctx, "multimember_add_member", aki_args())
            .await
            .unwrap();
        let day = today(ctx).await;

        call(
            ctx,
            "multimember_set_attendance",
            json!({"member": "Aki", "meal_type": "dinner", "present": false,
                   "date": day.to_string(), "note": "飲み会"}),
        )
        .await
        .unwrap();
        assert_eq!(store::attendance_on(ctx, day).await.unwrap().len(), 0);

        let wd = day.weekday().num_days_from_monday();
        call(
            ctx,
            "multimember_set_attendance",
            json!({"member": "Aki", "meal_type": "lunch", "present": true, "weekdays": [wd]}),
        )
        .await
        .unwrap();
        let on = store::attendance_on(ctx, day).await.unwrap();
        assert!(on.contains_key(&MealType::Lunch));

        call(
            ctx,
            "multimember_clear_attendance_override",
            json!({"member": "Aki", "meal_type": "dinner", "date": day.to_string()}),
        )
        .await
        .unwrap();
        assert!(store::attendance_on(ctx, day)
            .await
            .unwrap()
            .contains_key(&MealType::Dinner));

        for bad in [
            json!({"member": "Aki", "meal_type": "dinner", "present": true}),
            json!({"member": "Aki", "meal_type": "dinner", "present": true,
                   "date": day.to_string(), "weekdays": [1]}),
            json!({"member": "Aki", "meal_type": "dinner", "present": true, "weekdays": [7]}),
            json!({"member": "Aki", "meal_type": "snack", "present": true, "weekdays": [1]}),
        ] {
            assert!(matches!(
                call(ctx, "multimember_set_attendance", bad).await,
                Err(PluginError::InvalidArgs(_))
            ));
        }
    }

    #[tokio::test]
    async fn dislikes_through_tools() {
        let env = env().await;
        let ctx = &env.ctx;
        call(ctx, "multimember_add_member", aki_args())
            .await
            .unwrap();
        let out = call(
            ctx,
            "multimember_add_dislike",
            json!({"member": "Aki", "text": "セロリ"}),
        )
        .await
        .unwrap();
        let id = out.content["dislike"]["id"].as_str().unwrap().to_string();
        let list = call(ctx, "multimember_list_members", json!({}))
            .await
            .unwrap();
        assert_eq!(list.content["members"][0]["dislikes"][0]["text"], "セロリ");
        call(ctx, "multimember_remove_dislike", json!({"id": id}))
            .await
            .unwrap();
        let list = call(ctx, "multimember_list_members", json!({}))
            .await
            .unwrap();
        assert_eq!(list.content["members"][0]["dislikes"], json!([]));
    }
}
