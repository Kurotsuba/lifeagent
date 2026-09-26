//! Diet tools, per `docs/PHASE1_SPEC.md` §8.5.

use chrono::NaiveDate;
use lifeagent_core::events::EventSource;
use lifeagent_core::plugin::{PluginContext, PluginError, ToolOutput};
use lifeagent_core::time::parse_local_datetime;
use lifeagent_llm::types::ToolSpec;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::foods::validate_per_100g;
use crate::intake::{self, MEAL_EVENT};
use crate::model::{Macros, MealItem, MealLogged, MealType};
use crate::parser::parse_meal;
use crate::resolver::resolve_many;
use crate::sources::{self, SourceChain};

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "diet_log_meal".into(),
            description: "Log a meal, snack or caloric drink the user reports having eaten. \
                Always pass the user's original wording in `description`, unchanged — the app \
                works out foods, portions and nutrition itself. Set `meal_type` only if the \
                user states it. Set `eaten_at` (local time, YYYY-MM-DDTHH:MM) only if the user \
                ate at a different time than now. When the user corrects a logged meal (wrong \
                portion or item), delete that entry with core_delete_entry and log it again; \
                this re-log is the one exception to the original wording: describe the \
                corrected meal item by item, keeping the name and grams of each item the \
                user did not correct from the earlier result (e.g. \"拉面(熟)150g、牛肉(熟)80g、\
                卤蛋2个\"), otherwise unchanged items get new guessed portions. \
                Quote the returned numbers; never compute your own."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["description"],
                "properties": {
                    "description": {"type": "string"},
                    "meal_type": {
                        "type": "string",
                        "enum": ["breakfast", "lunch", "dinner", "snack"]
                    },
                    "eaten_at": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "diet_get_intake".into(),
            description: "Get the meals logged on a day and the nutrition totals against \
                the day's targets (`date` YYYY-MM-DD, default today). Use when the user asks \
                what they ate or how much they have left. Past dates include plan-vs-actual."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "date": {"type": "string"} }
            }),
        },
        ToolSpec {
            name: "diet_update_food".into(),
            description: "Correct the per-100 g nutrition of a food in the local food table \
                when the user gives better values (e.g. from a package label). Creates the \
                food if it doesn't exist. Only meals logged afterwards use the new values; \
                meals already logged keep theirs."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["food_name", "kcal", "protein_g", "carbs_g", "fat_g"],
                "properties": {
                    "food_name": {"type": "string"},
                    "kcal": {"type": "number"},
                    "protein_g": {"type": "number"},
                    "carbs_g": {"type": "number"},
                    "fat_g": {"type": "number"},
                    "fiber_g": {"type": "number"}
                }
            }),
        },
        ToolSpec {
            name: "diet_add_alias".into(),
            description: "Add another name for a food already in the food table (e.g. \
                '卤蛋' for '卤鸡蛋'), so meals that use that name match it."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["food_name", "alias"],
                "properties": {
                    "food_name": {"type": "string"},
                    "alias": {"type": "string"}
                }
            }),
        },
    ]
}

pub async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
    match name {
        "diet_log_meal" => log_meal(ctx, args).await,
        "diet_get_intake" => get_intake(ctx, args).await,
        "diet_update_food" => update_food(ctx, args).await,
        "diet_add_alias" => add_alias(ctx, args).await,
        other => Err(PluginError::NotFound(format!(
            "unknown diet tool {other:?}"
        ))),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, PluginError> {
    serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))
}

#[derive(Deserialize)]
struct LogMealArgs {
    description: String,
    meal_type: Option<MealType>,
    eaten_at: Option<String>,
}

async fn log_meal(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: LogMealArgs = parse_args(args)?;
    let description = args.description.trim();
    if description.is_empty() {
        return Err(PluginError::InvalidArgs(
            "description must not be empty".into(),
        ));
    }
    let eaten_at = match &args.eaten_at {
        Some(s) => parse_local_datetime(s)?,
        None => ctx.clock.now(),
    };

    let parsed = parse_meal(ctx, description, args.meal_type, eaten_at).await?;
    let names: Vec<String> = parsed.items.iter().map(|i| i.name.clone()).collect();
    let foods = resolve_many(ctx, &names).await?;
    let items: Vec<MealItem> = parsed
        .items
        .iter()
        .zip(foods)
        .map(|(item, food)| MealItem {
            macros: food.per_100g.for_grams(item.grams),
            name: food.name,
            grams: item.grams,
            food_id: food.id,
            source: food.source_kind,
            source_id: Some(food.source_id),
            source_name: Some(food.source_name),
        })
        .collect();
    let meal = MealLogged {
        meal_type: parsed.meal_type,
        raw_text: description.to_string(),
        total: items.iter().map(|i| i.macros).sum(),
        items,
    };

    let payload = serde_json::to_value(&meal).map_err(|e| PluginError::Other(e.to_string()))?;
    let event = ctx
        .events
        .append(MEAL_EVENT, eaten_at, payload, EventSource::Chat)
        .await?;

    let day_total = intake::total(&intake::meals_on(ctx, event.logical_date).await?);
    let target = intake::targets_for(ctx, event.logical_date).await?;
    let content = json!({
        "entry_id": event.id,
        "meal_type": meal.meal_type,
        "items": meal.items.iter().map(MealItem::to_json).collect::<Vec<_>>(),
        "total": meal.total.to_json(),
        "day_total": day_total.to_json(),
        "day_target": target.map_or(Value::Null, |t| t.as_macros().to_json()),
        "remaining": target.map_or(Value::Null, |t| (t.as_macros() - day_total).to_json()),
    });
    Ok(ToolOutput {
        content,
        summary: format!(
            "已记录{} · {:.0} kcal",
            meal.meal_type.zh_label(),
            meal.total.kcal
        ),
    })
}

#[derive(Deserialize)]
struct GetIntakeArgs {
    date: Option<String>,
}

async fn get_intake(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: GetIntakeArgs = parse_args(args)?;
    let date = match args.date {
        Some(s) => NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
            .map_err(|e| PluginError::InvalidArgs(format!("invalid date {s:?}: {e}")))?,
        None => intake::today(ctx).await?,
    };
    Ok(ToolOutput {
        content: intake::intake_json(ctx, date).await?,
        summary: format!("查看了 {date} 的饮食记录"),
    })
}

#[derive(Deserialize)]
struct UpdateFoodArgs {
    food_name: String,
    kcal: f64,
    protein_g: f64,
    carbs_g: f64,
    fat_g: f64,
    fiber_g: Option<f64>,
}

async fn update_food(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: UpdateFoodArgs = parse_args(args)?;
    // Without a new fiber value, keep the food's current chain-winner value
    // (`docs/NUTRITION_SOURCES.md` §7.2).
    let fiber_g = match args.fiber_g {
        Some(f) => f,
        None => SourceChain::load(ctx)
            .await?
            .lookup(&args.food_name)
            .await?
            .map_or(0.0, |f| f.per_100g.fiber_g),
    };
    let per_100g = Macros {
        kcal: args.kcal,
        protein_g: args.protein_g,
        carbs_g: args.carbs_g,
        fat_g: args.fat_g,
        fiber_g,
    };
    // User values always win (§16), so no kcal correction — but reject impossible ones.
    validate_per_100g(&per_100g).map_err(PluginError::InvalidArgs)?;

    let food = sources::upsert_manual_food(ctx, &args.food_name, per_100g).await?;
    Ok(ToolOutput {
        content: json!({
            "food_id": food.id,
            "name": food.name,
            "per_100g": food.per_100g.to_json(),
            "note": "Saved. Only meals logged from now on use these values; meals already \
                     logged keep their numbers.",
        }),
        summary: format!("已更新食物：{}", food.name),
    })
}

#[derive(Deserialize)]
struct AddAliasArgs {
    food_name: String,
    alias: String,
}

async fn add_alias(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: AddAliasArgs = parse_args(args)?;
    let (food, alias) = sources::add_user_alias(ctx, &args.food_name, &args.alias).await?;
    Ok(ToolOutput {
        content: json!({ "food_id": food.id, "name": food.name, "alias": alias }),
        summary: format!("已添加别名：{} → {}", args.alias.trim(), food.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{upsert_manual_food, SourceChain};
    use crate::testutil::{env, estimate_responder, json_response, seed_person, EGG, RICE};

    fn parser_reply() -> lifeagent_llm::types::ChatResponse {
        json_response(json!({
            "meal_type": "lunch",
            "items": [{"name": "熟米饭", "grams": 200}, {"name": "鸡蛋", "grams": 100}]
        }))
    }

    #[tokio::test]
    async fn log_meal_parses_resolves_stores_and_reports_day_totals() {
        let env = env().await;
        seed_person(&env.ctx).await;
        env.mock.push_response(parser_reply());
        env.mock.push_closure(estimate_responder);
        env.mock.push_closure(estimate_responder);

        let out = call(
            &env.ctx,
            "diet_log_meal",
            json!({"description": "午饭一碗米饭两个鸡蛋"}),
        )
        .await
        .unwrap();

        // 200 g rice at 116/100 g + 100 g egg at 144/100 g.
        assert_eq!(out.content["total"]["kcal"], 376.0);
        assert_eq!(out.content["day_total"]["kcal"], 376.0);
        assert_eq!(out.content["items"][0]["estimated"], true);
        assert!(out.content["remaining"]["kcal"].is_number());
        assert_eq!(out.summary, "已记录午餐 · 376 kcal");

        let event = env.ctx.events.latest(MEAL_EVENT).await.unwrap().unwrap();
        let stored: MealLogged = serde_json::from_value(event.payload).unwrap();
        assert_eq!(stored.raw_text, "午饭一碗米饭两个鸡蛋");
        assert_eq!(stored.meal_type, MealType::Lunch);
        assert_eq!(stored.items.len(), 2);
    }

    #[tokio::test]
    async fn log_meal_uses_eaten_at_for_the_logical_date() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "熟米饭", RICE).await.unwrap();
        env.mock.push_response(json_response(json!({
            "meal_type": null, "items": [{"name": "熟米饭", "grams": 150}]
        })));
        let yesterday = intake::today(&env.ctx).await.unwrap().pred_opt().unwrap();
        let eaten_at = format!("{}T19:00", yesterday.format("%Y-%m-%d"));

        let out = call(
            &env.ctx,
            "diet_log_meal",
            json!({"description": "昨晚吃了米饭", "eaten_at": eaten_at}),
        )
        .await
        .unwrap();

        assert_eq!(out.content["meal_type"], "dinner");
        assert_eq!(
            intake::meals_on(&env.ctx, yesterday).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn get_intake_defaults_to_today_and_rejects_bad_dates() {
        let env = env().await;
        let out = call(&env.ctx, "diet_get_intake", json!({})).await.unwrap();
        assert_eq!(
            out.content["date"],
            intake::today(&env.ctx).await.unwrap().to_string()
        );

        assert!(matches!(
            call(&env.ctx, "diet_get_intake", json!({"date": "昨天"})).await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn update_food_creates_or_corrects_with_user_source() {
        let env = env().await;
        crate::sources::insert_estimate(&env.ctx, "鸡蛋", EGG)
            .await
            .unwrap();

        call(
            &env.ctx,
            "diet_update_food",
            json!({"food_name": "鸡蛋", "kcal": 150, "protein_g": 13, "carbs_g": 1, "fat_g": 10}),
        )
        .await
        .unwrap();

        let egg = SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup("鸡蛋")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(egg.source_kind, crate::model::SourceKind::Manual);
        assert_eq!(egg.per_100g.kcal, 150.0);
    }

    #[tokio::test]
    async fn update_food_rejects_impossible_values() {
        let env = env().await;
        assert!(matches!(
            call(
                &env.ctx,
                "diet_update_food",
                json!({"food_name": "x", "kcal": 100, "protein_g": 60, "carbs_g": 60, "fat_g": 0}),
            )
            .await,
            Err(PluginError::InvalidArgs(_))
        ));
    }

    #[tokio::test]
    async fn add_alias_then_the_alias_resolves() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "卤鸡蛋", EGG).await.unwrap();

        let out = call(
            &env.ctx,
            "diet_add_alias",
            json!({"food_name": "卤鸡蛋", "alias": "卤蛋"}),
        )
        .await
        .unwrap();

        assert_eq!(out.summary, "已添加别名：卤蛋 → 卤鸡蛋");
        assert!(SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup("卤蛋")
            .await
            .unwrap()
            .is_some());
    }
}
