//! Meal parser: free text → dishes with ingredient breakdowns, per `docs/DISH_NUTRITION.md` §5.1
//! (replacing the flat item shape of `docs/PHASE1_SPEC.md` §8.4/§9.2).

use chrono::{DateTime, Local, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError};
use lifeagent_llm::json::complete_json;
use lifeagent_llm::types::{Message, Role};
use serde::Deserialize;

use crate::foods::normalize;
use crate::model::MealType;
use crate::prompts::MEAL_PARSER;
use crate::sources::{known_names, SourceChain};

/// How many known food names the parser prompt lists (§8.4).
const KNOWN_FOODS_LIMIT: u32 = 150;
/// §5.1: `{known_brands}` and `{known_dishes}` limits.
const KNOWN_BRANDS_LIMIT: u32 = 50;
const KNOWN_DISHES_LIMIT: u32 = 100;

const MAX_DISHES: usize = 20;
const MIN_SERVINGS: f64 = 0.25;
const MAX_SERVINGS: f64 = 10.0;
const MIN_DISH_GRAMS: f64 = 1.0;
const MAX_DISH_GRAMS: f64 = 3000.0;
const MAX_COMPONENTS_PER_DISH: usize = 20;
const MAX_TOTAL_COMPONENTS: usize = 40;
const MAX_ALT_NAMES: usize = 2;
const MIN_COMPONENT_GRAMS: f64 = 1.0;
const MAX_COMPONENT_GRAMS: f64 = 2000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMeal {
    pub meal_type: MealType,
    pub dishes: Vec<ParsedDish>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ParsedComponent {
    pub name: String,
    pub grams: f64,
    /// Up to 2 other names for the same food, used only as search queries for the match step
    /// (`docs/FOOD_MATCHING.md` §3).
    #[serde(default)]
    pub alt_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ParsedDish {
    pub name: String,
    #[serde(default)]
    pub brand: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default = "default_servings")]
    pub servings: f64,
    #[serde(default)]
    pub grams: Option<f64>,
    pub components: Vec<ParsedComponent>,
}

fn default_servings() -> f64 {
    1.0
}

/// What the model returns. `meal_type` is read as a string so both JSON `null` and the
/// string `"null"` (the prompt's own notation) mean "not stated".
#[derive(Deserialize)]
struct RawParsedMeal {
    #[serde(default)]
    meal_type: Option<String>,
    dishes: Vec<ParsedDish>,
}

/// Parses `description` into dishes. The meal type comes from `hint` (the user said it), else
/// the parser, else the local time of `eaten_at`.
pub async fn parse_meal(
    ctx: &PluginContext,
    description: &str,
    hint: Option<MealType>,
    eaten_at: DateTime<Utc>,
) -> Result<ParsedMeal, PluginError> {
    let known_foods = known_names(ctx, KNOWN_FOODS_LIMIT).await?;
    let chain = SourceChain::load(ctx).await?;
    let known_brands = chain.known_brands(KNOWN_BRANDS_LIMIT).await?;
    let known_dishes = chain.known_dishes(KNOWN_DISHES_LIMIT).await?;
    let to_json =
        |v: &Vec<String>| serde_json::to_string(v).map_err(|e| PluginError::Other(e.to_string()));
    let local = eaten_at.with_timezone(&Local);
    let prompt = MEAL_PARSER
        .replace("{known_foods}", &to_json(&known_foods)?)
        .replace("{known_brands}", &to_json(&known_brands)?)
        .replace("{known_dishes}", &to_json(&known_dishes)?)
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

    let dishes = validate_dishes(raw.dishes)?;
    let meal_type = hint
        .or_else(|| raw.meal_type.as_deref().and_then(MealType::parse))
        .unwrap_or_else(|| MealType::default_for(local.time()));
    Ok(ParsedMeal { meal_type, dishes })
}

/// §5.1's validation table. Failures are `InvalidArgs` so the conversation model can relay them.
pub fn validate_dishes(dishes: Vec<ParsedDish>) -> Result<Vec<ParsedDish>, PluginError> {
    if dishes.is_empty() {
        return Err(PluginError::InvalidArgs(
            "no dishes found in the description".into(),
        ));
    }
    if dishes.len() > MAX_DISHES {
        return Err(PluginError::InvalidArgs(format!(
            "too many dishes in one meal ({}, max {MAX_DISHES}); log it as separate meals",
            dishes.len()
        )));
    }

    let mut total_components = 0usize;
    let mut out = Vec::with_capacity(dishes.len());
    for dish in dishes {
        let name = dish.name.trim().to_string();
        if name.is_empty() {
            return Err(PluginError::InvalidArgs("a dish has no name".into()));
        }
        if !(MIN_SERVINGS..=MAX_SERVINGS).contains(&dish.servings) {
            return Err(PluginError::InvalidArgs(format!(
                "{name}: servings {} is outside {MIN_SERVINGS}–{MAX_SERVINGS}",
                dish.servings
            )));
        }
        if let Some(g) = dish.grams {
            if !(MIN_DISH_GRAMS..=MAX_DISH_GRAMS).contains(&g) {
                return Err(PluginError::InvalidArgs(format!(
                    "{name}: {g} g is outside {MIN_DISH_GRAMS}–{MAX_DISH_GRAMS} g"
                )));
            }
        }
        if dish.components.is_empty() || dish.components.len() > MAX_COMPONENTS_PER_DISH {
            return Err(PluginError::InvalidArgs(format!(
                "{name}: must have 1–{MAX_COMPONENTS_PER_DISH} components, got {}",
                dish.components.len()
            )));
        }

        let mut components = Vec::with_capacity(dish.components.len());
        for c in dish.components {
            let cname = c.name.trim().to_string();
            if cname.is_empty() {
                return Err(PluginError::InvalidArgs(format!(
                    "{name}: a component has no name"
                )));
            }
            if !(MIN_COMPONENT_GRAMS..=MAX_COMPONENT_GRAMS).contains(&c.grams) {
                return Err(PluginError::InvalidArgs(format!(
                    "{cname}: {} g is outside {MIN_COMPONENT_GRAMS}–{MAX_COMPONENT_GRAMS} g",
                    c.grams
                )));
            }
            let alt_names = clean_alt_names(&cname, c.alt_names);
            components.push(ParsedComponent {
                name: cname,
                grams: c.grams,
                alt_names,
            });
        }
        total_components += components.len();

        out.push(ParsedDish {
            name,
            brand: dish
                .brand
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty()),
            size: dish
                .size
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            servings: dish.servings,
            grams: dish.grams,
            components,
        });
    }

    if total_components > MAX_TOTAL_COMPONENTS {
        return Err(PluginError::InvalidArgs(format!(
            "too many components across the meal ({total_components}, max {MAX_TOTAL_COMPONENTS})"
        )));
    }
    Ok(out)
}

/// `docs/FOOD_MATCHING.md` §3: trims, drops blanks, duplicates and names equal to the component's
/// own, and keeps the first 2. Never an error: alternate names are only a search aid.
fn clean_alt_names(name: &str, alt_names: Vec<String>) -> Vec<String> {
    let name_key = normalize(name);
    let mut out: Vec<String> = Vec::new();
    for alt in alt_names {
        let alt = alt.trim().to_string();
        let key = normalize(&alt);
        if key.is_empty() || key == name_key || out.iter().any(|o| normalize(o) == key) {
            continue;
        }
        out.push(alt);
        if out.len() == MAX_ALT_NAMES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, json_response, local_instant};
    use serde_json::json;

    fn dish(name: &str, grams: f64) -> ParsedDish {
        ParsedDish {
            name: name.into(),
            brand: None,
            size: None,
            servings: 1.0,
            grams: None,
            components: vec![ParsedComponent {
                name: name.into(),
                grams,
                alt_names: vec![],
            }],
        }
    }

    #[test]
    fn validate_accepts_1_to_20_dishes_within_bounds() {
        assert!(validate_dishes(vec![dish("米饭", 1.0), dish("鸡蛋", 2000.0)]).is_ok());
        assert!(validate_dishes((0..20).map(|_| dish("x", 10.0)).collect()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_too_many_dishes_and_out_of_range_grams() {
        assert!(validate_dishes(vec![]).is_err());
        assert!(validate_dishes((0..21).map(|_| dish("x", 10.0)).collect()).is_err());
        assert!(validate_dishes(vec![dish("x", 0.5)]).is_err());
        assert!(validate_dishes(vec![dish("x", 2000.1)]).is_err());
        assert!(validate_dishes(vec![dish("  ", 10.0)]).is_err());
    }

    #[test]
    fn validate_rejects_out_of_range_servings_and_dish_grams() {
        let mut d = dish("牛めし", 300.0);
        d.servings = 0.1;
        assert!(validate_dishes(vec![d]).is_err());

        let mut d = dish("牛めし", 300.0);
        d.servings = 11.0;
        assert!(validate_dishes(vec![d]).is_err());

        let mut d = dish("牛めし", 300.0);
        d.grams = Some(3001.0);
        assert!(validate_dishes(vec![d]).is_err());
    }

    #[test]
    fn validate_rejects_too_many_components_per_dish_and_per_meal() {
        let mut many_components = dish("x", 10.0);
        many_components.components = (0..21)
            .map(|_| ParsedComponent {
                name: "y".into(),
                grams: 10.0,
                alt_names: vec![],
            })
            .collect();
        assert!(validate_dishes(vec![many_components]).is_err());

        // 3 dishes x 15 components = 45 > 40.
        let mut spread = dish("x", 10.0);
        spread.components = (0..15)
            .map(|_| ParsedComponent {
                name: "y".into(),
                grams: 10.0,
                alt_names: vec![],
            })
            .collect();
        assert!(validate_dishes(vec![spread.clone(), spread.clone(), spread]).is_err());
    }

    #[test]
    fn validate_trims_blank_brand_and_size_to_none() {
        let mut d = dish("牛めし", 300.0);
        d.brand = Some("  ".into());
        d.size = Some("".into());
        let out = validate_dishes(vec![d]).unwrap();
        assert_eq!(out[0].brand, None);
        assert_eq!(out[0].size, None);
    }

    fn dish_with_alts(name: &str, alts: &[&str]) -> ParsedDish {
        let mut d = dish(name, 100.0);
        d.components[0].alt_names = alts.iter().map(|a| a.to_string()).collect();
        d
    }

    #[test]
    fn validate_keeps_at_most_two_alt_names_and_drops_blanks_duplicates_and_the_name_itself() {
        let d = dish_with_alts("土豆", &[" ", "土豆", "马铃薯", " 马铃薯 ", "洋芋", "地蛋"]);

        let out = validate_dishes(vec![d]).unwrap();

        assert_eq!(out[0].components[0].alt_names, vec!["马铃薯", "洋芋"]);
    }

    #[test]
    fn validate_treats_alt_names_that_differ_only_by_case_or_trailing_punctuation_as_duplicates() {
        let d = dish_with_alts("Tomato", &["tomato。", "番茄", "番茄，"]);

        let out = validate_dishes(vec![d]).unwrap();

        assert_eq!(out[0].components[0].alt_names, vec!["番茄"]);
    }

    #[tokio::test]
    async fn alt_names_are_read_from_the_model_output_and_are_optional() {
        let env = env().await;
        env.mock.push_response(json_response(json!({
            "meal_type": null,
            "dishes": [{"name": "炒土豆", "components": [
                {"name": "土豆", "grams": 150, "alt_names": ["马铃薯"]},
                {"name": "食用油", "grams": 5}
            ]}]
        })));

        let meal = parse_meal(&env.ctx, "炒土豆", None, local_instant(12, 0))
            .await
            .unwrap();

        let c = &meal.dishes[0].components;
        assert_eq!(c[0].alt_names, vec!["马铃薯"]);
        assert!(c[1].alt_names.is_empty());
    }

    #[test]
    fn the_prompt_describes_alt_names() {
        assert!(MEAL_PARSER.contains(r#""alt_names""#));
        assert!(MEAL_PARSER.contains("up to 2 other"));
    }

    #[tokio::test]
    async fn parser_meal_type_is_used_when_no_hint() {
        let env = env().await;
        env.mock.push_response(json_response(json!({
            "meal_type": "breakfast",
            "dishes": [{"name": "燕麦", "components": [{"name": "燕麦", "grams": 50}]}]
        })));

        let meal = parse_meal(&env.ctx, "早上吃了燕麦", None, local_instant(12, 0))
            .await
            .unwrap();

        assert_eq!(meal.meal_type, MealType::Breakfast);
        assert_eq!(meal.dishes, vec![dish("燕麦", 50.0)]);
    }

    #[tokio::test]
    async fn hint_wins_over_the_parser() {
        let env = env().await;
        env.mock.push_response(json_response(json!({
            "meal_type": "breakfast",
            "dishes": [{"name": "燕麦", "components": [{"name": "燕麦", "grams": 50}]}]
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
            "meal_type": null,
            "dishes": [{"name": "米饭", "components": [{"name": "米饭", "grams": 150}]}]
        })));
        env.mock.push_response(json_response(json!({
            "meal_type": "null",
            "dishes": [{"name": "米饭", "components": [{"name": "米饭", "grams": 150}]}]
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
    async fn prompt_lists_known_foods_brands_dishes_and_the_description() {
        let env = env().await;
        crate::sources::upsert_manual_food(&env.ctx, "熟米饭", crate::testutil::RICE)
            .await
            .unwrap();
        crate::sources::upsert_manual_dish(
            &env.ctx,
            "A定食（大）",
            None,
            crate::testutil::RICE,
            None,
        )
        .await
        .unwrap();
        crate::sources::upsert_manual_dish(
            &env.ctx,
            "牛めし（並盛）",
            Some("松屋"),
            crate::testutil::RICE,
            None,
        )
        .await
        .unwrap();
        env.mock.push_response(json_response(json!({
            "meal_type": null,
            "dishes": [{"name": "熟米饭", "components": [{"name": "熟米饭", "grams": 150}]}]
        })));

        parse_meal(&env.ctx, "一碗饭", None, local_instant(12, 0))
            .await
            .unwrap();

        let request = &env.mock.received_requests()[0];
        let Message::User { content } = &request.messages[0] else {
            panic!("expected a user message");
        };
        assert!(content.contains(r#"KNOWN_FOODS: ["熟米饭"]"#));
        assert!(content.contains(r#"KNOWN_BRANDS: ["松屋"]"#));
        assert!(content.contains(r#"KNOWN_DISHES: ["A定食（大）"]"#));
        assert!(content.contains("Description: 一碗饭"));
    }

    #[tokio::test]
    async fn invalid_dishes_from_the_model_are_invalid_args() {
        let env = env().await;
        env.mock
            .push_response(json_response(json!({"meal_type": null, "dishes": []})));

        assert!(matches!(
            parse_meal(&env.ctx, "喝了水", None, local_instant(12, 0)).await,
            Err(PluginError::InvalidArgs(_))
        ));
    }
}
