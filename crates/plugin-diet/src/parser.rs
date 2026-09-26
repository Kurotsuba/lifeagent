//! Meal parser: free text → foods with grams, per `docs/PHASE1_SPEC.md` §8.4/§9.2.

use chrono::{DateTime, Local, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_llm::json::complete_json;
use lifeagent_llm::types::{Message, Role};
use serde::Deserialize;

use crate::model::MealType;
use crate::prompts::MEAL_PARSER;
use crate::sources::known_names;

/// How many known food names the parser prompt lists (§8.4).
const KNOWN_FOODS_LIMIT: u32 = 150;
const MAX_ITEMS: usize = 20;
const MIN_GRAMS: f64 = 1.0;
const MAX_GRAMS: f64 = 2000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMeal {
    pub meal_type: MealType,
    pub items: Vec<ParsedItem>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ParsedItem {
    pub name: String,
    pub grams: f64,
}

/// What the model returns. `meal_type` is read as a string so both JSON `null` and the
/// string `"null"` (the prompt's own notation) mean "not stated".
#[derive(Deserialize)]
struct RawParsedMeal {
    #[serde(default)]
    meal_type: Option<String>,
    items: Vec<ParsedItem>,
}

/// Parses `description` into items. The meal type comes from `hint` (the user said it), else
/// the parser, else the local time of `eaten_at`.
pub async fn parse_meal(
    ctx: &PluginContext,
    description: &str,
    hint: Option<MealType>,
    eaten_at: DateTime<Utc>,
) -> Result<ParsedMeal, PluginError> {
    let known = known_names(ctx, KNOWN_FOODS_LIMIT).await?;
    let known_json =
        serde_json::to_string(&known).map_err(|e| PluginError::Other(e.to_string()))?;
    let local = eaten_at.with_timezone(&Local);
    let prompt = MEAL_PARSER
        .replace("{known_foods}", &known_json)
        .replace(
            "{eaten_at_local}",
            &local.format("%Y-%m-%d %H:%M (%A)").to_string(),
        )
        // Last, so text in the user's description is never treated as a placeholder.
        .replace("{description}", description);

    let raw: RawParsedMeal = complete_json(
        &ctx.llm,
        Role::Parser,
        vec![Message::User { content: prompt }],
    )
    .await?;

    let items = validate_items(raw.items)?;
    let meal_type = hint
        .or_else(|| raw.meal_type.as_deref().and_then(MealType::parse))
        .unwrap_or_else(|| MealType::default_for(local.time()));
    Ok(ParsedMeal { meal_type, items })
}

/// §8.4: 1–20 items, each with a non-empty name and grams in 1–2000. Failures are
/// `InvalidArgs` so the conversation model can relay them.
pub fn validate_items(items: Vec<ParsedItem>) -> Result<Vec<ParsedItem>, PluginError> {
    if items.is_empty() {
        return Err(PluginError::InvalidArgs(
            "no food items found in the description".into(),
        ));
    }
    if items.len() > MAX_ITEMS {
        return Err(PluginError::InvalidArgs(format!(
            "too many items in one meal ({}, max {MAX_ITEMS}); log it as separate meals",
            items.len()
        )));
    }
    items
        .into_iter()
        .map(|item| {
            let name = item.name.trim().to_string();
            if name.is_empty() {
                return Err(PluginError::InvalidArgs("an item has no name".into()));
            }
            if !(MIN_GRAMS..=MAX_GRAMS).contains(&item.grams) {
                return Err(PluginError::InvalidArgs(format!(
                    "{name}: {} g is outside {MIN_GRAMS}–{MAX_GRAMS} g",
                    item.grams
                )));
            }
            Ok(ParsedItem {
                name,
                grams: item.grams,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, json_response, local_instant};
    use serde_json::json;

    fn item(name: &str, grams: f64) -> ParsedItem {
        ParsedItem {
            name: name.into(),
            grams,
        }
    }

    #[test]
    fn validate_accepts_1_to_20_items_within_gram_bounds() {
        assert!(validate_items(vec![item("米饭", 1.0), item("鸡蛋", 2000.0)]).is_ok());
        assert!(validate_items((0..20).map(|_| item("x", 10.0)).collect()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_too_many_and_out_of_range_grams() {
        assert!(validate_items(vec![]).is_err());
        assert!(validate_items((0..21).map(|_| item("x", 10.0)).collect()).is_err());
        assert!(validate_items(vec![item("x", 0.5)]).is_err());
        assert!(validate_items(vec![item("x", 2000.1)]).is_err());
        assert!(validate_items(vec![item("  ", 10.0)]).is_err());
    }

    #[tokio::test]
    async fn parser_meal_type_is_used_when_no_hint() {
        let env = env().await;
        env.mock.push_response(json_response(json!({
            "meal_type": "breakfast",
            "items": [{"name": "燕麦", "grams": 50}]
        })));

        let meal = parse_meal(&env.ctx, "早上吃了燕麦", None, local_instant(12, 0))
            .await
            .unwrap();

        assert_eq!(meal.meal_type, MealType::Breakfast);
        assert_eq!(meal.items, vec![item("燕麦", 50.0)]);
    }

    #[tokio::test]
    async fn hint_wins_over_the_parser() {
        let env = env().await;
        env.mock.push_response(json_response(json!({
            "meal_type": "breakfast",
            "items": [{"name": "燕麦", "grams": 50}]
        })));

        let meal = parse_meal(&env.ctx, "燕麦", Some(MealType::Snack), local_instant(8, 0))
            .await
            .unwrap();
        assert_eq!(meal.meal_type, MealType::Snack);
    }

    #[tokio::test]
    async fn falls_back_to_the_local_time_default() {
        let env = env().await;
        // Both spellings of "not stated" the model might use.
        env.mock.push_response(json_response(json!({
            "meal_type": null, "items": [{"name": "米饭", "grams": 150}]
        })));
        env.mock.push_response(json_response(json!({
            "meal_type": "null", "items": [{"name": "米饭", "grams": 150}]
        })));

        let at_lunch = parse_meal(&env.ctx, "米饭", None, local_instant(12, 40))
            .await
            .unwrap();
        let at_dinner = parse_meal(&env.ctx, "米饭", None, local_instant(18, 30))
            .await
            .unwrap();

        assert_eq!(at_lunch.meal_type, MealType::Lunch);
        assert_eq!(at_dinner.meal_type, MealType::Dinner);
    }

    #[tokio::test]
    async fn prompt_lists_known_foods_and_the_description() {
        let env = env().await;
        crate::sources::upsert_manual_food(&env.ctx, "熟米饭", crate::testutil::RICE)
            .await
            .unwrap();
        env.mock.push_response(json_response(json!({
            "meal_type": null, "items": [{"name": "熟米饭", "grams": 150}]
        })));

        parse_meal(&env.ctx, "一碗饭", None, local_instant(12, 0))
            .await
            .unwrap();

        let request = &env.mock.received_requests()[0];
        let Message::User { content } = &request.messages[0] else {
            panic!("expected a user message");
        };
        assert!(content.contains(r#"KNOWN_FOODS: ["熟米饭"]"#));
        assert!(content.contains("Description: 一碗饭"));
    }

    #[tokio::test]
    async fn invalid_items_from_the_model_are_invalid_args() {
        let env = env().await;
        env.mock
            .push_response(json_response(json!({"meal_type": null, "items": []})));

        assert!(matches!(
            parse_meal(&env.ctx, "喝了水", None, local_instant(12, 0)).await,
            Err(PluginError::InvalidArgs(_))
        ));
    }
}
