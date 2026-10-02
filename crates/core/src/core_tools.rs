//! Core tool specs and handlers, per `docs/PHASE1_SPEC.md` §7.8.

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::context::render_plan_compact;
use crate::events::EventSource;
use crate::plan::{PlanService, PlanTrigger, StoredPlan};
use crate::plugin::{PluginContext, PluginError, ToolOutput};
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
            name: "core_remember".into(),
            description: "Save a durable preference or fact about the user that should \
                influence future plans (e.g. 'no cilantro', 'weekday lunch is at the company \
                canteen'). Do not use for one-off events."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["note"],
                "properties": { "note": {"type": "string"} }
            }),
        },
        ToolSpec {
            name: "core_forget".into(),
            description: "Delete a previously saved memory note by its id.".into(),
            parameters: json!({
                "type": "object",
                "required": ["note_id"],
                "properties": { "note_id": {"type": "string"} }
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
                tomorrow). Only when the user asks for a plan; it takes a while. To change \
                an existing plan, use core_revise_plan instead."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "date": {"type": "string"} }
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
                asks to change that earlier day's record."
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

pub async fn call(
    ctx: &PluginContext,
    planner: &PlanService,
    name: &str,
    args: Value,
) -> Result<ToolOutput, PluginError> {
    match name {
        "core_log_weight" => log_weight(ctx, args).await,
        "core_remember" => remember(ctx, args).await,
        "core_forget" => forget(ctx, args).await,
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
        summary: format!("查看了 {date} 的计划"),
    })
}

async fn generate_plan(
    ctx: &PluginContext,
    planner: &PlanService,
    args: Value,
) -> Result<ToolOutput, PluginError> {
    let args: DateArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let date = match args.date.as_deref() {
        Some(s) => parse_date(s)?,
        None => ctx
            .today()
            .await?
            .succ_opt()
            .ok_or_else(|| PluginError::Other("date out of range".into()))?,
    };
    let plan = planner.generate(ctx, date, PlanTrigger::Manual).await?;
    Ok(ToolOutput {
        content: plan_json(&plan),
        summary: format!("已生成 {date} 的计划（第 {} 版）", plan.version),
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
        summary: format!("已调整 {date} 的计划（第 {} 版）", plan.version),
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
        summary: format!("已记录体重 {} kg", args.kg),
    })
}

#[derive(Deserialize)]
struct RememberArgs {
    note: String,
}

async fn remember(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: RememberArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let note = ctx.memory.add(&args.note).await?;
    Ok(ToolOutput {
        content: json!({"note_id": note.id}),
        summary: format!("已记住：{}", note.note),
    })
}

#[derive(Deserialize)]
struct ForgetArgs {
    note_id: String,
}

async fn forget(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: ForgetArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    ctx.memory.forget(&args.note_id).await?;
    Ok(ToolOutput {
        content: json!({"note_id": args.note_id}),
        summary: "已忘记该记忆".to_string(),
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
        summary: "已删除记录".to_string(),
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

    #[tokio::test]
    async fn remember_then_forget_round_trips() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        let out = call(&ctx, "core_remember", json!({"note": "不吃香菜"}))
            .await
            .unwrap();
        let note_id = out.content["note_id"].as_str().unwrap().to_string();

        assert_eq!(ctx.memory.list(50).await.unwrap().len(), 1);

        call(&ctx, "core_forget", json!({"note_id": note_id}))
            .await
            .unwrap();
        assert_eq!(ctx.memory.list(50).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn forget_of_unknown_note_is_not_found() {
        let (ctx, _mock, _dir) = crate::test_support::context().await;
        assert!(matches!(
            call(&ctx, "core_forget", json!({"note_id": "nope"})).await,
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
