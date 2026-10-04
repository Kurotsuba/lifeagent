//! Taking a logged meal's ingredients out of stock.
//!
//! Listens for the diet plugin's `diet.meal_logged` event: this crate depends only on that
//! event's JSON payload, never on the diet crate, so either plugin works without the other. The
//! model decides which stocked items the meal used and how much, in each item's own unit
//! (cooked rice -> raw rice, grams of egg -> eggs); Rust checks the indexes, caps each amount at
//! what's left and does the subtraction.

use lifeagent_core::events::Event;
use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_llm::json::complete_json;
use lifeagent_llm::types::{Message, Role};
use serde::Deserialize;

use crate::model::{StockItem, StockUse};
use crate::{settings, store};

pub const MEAL_EVENT: &str = "diet.meal_logged";

/// The parts of a `diet.meal_logged` payload this plugin reads. Unknown fields are ignored, and
/// missing optional ones default, so the diet plugin can grow its payload freely.
#[derive(Debug, Deserialize)]
struct MealPayload {
    #[serde(default)]
    raw_text: String,
    #[serde(default)]
    items: Vec<MealItemPayload>,
}

#[derive(Debug, Deserialize)]
struct MealItemPayload {
    name: String,
    /// `"food"` or `"dish"`; absent on old events, which are all foods.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    grams: Option<f64>,
    /// Bought ready to eat or from a branded dish (the parser's call); absent on old events.
    #[serde(default)]
    eaten_out: bool,
}

#[derive(Debug, Deserialize)]
struct MatchOutput {
    #[serde(default)]
    uses: Vec<MatchedUse>,
}

#[derive(Debug, Deserialize)]
struct MatchedUse {
    stock: usize,
    amount: f64,
}

const PROMPT: &str = r#"The user just logged a meal. Decide which of the foods in the user's home stock the meal used, and how much.

Meal, as the user described it: {raw_text}

Foods in the meal (weight as eaten):
{foods}

Home stock (index: name, amount left):
{stock}

Rules:
- Count a stock item only if the meal plausibly used it. If the user ate out or bought the food ready to eat (restaurant, canteen, convenience store, takeout, delivery), return no uses.
- Give each amount in the stock item's own unit. Convert as eaten to as stored: cooked rice or pasta weighs about 2.2 times its dry weight; "piece" means whole items (one egg is about 50 g).
- Never more than the amount left.

Return JSON: {"uses": [{"stock": <index>, "amount": <number>}]}"#;

/// Handles one `diet.meal_logged` event. Does nothing when auto-consume is off, stock is empty,
/// the meal has only restaurant/menu dishes or food eaten out (a chain item split into its
/// ingredients included), or the event was already handled.
pub async fn on_meal_logged(
    ctx: &PluginContext,
    event: &Event,
) -> Result<Vec<StockUse>, PluginError> {
    if !settings::get(ctx).await?.auto_consume || store::has_uses(ctx, &event.id).await? {
        return Ok(Vec::new());
    }
    let meal: MealPayload = serde_json::from_value(event.payload.clone())
        .map_err(|e| PluginError::Other(format!("unreadable meal payload: {e}")))?;
    let foods: Vec<&MealItemPayload> = meal
        .items
        .iter()
        .filter(|i| i.kind.as_deref() != Some("dish") && !i.eaten_out)
        .collect();
    if foods.is_empty() {
        return Ok(Vec::new());
    }
    let stock = store::list(ctx).await?;
    if stock.is_empty() {
        return Ok(Vec::new());
    }

    let prompt = PROMPT
        .replace("{foods}", &render_foods(&foods))
        .replace("{stock}", &render_stock(&stock))
        // User text last, so braces in it are never taken for a placeholder.
        .replace("{raw_text}", &meal.raw_text);
    let output: MatchOutput = complete_json(
        &ctx.llm,
        Role::Parser,
        vec![Message::User { content: prompt }],
    )
    .await?;

    let uses = checked_uses(&stock, output.uses);
    if uses.is_empty() {
        return Ok(Vec::new());
    }
    let recorded = store::consume(ctx, event, &uses).await?;
    tracing::debug!(event_id = %event.id, uses = recorded.len(), "meal took items out of stock");
    Ok(recorded)
}

fn render_foods(foods: &[&MealItemPayload]) -> String {
    foods
        .iter()
        .map(|f| match f.grams {
            Some(g) => format!("- {} {g:.0} g", f.name),
            None => format!("- {}", f.name),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_stock(stock: &[StockItem]) -> String {
    stock
        .iter()
        .enumerate()
        .map(|(i, s)| format!("- {i}: {}, {} {}", s.name, s.quantity, s.unit.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drops out-of-range indexes and non-positive amounts, merges repeats of one item and caps
/// each total at what's left.
fn checked_uses(stock: &[StockItem], uses: Vec<MatchedUse>) -> Vec<(StockItem, f64)> {
    let mut totals: Vec<(usize, f64)> = Vec::new();
    for u in uses {
        if u.stock >= stock.len() || !u.amount.is_finite() || u.amount <= 0.0 {
            continue;
        }
        match totals.iter_mut().find(|(i, _)| *i == u.stock) {
            Some((_, total)) => *total += u.amount,
            None => totals.push((u.stock, u.amount)),
        }
    }
    totals
        .into_iter()
        .map(|(i, total)| (stock[i].clone(), total.min(stock[i].quantity)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Unit;
    use crate::settings::StockSettings;
    use crate::store::NewStock;
    use crate::testutil::{env, json_response, meal_event};
    use serde_json::json;

    async fn stock(env: &lifeagent_core::test_support::TestEnv, name: &str, q: f64, unit: Unit) {
        store::add(
            &env.ctx,
            &NewStock {
                name: name.into(),
                quantity: q,
                unit,
                expires_on: None,
            },
        )
        .await
        .unwrap();
    }

    fn left(items: &[StockItem], name: &str) -> f64 {
        items
            .iter()
            .find(|i| i.name == name)
            .map_or(0.0, |i| i.quantity)
    }

    #[tokio::test]
    async fn takes_the_matched_amounts_out_of_stock() {
        let env = env().await;
        stock(&env, "米", 2000.0, Unit::G).await;
        stock(&env, "鸡蛋", 6.0, Unit::Piece).await;
        stock(&env, "牛奶", 1000.0, Unit::Ml).await;
        let event = meal_event(
            &env.ctx,
            "早饭米饭和两个鸡蛋",
            &[("熟米饭", 200.0), ("鸡蛋", 100.0)],
        )
        .await;
        // Stock is listed soonest-expiring then by name: 牛奶, 米, 鸡蛋 (all undated).
        env.mock.push_response(json_response(json!({
            "uses": [{"stock": 1, "amount": 90}, {"stock": 2, "amount": 2}, {"stock": 9, "amount": 1}]
        })));

        let used = on_meal_logged(&env.ctx, &event).await.unwrap();

        assert_eq!(used.len(), 2);
        let items = store::list(&env.ctx).await.unwrap();
        assert_eq!(left(&items, "米"), 1910.0);
        assert_eq!(left(&items, "鸡蛋"), 4.0);
        assert_eq!(left(&items, "牛奶"), 1000.0);
        let prompt = format!("{:?}", env.mock.received_requests()[0].messages);
        assert!(prompt.contains("熟米饭 200 g"));
        assert!(prompt.contains("1: 米, 2000 g"));
    }

    #[tokio::test]
    async fn a_redelivered_event_is_not_consumed_twice() {
        let env = env().await;
        stock(&env, "鸡蛋", 6.0, Unit::Piece).await;
        let event = meal_event(&env.ctx, "两个鸡蛋", &[("鸡蛋", 100.0)]).await;
        env.mock
            .push_response(json_response(json!({"uses": [{"stock": 0, "amount": 2}]})));

        on_meal_logged(&env.ctx, &event).await.unwrap();
        on_meal_logged(&env.ctx, &event).await.unwrap();

        assert_eq!(store::list(&env.ctx).await.unwrap()[0].quantity, 4.0);
        assert_eq!(env.mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn skips_the_model_when_off_empty_dishes_only_or_eaten_out() {
        let env = env().await;
        // Empty stock.
        let event = meal_event(&env.ctx, "米饭", &[("熟米饭", 200.0)]).await;
        assert!(on_meal_logged(&env.ctx, &event).await.unwrap().is_empty());

        // Only a chain-restaurant dish.
        stock(&env, "米", 2000.0, Unit::G).await;
        let dish = crate::testutil::event_with_payload(
            &env.ctx,
            json!({"raw_text": "松屋 牛めし", "items": [{"name": "牛めし（並盛）", "kind": "dish", "grams": 350}]}),
        )
        .await;
        assert!(on_meal_logged(&env.ctx, &dish).await.unwrap().is_empty());

        // A chain item with no menu record, split into ingredients the diet plugin marked as
        // eaten out.
        let split = crate::testutil::event_with_payload(
            &env.ctx,
            json!({"raw_text": "マクドナルドでビッグマック", "items": [
                {"name": "牛肉(熟)", "kind": "food", "grams": 90, "eaten_out": true},
                {"name": "バンズ", "kind": "food", "grams": 70, "eaten_out": true}
            ]}),
        )
        .await;
        assert!(on_meal_logged(&env.ctx, &split).await.unwrap().is_empty());

        // Switched off.
        settings::set(
            &env.ctx,
            &StockSettings {
                auto_consume: false,
                plan_from_stock: true,
            },
            env.ctx.clock.now(),
        )
        .await
        .unwrap();
        assert!(on_meal_logged(&env.ctx, &event).await.unwrap().is_empty());

        assert!(env.mock.received_requests().is_empty());
        assert_eq!(store::list(&env.ctx).await.unwrap()[0].quantity, 2000.0);
    }

    #[test]
    fn checked_uses_merges_repeats_and_caps_at_stock() {
        let item = StockItem {
            id: "a".into(),
            name: "鸡蛋".into(),
            quantity: 3.0,
            unit: Unit::Piece,
            expires_on: None,
            updated_at: String::new(),
        };
        let uses = checked_uses(
            &[item],
            vec![
                MatchedUse {
                    stock: 0,
                    amount: 2.0,
                },
                MatchedUse {
                    stock: 0,
                    amount: 2.0,
                },
                MatchedUse {
                    stock: 0,
                    amount: -1.0,
                },
                MatchedUse {
                    stock: 0,
                    amount: f64::INFINITY,
                },
            ],
        );
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].1, 3.0);
    }
}
