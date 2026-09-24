//! Core tool specs and handlers, per `docs/PHASE1_SPEC.md` §7.8.
//!
//! The plan tools (`core_get_plan`, `core_generate_plan`, `core_revise_plan`) are stubbed per
//! the M3 milestone note in `docs/PHASE1_SPEC.md` §14 ("core tools (except plan tools,
//! stubbed)") — they are not registered at all until the plan service lands in M5, since
//! there is nothing meaningful for them to do without it (see `docs/DECISIONS.md`).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::events::EventSource;
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
            name: "core_delete_entry".into(),
            description: "Soft-delete a previously logged entry (meal, weight, ...) by its \
                id. Use this when the user corrects or retracts something they reported, \
                before logging the corrected version."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["entry_id"],
                "properties": { "entry_id": {"type": "string"} }
            }),
        },
    ]
}

pub async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
    match name {
        "core_log_weight" => log_weight(ctx, args).await,
        "core_remember" => remember(ctx, args).await,
        "core_forget" => forget(ctx, args).await,
        "core_delete_entry" => delete_entry(ctx, args).await,
        other => Err(PluginError::NotFound(format!(
            "unknown core tool {other:?}"
        ))),
    }
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
}

async fn delete_entry(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: DeleteEntryArgs =
        serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;
    let event = ctx.events.soft_delete(&args.entry_id).await?;
    Ok(ToolOutput {
        content: json!({"entry_id": event.id, "event_type": event.event_type}),
        summary: "已删除记录".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
