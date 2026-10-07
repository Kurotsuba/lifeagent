//! Core tool specs and handlers, per `docs/PHASE1_SPEC.md` §7.8.

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::context::render_plan_compact;
use crate::events::EventSource;
use crate::plan::{PlanService, PlanTrigger, StoredPlan};
use crate::plugin::{PluginContext, PluginError, ToolOutput, ToolSummary};
use crate::preferences::{NewPreference, PreferenceKind};
use crate::time::parse_local_datetime;

pub fn specs() -> Vec<lifeagent_llm::types::ToolSpec> {
    use lifeagent_llm::types::ToolSpec;

    vec![
        ToolSpec {
            name: "core_log_weight".into(),
            description: "Record the user's body weight. Use whenever the user reports a \
                current weight."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["kg"],
                "properties": {
                    "kg": {"type": "number"},
                    "measured_at": {
                        "type": "string",
                        "description": "Local ISO datetime (YYYY-MM-DDTHH:MM); defaults to now."
                    }
                }
            }),
        },
        ToolSpec {
            name: "core_update_allergies".into(),
            description: "Add or remove foods the user is allergic to. Use whenever the user \
                reports an allergy (or that one is gone). Allergies are permanent hard \
                constraints; use core_add_preference for foods they merely dislike. The change \
                waits for the user's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "add": {"type": "array", "items": {"type": "string"}},
                    "remove": {"type": "array", "items": {"type": "string"}}
                }
            }),
        },
        ToolSpec {
            name: "core_add_preference".into(),
            description: "Save a preference that should shape future plans. kind \"dislike\" \
                is one food the user doesn't want (text is the food's name, e.g. 'cilantro'); \
                kind \"general\" is anything else (e.g. 'weekday lunch is at the company \
                canteen', 'only convenience-store food'). valid_from/valid_until are logical \
                dates YYYY-MM-DD, both inclusive. If the user says how long it applies \
                (e.g. 'this week I work late'), set valid_until to match. Without valid_until \
                it lasts one month from valid_from (default today). Set permanent: true only \
                when the user says it always applies (e.g. 'I never eat cilantro'). Text at \
                most 300 characters. Not for allergies (core_update_allergies) or one-off changes to \
                a single plan (core_revise_plan). Waits for the user's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["kind", "text"],
                "properties": {
                    "kind": {"type": "string", "enum": ["dislike", "general"]},
                    "text": {"type": "string"},
                    "valid_from": {"type": "string"},
                    "valid_until": {"type": "string"},
                    "permanent": {"type": "boolean"}
                }
            }),
        },
        ToolSpec {
            name: "core_remove_preference".into(),
            description: "Delete a saved preference by its id, e.g. when the user says it no \
                longer applies. To change one, remove it and add the new version. Waits for \
                the user's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["preference_id"],
                "properties": { "preference_id": {"type": "string"} }
            }),
        },
        ToolSpec {
            name: "core_get_plan".into(),
            description: "Get the current plan for a day. `date` is YYYY-MM-DD; by default, \
                tomorrow if it has a plan, else today. Use when the user asks about a plan \
                that isn't already shown in your context."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "date": {"type": "string"} }
            }),
        },
        ToolSpec {
            name: "core_generate_plan".into(),
            description: "Generate a new plan for a day (`date` YYYY-MM-DD, default \
                tomorrow). Only when the user asks for a plan; it takes a while. Pass any \
                wishes for this plan only as `request`. To change an existing plan, use \
                core_revise_plan instead."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "date": {"type": "string"},
                    "request": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "core_revise_plan".into(),
            description: "Change an existing plan following the user's request (e.g. \
                'tomorrow's lunch is a company dinner'). Pass the user's request as \
                `instruction`. `date` defaults to tomorrow if it has a plan, else today."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["instruction"],
                "properties": {
                    "instruction": {"type": "string"},
                    "date": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "core_delete_entry".into(),
            description: "Soft-delete a previously logged entry (meal, weight, ...) by its \
                id. Use this when the user corrects or retracts something they reported, \
                before logging the corrected version. Entries from an earlier logical date \
                are refused unless `past_day` is true; set it only when the user explicitly \
                asks to change that earlier day's record. Such a delete then waits for the \
                user's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["entry_id"],
                "properties": {
                    "entry_id": {"type": "string"},
                    "past_day": {"type": "boolean"}
                }
            }),
        },
    ]
}

/// Which core tool calls wait for the user's approval (`crate::approvals`), with what the UI
/// shows for them:
/// - allergy changes: allergies are hard constraints the planner must never break, so a
///   prompt-injected removal would be dangerous;
/// - adding or removing a preference: preferences shape every later plan and are quoted in
///   every prompt, which makes them the place an injected instruction would try to persist;
/// - deleting an entry from an earlier day (today's corrections stay one step).
pub async fn approval_preview(
    ctx: &PluginContext,
    name: &str,
    args: &Value,
) -> Result<Option<Value>, PluginError> {
    match name {
        "core_update_allergies" => {
            let args: UpdateAllergiesArgs = parse(args)?;
            let trimmed = |terms: Vec<String>| -> Vec<String> {
                terms
                    .into_iter()
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            };
            let (add, remove) = (trimmed(args.add), trimmed(args.remove));
            if add.is_empty() && remove.is_empty() {
                return Ok(None);
            }
            Ok(Some(
                json!({"kind": "update_allergies", "add": add, "remove": remove}),
            ))
        }
        "core_add_preference" => {
            let args: AddPreferenceArgs = parse(args)?;
            Ok(Some(json!({
                "kind": "add_preference",
                "preference_kind": args.kind,
                "text": args.text.trim(),
                "valid_from": args.valid_from,
                "valid_until": args.valid_until,
                "permanent": args.permanent,
            })))
        }
        "core_remove_preference" => {
            let args: RemovePreferenceArgs = parse(args)?;
            // An unknown id runs (and fails) right away instead of asking about nothing.
            let Some(preference) = ctx.preferences.get(&args.preference_id).await? else {
                return Ok(None);
            };
            Ok(Some(json!({
                "kind": "remove_preference",
                "preference_kind": preference.kind,
                "text": preference.text,
            })))
        }
        "core_delete_entry" => {
            let args: DeleteEntryArgs = parse(args)?;
            // Without `past_day`, a past entry is refused by the tool itself (no approval to
            // ask for); today's entries need none.
            if !args.past_day {
                return Ok(None);
            }
            let Some(entry) = ctx.events.get(&args.entry_id).await? else {
                return Ok(None);
            };
            if entry.logical_date >= ctx.today().await? {
                return Ok(None);
            }
            let p = &entry.payload;
            let description = p
                .get("raw_text")
                .or_else(|| p.get("title"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    p.get("kg")
                        .and_then(Value::as_f64)
                        .map(|kg| format!("{kg} kg"))
                });
            Ok(Some(json!({
                "kind": "delete_past_entry",
                "event_type": entry.event_type,
                "logical_date": entry.logical_date.to_string(),
                "description": description,
            })))
        }
        _ => Ok(None),
    }
}

fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, PluginError> {
    serde_json::from_value(args.clone()).map_err(|e| PluginError::InvalidArgs(e.to_string()))
}

pub async fn call(
    ctx: &PluginContext,
    planner: &PlanService,
    name: &str,
    args: Value,
) -> Result<ToolOutput, PluginError> {
    match name {
        "core_log_weight" => log_weight(ctx, args).await,
        "core_update_allergies" => update_allergies(ctx, args).await,
        "core_add_preference" => add_preference(ctx, args).await,
        "core_remove_preference" => remove_preference(ctx, args).await,
        "core_get_plan" => get_plan(ctx, args).await,
        "core_generate_plan" => generate_plan(ctx, planner, args).await,
        "core_revise_plan" => revise_plan(ctx, planner, args).await,
        "core_delete_entry" => delete_entry(ctx, args).await,
        other => Err(PluginError::NotFound(format!(
            "unknown core tool {other:?}"
        ))),
    }
}

fn parse_date(s: &str) -> Result<NaiveDate, PluginError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|e| PluginError::InvalidArgs(format!("invalid date {s:?}: {e}")))
}

/// The explicit date, else tomorrow if it has a plan, else today (§7.8).
async fn plan_date_or_default(
    ctx: &PluginContext,
    date: Option<&str>,
) -> Result<NaiveDate, PluginError> {
    if let Some(s) = date {
        return parse_date(s);
    }
    let today = ctx.today().await?;
    let tomorrow = today
        .succ_opt()
        .ok_or_else(|| PluginError::Other("date out of range".into()))?;
    Ok(if ctx.plans.current(tomorrow).await?.is_some() {
        tomorrow
    } else {
        today
    })
}

/// Plans go back to the model in the same compact form the system prompt uses, which stays
/// well under the 4,000-char tool-result limit (§7.10) where the full JSON might not.
fn plan_json(plan: &StoredPlan) -> Value {
    json!({
        "date": plan.plan_date.to_string(),
        "version": plan.version,
        "status": plan.status,
        "plan": render_plan_compact("Plan", plan),
        "warnings": plan.warnings,
        "disclaimer": plan.content.disclaimer,
    })
}

#[derive(Deserialize)]
struct DateArgs {
    date: Option<String>,
}

async fn get_plan(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: DateArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let date = plan_date_or_default(ctx, args.date.as_deref()).await?;
    let plan = ctx.plans.current(date).await?;
    Ok(ToolOutput {
        content: match &plan {
            Some(plan) => plan_json(plan),
            None => json!({ "date": date.to_string(), "plan": null }),
        },
        summary: ToolSummary::new("core_plan_viewed").with("date", date),
    })
}

#[derive(Deserialize)]
struct GenerateArgs {
    date: Option<String>,
    request: Option<String>,
}

async fn generate_plan(
    ctx: &PluginContext,
    planner: &PlanService,
    args: Value,
) -> Result<ToolOutput, PluginError> {
    let args: GenerateArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let date = match args.date.as_deref() {
        Some(s) => parse_date(s)?,
        None => ctx
            .today()
            .await?
            .succ_opt()
            .ok_or_else(|| PluginError::Other("date out of range".into()))?,
    };
    let plan = planner
        .generate(ctx, date, PlanTrigger::Manual, args.request.as_deref())
        .await?;
    Ok(ToolOutput {
        content: plan_json(&plan),
        summary: ToolSummary::new("core_plan_generated")
            .with("date", date)
            .with("version", plan.version),
    })
}

#[derive(Deserialize)]
struct ReviseArgs {
    instruction: String,
    date: Option<String>,
}

async fn revise_plan(
    ctx: &PluginContext,
    planner: &PlanService,
    args: Value,
) -> Result<ToolOutput, PluginError> {
    let args: ReviseArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let date = plan_date_or_default(ctx, args.date.as_deref()).await?;
    let plan = planner.revise(ctx, date, &args.instruction).await?;
    Ok(ToolOutput {
        content: plan_json(&plan),
        summary: ToolSummary::new("core_plan_revised")
            .with("date", date)
            .with("version", plan.version),
    })
}

#[derive(Deserialize)]
struct LogWeightArgs {
    kg: f64,
    measured_at: Option<String>,
}

async fn log_weight(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: LogWeightArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;

    if !(30.0..=300.0).contains(&args.kg) {
        return Err(PluginError::InvalidArgs(format!(
            "kg must be within 30..=300, got {}",
            args.kg
        )));
    }

    let occurred_at = match args.measured_at {
        Some(s) => parse_local_datetime(&s)?,
        None => ctx.clock.now(),
    };

    let event = ctx
        .events
        .append(
            "body.weight_logged",
            occurred_at,
            json!({"kg": args.kg}),
            EventSource::Chat,
        )
        .await?;

    Ok(ToolOutput {
        content: json!({"entry_id": event.id, "kg": args.kg}),
        summary: ToolSummary::new("core_weight_logged").with("kg", args.kg),
    })
}

#[derive(Deserialize)]
struct UpdateAllergiesArgs {
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

/// Case-insensitive, so "Peanut" and "peanut" are one allergy.
fn same_term(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

async fn update_allergies(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: UpdateAllergiesArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let Some(mut profile) = ctx.profile.get().await? else {
        return Err(PluginError::NotFound(
            "profile (the user hasn't set one up yet)".into(),
        ));
    };

    let mut not_found = Vec::new();
    for term in &args.remove {
        let before = profile.allergies.len();
        profile.allergies.retain(|a| !same_term(a, term));
        if profile.allergies.len() == before {
            not_found.push(term.trim().to_string());
        }
    }
    for term in &args.add {
        let term = term.trim();
        if !term.is_empty() && !profile.allergies.iter().any(|a| same_term(a, term)) {
            profile.allergies.push(term.to_string());
        }
    }
    ctx.profile.set(&profile).await?;

    Ok(ToolOutput {
        content: json!({"allergies": profile.allergies, "not_found": not_found}),
        summary: ToolSummary::new("core_allergies_updated").with("allergies", &profile.allergies),
    })
}

#[derive(Deserialize)]
struct AddPreferenceArgs {
    kind: PreferenceKind,
    text: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
    #[serde(default)]
    permanent: bool,
}

async fn add_preference(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: AddPreferenceArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let new = NewPreference {
        kind: args.kind,
        text: args.text,
        valid_from: args.valid_from.as_deref().map(parse_date).transpose()?,
        valid_until: args.valid_until.as_deref().map(parse_date).transpose()?,
        permanent: args.permanent,
    };
    let preference = ctx.preferences.add(&new, ctx.today().await?).await?;
    Ok(ToolOutput {
        content: json!({"preference_id": preference.id}),
        summary: ToolSummary::new("core_preference_added")
            .with("preference_kind", preference.kind)
            .with("text", &preference.text)
            .with("valid_from", preference.valid_from)
            .with("valid_until", preference.valid_until),
    })
}

#[derive(Deserialize)]
struct RemovePreferenceArgs {
    preference_id: String,
}

async fn remove_preference(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: RemovePreferenceArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    ctx.preferences.remove(&args.preference_id).await?;
    Ok(ToolOutput {
        content: json!({"preference_id": args.preference_id}),
        summary: ToolSummary::new("core_preference_removed"),
    })
}

#[derive(Deserialize)]
struct DeleteEntryArgs {
    entry_id: String,
    #[serde(default)]
    past_day: bool,
}

/// Refuses entries from an earlier logical date unless `past_day` is set: a model that
/// mistakes today's report for a correction of yesterday's otherwise silently deletes
/// yesterday's data.
async fn delete_entry(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: DeleteEntryArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let today = ctx.today().await?;
    let Some(existing) = ctx.events.get(&args.entry_id).await? else {
        return Err(PluginError::NotFound(format!("entry {}", args.entry_id)));
    };
    if existing.logical_date < today && !args.past_day {
        return Err(PluginError::InvalidArgs(format!(
            "entry {} is from {}, not today ({today}). Not deleted. If the user is reporting \
             something new for today, log it without deleting anything. Only if they \
             explicitly asked to change the {} record, call again with past_day: true.",
            existing.id, existing.logical_date, existing.logical_date
        )));
    }
    let event = ctx.events.soft_delete(&args.entry_id).await?;
    Ok(ToolOutput {
        content: json!({
            "entry_id": event.id,
            "event_type": event.event_type,
            "logical_date": event.logical_date.to_string(),
        }),
        summary: ToolSummary::new("core_entry_deleted").with("event_type", &event.event_type),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanContent;

    /// Most tests here don't touch plan generation, so an empty planner is enough. Shadows
    /// the glob-imported `call`.
    async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
        super::call(ctx, &PlanService::new(vec![]), name, args).await
    }

    async fn insert_plan(ctx: &PluginContext, date: NaiveDate) {
        let content = PlanContent {
            summary: "测试".into(),
            sections: Default::default(),
            disclaimer: None,
        };
        ctx.plans
            .insert(date, PlanTrigger::Manual, None, &content, &[])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_plan_defaults_to_tomorrow_when_it_has_a_plan_else_today() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let today = ctx.today().await.unwrap();
        let tomorrow = today.succ_opt().unwrap();

        let out = call(&ctx, "core_get_plan", json!({})).await.unwrap();
        assert_eq!(
            out.content,
            json!({"date": today.to_string(), "plan": null})
        );

        insert_plan(&ctx, tomorrow).await;
        let out = call(&ctx, "core_get_plan", json!({})).await.unwrap();
        assert_eq!(out.content["date"], tomorrow.to_string());
        assert_eq!(out.content["version"], 1);
    }

    #[tokio::test]
    async fn generate_plan_with_nothing_to_plan_reaches_the_model_as_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_generate_plan", json!({})).await,
            Err(PluginError::NotFound(m)) if m.contains("nothing to plan")
        ));
    }

    #[tokio::test]
    async fn revise_plan_without_a_plan_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_revise_plan", json!({"instruction": "少吃点"})).await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn plan_tools_reject_malformed_dates() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_get_plan", json!({"date": "明天"})).await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn log_weight_appends_a_weight_event_defaulting_to_now() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let out = call(&ctx, "core_log_weight", json!({"kg": 62.5}))
            .await
            .unwrap();
        assert_eq!(out.content["kg"], 62.5);

        let latest = ctx
            .events
            .latest("body.weight_logged")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.payload, json!({"kg": 62.5}));
        assert_eq!(latest.occurred_at, ctx.clock.now());
    }

    #[tokio::test]
    async fn log_weight_uses_measured_at_when_given() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        call(
            &ctx,
            "core_log_weight",
            json!({"kg": 62.5, "measured_at": "2026-09-20T08:00:00"}),
        )
        .await
        .unwrap();

        let expected = parse_local_datetime("2026-09-20T08:00:00").unwrap();
        let latest = ctx
            .events
            .latest("body.weight_logged")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.occurred_at, expected);
    }

    #[tokio::test]
    async fn log_weight_rejects_out_of_range_kg() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_log_weight", json!({"kg": 29.9})).await,
            Err(PluginError::InvalidArgs(_))
        ));
        assert!(matches!(
            call(&ctx, "core_log_weight", json!({"kg": 300.1})).await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn log_weight_rejects_an_unparsable_measured_at() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(
                &ctx,
                "core_log_weight",
                json!({"kg": 60.0, "measured_at": "not-a-datetime"})
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    async fn set_profile(ctx: &PluginContext, allergies: &[&str]) {
        use crate::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
        let profile = Profile {
            display_name: None,
            sex: Sex::Female,
            birth_date: NaiveDate::from_ymd_opt(1996, 5, 1).unwrap(),
            height_cm: 165.0,
            activity_level: ActivityLevel::Moderate,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: allergies.iter().map(|a| a.to_string()).collect(),
            medical_notes: None,
            meals_per_day: 3,
        };
        ctx.profile.set(&profile).await.unwrap();
    }

    #[tokio::test]
    async fn update_allergies_adds_without_duplicates_and_removes() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        set_profile(&ctx, &["Peanut"]).await;

        let out = call(
            &ctx,
            "core_update_allergies",
            json!({"add": ["虾", "peanut", " "], "remove": ["牛奶"]}),
        )
        .await
        .unwrap();
        assert_eq!(out.content["allergies"], json!(["Peanut", "虾"]));
        assert_eq!(out.content["not_found"], json!(["牛奶"]));

        call(&ctx, "core_update_allergies", json!({"remove": ["PEANUT"]}))
            .await
            .unwrap();
        let profile = ctx.profile.get().await.unwrap().unwrap();
        assert_eq!(profile.allergies, vec!["虾"]);
    }

    #[tokio::test]
    async fn update_allergies_without_a_profile_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_update_allergies", json!({"add": ["虾"]})).await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn add_then_remove_preference_round_trips() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let out = call(
            &ctx,
            "core_add_preference",
            json!({
                "kind": "general",
                "text": "这周只能吃便利店",
                "valid_until": "2026-10-09"
            }),
        )
        .await
        .unwrap();
        assert_eq!(out.summary.kind(), "core_preference_added");
        assert_eq!(out.summary.0["preference_kind"], "general");
        assert_eq!(out.summary.0["text"], "这周只能吃便利店");
        assert_eq!(out.summary.0["valid_until"], "2026-10-09");
        let id = out.content["preference_id"].as_str().unwrap().to_string();

        let saved = ctx.preferences.list(50).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].valid_until, NaiveDate::from_ymd_opt(2026, 10, 9));

        call(&ctx, "core_remove_preference", json!({"preference_id": id}))
            .await
            .unwrap();
        assert!(ctx.preferences.list(50).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_preference_defaults_to_one_month_unless_permanent() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let today = ctx.today().await.unwrap();

        call(
            &ctx,
            "core_add_preference",
            json!({"kind": "general", "text": "多吃鱼"}),
        )
        .await
        .unwrap();
        call(
            &ctx,
            "core_add_preference",
            json!({"kind": "dislike", "text": "香菜", "permanent": true}),
        )
        .await
        .unwrap();

        let saved = ctx.preferences.list(50).await.unwrap();
        assert_eq!(saved[0].text, "香菜");
        assert_eq!(saved[0].valid_until, None);
        assert_eq!(saved[1].text, "多吃鱼");
        assert_eq!(
            saved[1].valid_until,
            crate::preferences::default_valid_until(today)
        );
    }

    #[tokio::test]
    async fn add_preference_rejects_bad_kinds_and_dates() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(
                &ctx,
                "core_add_preference",
                json!({"kind": "allergy", "text": "虾"})
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
        assert!(matches!(
            call(
                &ctx,
                "core_add_preference",
                json!({"kind": "dislike", "text": "香菜", "valid_until": "下周"})
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn remove_of_unknown_preference_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(
                &ctx,
                "core_remove_preference",
                json!({"preference_id": "nope"})
            )
            .await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn delete_entry_soft_deletes_the_event() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let event = ctx
            .events
            .append(
                "body.weight_logged",
                ctx.clock.now(),
                json!({"kg": 60.0}),
                EventSource::Chat,
            )
            .await
            .unwrap();

        call(&ctx, "core_delete_entry", json!({"entry_id": event.id}))
            .await
            .unwrap();
        assert_eq!(ctx.events.latest("body.weight_logged").await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_entry_reports_the_entry_logical_date() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let event = ctx
            .events
            .append(
                "body.weight_logged",
                ctx.clock.now(),
                json!({"kg": 60.0}),
                EventSource::Chat,
            )
            .await
            .unwrap();

        let out = call(&ctx, "core_delete_entry", json!({"entry_id": event.id}))
            .await
            .unwrap();
        assert_eq!(out.content["logical_date"], event.logical_date.to_string());
    }

    #[tokio::test]
    async fn delete_entry_refuses_an_earlier_day_entry_unless_past_day_is_set() {
        let env = crate::test_support::env().await;
        let ctx = &env.ctx;
        let event = ctx
            .events
            .append(
                "body.weight_logged",
                ctx.clock.now(),
                json!({"kg": 60.0}),
                EventSource::Chat,
            )
            .await
            .unwrap();
        env.clock.advance(chrono::Duration::days(1));

        let refused = call(ctx, "core_delete_entry", json!({"entry_id": event.id})).await;
        assert!(matches!(refused, Err(PluginError::InvalidArgs(m)) if m.contains("past_day")));
        assert!(ctx.events.get(&event.id).await.unwrap().is_some());

        call(
            ctx,
            "core_delete_entry",
            json!({"entry_id": event.id, "past_day": true}),
        )
        .await
        .unwrap();
        assert!(ctx.events.get(&event.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn only_a_past_day_delete_needs_approval() {
        let env = crate::test_support::env().await;
        let ctx = &env.ctx;
        let append = |kg: f64| {
            ctx.events.append(
                "body.weight_logged",
                ctx.clock.now(),
                json!({"kg": kg}),
                EventSource::Chat,
            )
        };
        let old = append(60.0).await.unwrap();
        env.clock.advance(chrono::Duration::days(1));
        let today = append(61.0).await.unwrap();

        let preview = |id: String, past_day: bool| async move {
            let args = json!({"entry_id": id, "past_day": past_day});
            approval_preview(ctx, "core_delete_entry", &args).await
        };
        assert_eq!(preview(today.id.clone(), true).await.unwrap(), None);
        // Refused by the tool itself, nothing to approve.
        assert_eq!(preview(old.id.clone(), false).await.unwrap(), None);
        assert_eq!(
            preview(old.id.clone(), true).await.unwrap(),
            Some(json!({
                "kind": "delete_past_entry",
                "event_type": "body.weight_logged",
                "logical_date": old.logical_date.to_string(),
                "description": "60 kg",
            }))
        );
    }

    #[tokio::test]
    async fn preference_and_allergy_changes_need_approval_but_reads_do_not() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let preview = |name: &'static str, args: Value| {
            let ctx = &ctx;
            async move { approval_preview(ctx, name, &args).await.unwrap() }
        };
        assert!(preview(
            "core_add_preference",
            json!({"kind": "general", "text": "x"})
        )
        .await
        .is_some());
        assert!(preview("core_update_allergies", json!({"add": ["虾"]}))
            .await
            .is_some());
        // Nothing to change: runs (as a no-op) without asking.
        assert!(preview("core_update_allergies", json!({"add": [" "]}))
            .await
            .is_none());
        // An unknown preference id runs, and fails, without asking about nothing.
        assert!(
            preview("core_remove_preference", json!({"preference_id": "nope"}))
                .await
                .is_none()
        );
        assert!(preview("core_get_plan", json!({})).await.is_none());
        assert!(preview("core_log_weight", json!({"kg": 60}))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn delete_entry_of_unknown_id_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_delete_entry", json!({"entry_id": "nope"})).await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn unknown_core_tool_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_no_such_tool", json!({})).await,
            Err(PluginError::NotFound(_))
        ));
    }
}
