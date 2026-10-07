//! Stock tools for the conversation model.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PluginContext, PluginError, ToolOutput, ToolSummary};
use lifeagent_llm::types::ToolSpec;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::model::{StockItem, Unit};
use crate::store::{self, NewStock};

pub fn specs() -> Vec<ToolSpec> {
    let unit = json!({"type": "string", "enum": ["g", "ml", "piece"]});
    vec![
        ToolSpec {
            name: "stock_add".into(),
            description: "Add food the user bought or has at home to their stock (e.g. \"买了一盒\
                鸡蛋和1L牛奶\", \"冷蔵庫に豆腐が2丁ある\"). One entry per food, with a generic name \
                in the user's language, a quantity and a unit: g for things sold by weight, ml \
                for liquids, piece for things counted (eggs, tofu blocks, bread rolls). Convert \
                what the user says (1 L -> 1000 ml, 1 kg -> 1000 g, a carton of 10 eggs -> 10 \
                piece); if a pack size is unclear, assume a typical one and say so. Set \
                `expires_on` (YYYY-MM-DD) only if the user gives a date. Adding a food already \
                in stock with the same unit tops it up."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["items"],
                "properties": {
                    "items": {"type": "array", "items": {
                        "type": "object",
                        "required": ["name", "quantity", "unit"],
                        "properties": {
                            "name": {"type": "string"},
                            "quantity": {"type": "number"},
                            "unit": unit,
                            "expires_on": {"type": "string"}
                        }
                    }}
                }
            }),
        },
        ToolSpec {
            name: "stock_list".into(),
            description: "List the food in the user's stock with amounts left and best-before \
                dates. Use when the user asks what they have at home or what to cook."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "stock_update".into(),
            description: "Set how much of a stocked food is left, when the user says so (\"鸡蛋\
                只剩3个\", \"牛乳を捨てた\" -> 0). 0 means used up or thrown away. Not for meals: \
                meals logged with the diet tool take what they used out of stock by themselves. \
                Pass `unit` only if the food is stocked in more than one unit."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "quantity"],
                "properties": {
                    "name": {"type": "string"},
                    "quantity": {"type": "number"},
                    "unit": unit
                }
            }),
        },
    ]
}

pub async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
    match name {
        "stock_add" => add(ctx, args).await,
        "stock_list" => list(ctx).await,
        "stock_update" => update(ctx, args).await,
        other => Err(PluginError::NotFound(format!(
            "unknown stock tool {other:?}"
        ))),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, PluginError> {
    serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))
}

fn item_json(item: &StockItem) -> Value {
    json!({
        "name": item.name,
        "quantity": item.quantity,
        "unit": item.unit,
        "expires_on": item.expires_on.map(|d| d.to_string()),
    })
}

#[derive(Deserialize)]
struct AddArgs {
    items: Vec<AddItem>,
}

#[derive(Deserialize)]
struct AddItem {
    name: String,
    quantity: f64,
    unit: Unit,
    expires_on: Option<String>,
}

async fn add(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: AddArgs = parse_args(args)?;
    if args.items.is_empty() {
        return Err(PluginError::InvalidArgs("items must not be empty".into()));
    }
    // Check every entry before writing any, so a bad one doesn't leave half the list added.
    let mut new_items = Vec::with_capacity(args.items.len());
    for item in args.items {
        let expires_on = item
            .expires_on
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .map_err(|e| PluginError::InvalidArgs(format!("invalid expires_on {s:?}: {e}")))
            })
            .transpose()?;
        new_items.push(NewStock {
            name: item.name,
            quantity: item.quantity,
            unit: item.unit,
            expires_on,
        });
    }
    let mut added = Vec::with_capacity(new_items.len());
    for new in &new_items {
        added.push(store::add(ctx, new).await?);
    }
    let names = added.iter().map(|i| i.name.as_str()).collect::<Vec<_>>();
    Ok(ToolOutput {
        content: json!({ "stock": added.iter().map(item_json).collect::<Vec<_>>() }),
        summary: ToolSummary::new("stock_added").with("names", names),
    })
}

async fn list(ctx: &PluginContext) -> Result<ToolOutput, PluginError> {
    let items = store::list(ctx).await?;
    Ok(ToolOutput {
        content: json!({ "stock": items.iter().map(item_json).collect::<Vec<_>>() }),
        summary: ToolSummary::new("stock_viewed"),
    })
}

#[derive(Deserialize)]
struct UpdateArgs {
    name: String,
    quantity: f64,
    unit: Option<Unit>,
}

async fn update(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: UpdateArgs = parse_args(args)?;
    let mut matches = store::find_by_name(ctx, &args.name).await?;
    if let Some(unit) = args.unit {
        matches.retain(|i| i.unit == unit);
    }
    let item = match matches.len() {
        1 => matches.remove(0),
        0 => {
            let names: Vec<String> = store::list(ctx)
                .await?
                .into_iter()
                .map(|i| i.name)
                .collect();
            return Err(PluginError::NotFound(format!(
                "{:?} is not in stock; stocked items: {}",
                args.name.trim(),
                if names.is_empty() {
                    "none".into()
                } else {
                    names.join(", ")
                }
            )));
        }
        _ => {
            return Err(PluginError::InvalidArgs(format!(
                "{:?} is stocked in more than one unit; pass unit",
                args.name.trim()
            )))
        }
    };
    let item = store::set_quantity(ctx, &item.id, args.quantity).await?;
    Ok(ToolOutput {
        content: json!({ "stock": item_json(&item) }),
        summary: ToolSummary::new("stock_updated").with("name", &item.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env;

    #[tokio::test]
    async fn add_then_list_then_update() {
        let env = env().await;
        let out = call(
            &env.ctx,
            "stock_add",
            json!({"items": [
                {"name": "鸡蛋", "quantity": 10, "unit": "piece", "expires_on": "2026-10-15"},
                {"name": "牛奶", "quantity": 1000, "unit": "ml"}
            ]}),
        )
        .await
        .unwrap();
        assert_eq!(
            out.summary,
            ToolSummary::new("stock_added").with("names", ["鸡蛋", "牛奶"])
        );

        let out = call(&env.ctx, "stock_list", json!({})).await.unwrap();
        assert_eq!(out.content["stock"][0]["name"], "鸡蛋");
        assert_eq!(out.content["stock"][0]["expires_on"], "2026-10-15");

        call(
            &env.ctx,
            "stock_update",
            json!({"name": "牛奶", "quantity": 0}),
        )
        .await
        .unwrap();
        let out = call(&env.ctx, "stock_list", json!({})).await.unwrap();
        assert_eq!(out.content["stock"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn add_writes_nothing_if_any_entry_is_bad() {
        let env = env().await;
        let result = call(
            &env.ctx,
            "stock_add",
            json!({"items": [
                {"name": "鸡蛋", "quantity": 10, "unit": "piece"},
                {"name": "牛奶", "quantity": 1, "unit": "ml", "expires_on": "next week"}
            ]}),
        )
        .await;
        assert!(matches!(result, Err(PluginError::InvalidArgs(_))));
        assert!(store::list(&env.ctx).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn update_names_what_is_stocked_when_the_item_is_unknown_or_ambiguous() {
        let env = env().await;
        call(
            &env.ctx,
            "stock_add",
            json!({"items": [
                {"name": "鸡蛋", "quantity": 10, "unit": "piece"},
                {"name": "鸡蛋", "quantity": 300, "unit": "g"}
            ]}),
        )
        .await
        .unwrap();

        match call(
            &env.ctx,
            "stock_update",
            json!({"name": "豆腐", "quantity": 1}),
        )
        .await
        {
            Err(PluginError::NotFound(msg)) => assert!(msg.contains("鸡蛋"), "{msg}"),
            other => panic!("unexpected: {:?}", other.map(|o| o.content)),
        }
        assert!(matches!(
            call(
                &env.ctx,
                "stock_update",
                json!({"name": "鸡蛋", "quantity": 1})
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
        let out = call(
            &env.ctx,
            "stock_update",
            json!({"name": "鸡蛋", "quantity": 4, "unit": "piece"}),
        )
        .await
        .unwrap();
        assert_eq!(out.content["stock"]["quantity"], 4.0);
    }
}
