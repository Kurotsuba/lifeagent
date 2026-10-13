//! Diet tools, per `docs/PHASE1_SPEC.md` §8.5.

use chrono::NaiveDate;
use lifeagent_core::events::EventSource;
use lifeagent_core::plugin::{PluginContext, PluginError, ToolOutput, ToolSummary};
use lifeagent_core::time::parse_local_datetime;
use lifeagent_llm::types::ToolSpec;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::dish_resolver::{self, DishOutcome};
use crate::foods::{kcal_disagrees_with_macros, normalize, validate_per_100g};
use crate::intake::{self, MEAL_EVENT};
use crate::model::{ItemKind, LabelNutrition, Macros, MealItem, MealLogged, MealType};
use crate::parser::parse_meal;
use crate::resolver::{resolve_many, FoodQuery};
use crate::sources::{self, SourceChain};

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "diet_calculate_label".into(),
            description: "Read-only nutrition calculation for a portion from values the user \
                explicitly supplies on a per-100 g label. Use for questions such as '10 g \
                protein per 100 g, how much in 150 g?', even if the food has not been eaten. \
                Supply only known label nutrients; do not invent missing values or use this \
                to estimate food nutrition. Energy is kcal, nutrient and portion quantities \
                are grams. If the label is per serving or per 100 ml, ask for the matching \
                weight basis instead of assuming ml equals g. This never records a meal, \
                updates food data, or calls a model. Quote the returned values and their \
                user-label source; do not call a write tool just to answer a label question."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["basis", "portion_grams", "per_100g"],
                "additionalProperties": false,
                "properties": {
                    "basis": {"type": "string", "enum": ["per_100g"]},
                    "portion_grams": {"type": "number"},
                    "per_100g": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "kcal": {"type": "number"},
                            "protein_g": {"type": "number"},
                            "carbs_g": {"type": "number"},
                            "fat_g": {"type": "number"},
                            "fiber_g": {"type": "number"}
                        }
                    }
                }
            }),
        },
        ToolSpec {
            name: "diet_log_meal".into(),
            description: "Log a meal, snack or caloric drink the user reports having eaten. \
                Only log the user's own actual consumption: a label question, quotation, \
                translation, hypothetical example or plan to eat is not a meal report. Do \
                not log when the user asks only for information or says not to record. \
                Always pass the user's original wording in `description`, unchanged — the app \
                works out dishes, foods, portions and nutrition itself. The app has its own \
                menu and food data (chain restaurants, canteen set meals, imported tables) \
                that you can't see, so call this tool even for items you don't recognize, \
                such as a canteen set meal (\"A定食（大）\") or a menu item name; never ask the \
                user for ingredients or say there's no data before logging. Set `meal_type` only if \
                the user states it. Set `eaten_at` (local time, YYYY-MM-DDTHH:MM) only if the \
                user ate at a different time than now. When the user corrects a logged meal \
                (wrong portion or item), delete that entry with core_delete_entry and log it \
                again; this re-log is the one exception to the original wording: describe the \
                corrected meal item by item, keeping the name and grams (or, for a chain menu \
                item, the brand, name and servings) of each item the user did not correct from \
                the earlier result (e.g. \"拉面(熟)150g、牛肉(熟)80g、卤蛋2个\" or \"松屋 牛めし\
                （並盛）1份、味噌汁1份\"), otherwise unchanged items get new guessed portions. \
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
                meals already logged keep theirs. A large change, or kcal that doesn't match the \
                macros, waits for the user's approval in the app."
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
            name: "diet_update_dish".into(),
            description: "Correct the per-serving nutrition of a chain restaurant / menu item \
                (a 'dish', not a per-100 g food) when the user gives better values, e.g. from a \
                receipt or the chain's own site. Creates the dish if it doesn't exist. Pass \
                `brand` when the item belongs to a specific chain; omit it only for a brandless \
                dish (e.g. a canteen item). Only meals logged afterwards use the new values; \
                meals already logged keep theirs. A large change, or kcal that doesn't match the \
                macros, waits for the user's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["dish_name", "kcal", "protein_g", "carbs_g", "fat_g"],
                "properties": {
                    "dish_name": {"type": "string"},
                    "brand": {"type": "string"},
                    "kcal": {"type": "number"},
                    "protein_g": {"type": "number"},
                    "carbs_g": {"type": "number"},
                    "fat_g": {"type": "number"},
                    "fiber_g": {"type": "number"},
                    "serving_g": {"type": "number"}
                }
            }),
        },
        ToolSpec {
            name: "diet_remove_alias".into(),
            description: "Remove a name redirect, so that name is looked up on its own again \
                (and estimated if it is not in the food table). Use when the user says a food \
                was matched to the wrong entry, e.g. '熟米饭不是那个稻米饭(蒸)，按估算算'. The \
                name is not matched to a table entry again until the imported food data changes. \
                Waits for the user's approval in the app."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["alias"],
                "properties": { "alias": {"type": "string"} }
            }),
        },
        ToolSpec {
            name: "diet_add_alias".into(),
            description: "Add another name for a food already in the food table (e.g. \
                '卤蛋' for '卤鸡蛋'), so meals that use that name match it. Waits for the user's \
                approval in the app."
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

/// Below these, a change to a food's per-100 g values never counts as large, however big in
/// percent (cucumber 14 → 18 kcal is a label correction, not an attack).
const FOOD_FLOOR: ChangeFloor = ChangeFloor {
    kcal: 20.0,
    grams: 5.0,
};
/// The same for a dish's per-serving values, which run several times larger.
const DISH_FLOOR: ChangeFloor = ChangeFloor {
    kcal: 50.0,
    grams: 5.0,
};
/// A change larger than this share of the current value counts as large.
const LARGE_CHANGE_SHARE: f64 = 0.3;

struct ChangeFloor {
    kcal: f64,
    grams: f64,
}

/// Which diet tool calls wait for the user's approval (`lifeagent_core::approvals`). Food and
/// dish values and name redirects feed every later meal total and plan, so one injected
/// "rice = 10 kcal" or "salad → fried chicken" would quietly skew all of them:
/// - a food or dish correction that moves kcal, protein, carbs or fat by more than 30% of the
///   current value (and more than a small floor), or whose kcal doesn't match its macros —
///   ordinary label corrections run right away;
/// - adding or removing a name redirect.
///
/// A call the tool would refuse anyway (impossible values, unknown food or alias) runs and fails
/// without asking.
pub async fn approval_preview(
    ctx: &PluginContext,
    name: &str,
    args: &Value,
) -> Result<Option<Value>, PluginError> {
    match name {
        "diet_update_food" => {
            let args: UpdateFoodArgs = parse_args(args.clone())?;
            let after = food_values(ctx, &args).await?;
            if validate_per_100g(&after).is_err() {
                return Ok(None);
            }
            let before = SourceChain::load(ctx)
                .await?
                .lookup(&args.food_name)
                .await?
                .map(|f| f.per_100g);
            Ok(nutrition_preview(
                args.food_name.trim(),
                None,
                "per_100g",
                before,
                after,
                &FOOD_FLOOR,
            ))
        }
        "diet_update_dish" => {
            let args: UpdateDishArgs = parse_args(args.clone())?;
            let after = dish_values(ctx, &args).await?;
            if sources::validate_dish_macros(&after, args.serving_g).is_err() {
                return Ok(None);
            }
            let before = SourceChain::load(ctx)
                .await?
                .lookup_dish(args.brand.as_deref().unwrap_or(""), &args.dish_name)
                .await?
                .map(|d| d.per_serving);
            Ok(nutrition_preview(
                args.dish_name.trim(),
                args.brand.as_deref().map(str::trim),
                "per_serving",
                before,
                after,
                &DISH_FLOOR,
            ))
        }
        "diet_add_alias" => {
            let args: AddAliasArgs = parse_args(args.clone())?;
            let alias_key = normalize(&args.alias);
            let chain = SourceChain::load(ctx).await?;
            let Some(food) = chain.lookup(&args.food_name).await? else {
                return Ok(None);
            };
            // Empty, the food's own name, or a name that already resolves: the tool returns
            // or refuses without changing anything.
            if alias_key.is_empty()
                || alias_key == food.name_key
                || chain.lookup(&alias_key).await?.is_some()
            {
                return Ok(None);
            }
            Ok(Some(json!({
                "kind": "add_food_alias",
                "alias": args.alias.trim(),
                "food": food.name,
            })))
        }
        "diet_remove_alias" => {
            let args: RemoveAliasArgs = parse_args(args.clone())?;
            let Some(redirect) = sources::lookup_redirect(ctx, &args.alias).await? else {
                return Ok(None);
            };
            Ok(Some(json!({
                "kind": "remove_food_alias",
                "alias": args.alias.trim(),
                "target": redirect.target,
            })))
        }
        _ => Ok(None),
    }
}

/// The preview for a food or dish correction, or `None` when it is neither large nor
/// inconsistent. Without a current entry there is nothing to compare, so only the kcal check
/// applies.
fn nutrition_preview(
    name: &str,
    brand: Option<&str>,
    basis: &str,
    before: Option<Macros>,
    after: Macros,
    floor: &ChangeFloor,
) -> Option<Value> {
    let mut reasons = Vec::new();
    if before.is_some_and(|b| is_large_change(&b, &after, floor)) {
        reasons.push("large_change");
    }
    if kcal_disagrees_with_macros(&after) {
        reasons.push("kcal_mismatch");
    }
    if reasons.is_empty() {
        return None;
    }
    Some(json!({
        "kind": "update_nutrition",
        "name": name,
        "brand": brand,
        "basis": basis,
        "before": before.map(|b| b.to_json()),
        "after": after.to_json(),
        "reasons": reasons,
    }))
}

fn is_large_change(before: &Macros, after: &Macros, floor: &ChangeFloor) -> bool {
    let moved =
        |b: f64, a: f64, floor: f64| (a - b).abs() > f64::max(LARGE_CHANGE_SHARE * b, floor);
    moved(before.kcal, after.kcal, floor.kcal)
        || moved(before.protein_g, after.protein_g, floor.grams)
        || moved(before.carbs_g, after.carbs_g, floor.grams)
        || moved(before.fat_g, after.fat_g, floor.grams)
}

pub async fn call(ctx: &PluginContext, name: &str, args: Value) -> Result<ToolOutput, PluginError> {
    match name {
        "diet_calculate_label" => calculate_label(args),
        "diet_log_meal" => log_meal(ctx, args).await,
        "diet_get_intake" => get_intake(ctx, args).await,
        "diet_update_food" => update_food(ctx, args).await,
        "diet_update_dish" => update_dish(ctx, args).await,
        "diet_add_alias" => add_alias(ctx, args).await,
        "diet_remove_alias" => remove_alias(ctx, args).await,
        other => Err(PluginError::NotFound(format!(
            "unknown diet tool {other:?}"
        ))),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, PluginError> {
    serde_json::from_value(args).map_err(|e| PluginError::InvalidArgs(e.to_string()))
}

#[derive(Deserialize)]
enum LabelBasis {
    #[serde(rename = "per_100g")]
    Per100g,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelCalculationArgs {
    basis: LabelBasis,
    portion_grams: f64,
    per_100g: LabelNutrition,
}

/// Deliberately has no PluginContext: calculating a supplied label cannot reach the database
/// or a model. Unknown units/fields are rejected by deserialization rather than ignored.
fn calculate_label(args: Value) -> Result<ToolOutput, PluginError> {
    let args: LabelCalculationArgs = parse_args(args)?;
    let LabelBasis::Per100g = args.basis;
    let portion = args
        .per_100g
        .for_grams(args.portion_grams)
        .map_err(PluginError::InvalidArgs)?;
    Ok(ToolOutput {
        content: json!({
            "source": "user_label",
            "basis": "per_100g",
            "per_100g": args.per_100g,
            "portion_grams": args.portion_grams,
            "portion": portion,
            "recorded": false,
            "note": "Calculated only from the supplied label. Missing values remain unknown; no meal or food-table data was saved."
        }),
        summary: ToolSummary::new("diet_label_calculated")
            .with("portion_grams", args.portion_grams),
    })
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
    // Per parsed dish, in the order `resolve_dishes` keeps. A branded dish counts as eaten out
    // even if the parser missed it, so a chain item split into ingredients (no menu record)
    // still never comes out of home stock.
    let eaten_out: Vec<bool> = parsed
        .dishes
        .iter()
        .map(|d| d.eaten_out || d.brand.is_some())
        .collect();
    let outcomes = dish_resolver::resolve_dishes(ctx, parsed.dishes).await?;

    // Stage 4 (§5.5): every unmatched dish's components go through the food resolver in one
    // call, flattened across the whole meal.
    let mut component_names = Vec::new();
    for outcome in &outcomes {
        if let DishOutcome::Unmatched(pd) = outcome {
            component_names.extend(pd.components.iter().map(|c| FoodQuery {
                name: c.name.clone(),
                alt_names: c.alt_names.clone(),
            }));
        }
    }
    let mut foods = resolve_many(ctx, &component_names).await?.into_iter();

    let chain = SourceChain::load(ctx).await?;
    let mut items = Vec::with_capacity(outcomes.len());
    for (outcome, eaten_out) in outcomes.into_iter().zip(eaten_out) {
        match outcome {
            DishOutcome::Matched(m) => {
                chain.record_dish_use(&m.dish).await?;
                let dish = m.dish;
                let macros = dish.per_serving.times(m.servings);
                items.push(MealItem {
                    macros,
                    name: dish.name,
                    display_name: None,
                    kind: ItemKind::Dish,
                    brand: (!dish.brand.is_empty()).then_some(dish.brand),
                    grams: m.grams,
                    servings: Some(m.servings),
                    food_id: None,
                    dish_id: Some(dish.id),
                    source: dish.source_kind,
                    source_id: Some(dish.source_id),
                    source_name: Some(dish.source_name),
                    eaten_out,
                });
            }
            DishOutcome::Unmatched(pd) => {
                for component in pd.components {
                    let food = foods.next().ok_or_else(|| {
                        PluginError::Other("resolver returned too few foods".into())
                    })?;
                    items.push(MealItem {
                        macros: food.per_100g.for_grams(component.grams),
                        display_name: MealItem::display_name_for(&component.name, &food.name),
                        name: food.name,
                        kind: ItemKind::Food,
                        brand: None,
                        grams: Some(component.grams),
                        servings: None,
                        food_id: Some(food.id),
                        dish_id: None,
                        source: food.source_kind,
                        source_id: Some(food.source_id),
                        source_name: Some(food.source_name),
                        eaten_out,
                    });
                }
            }
        }
    }
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
        "date": event.logical_date.to_string(),
        "meal_type": meal.meal_type,
        "items": meal.items.iter().map(MealItem::to_json).collect::<Vec<_>>(),
        "total": meal.total.to_json(),
        "day_total": day_total.to_json(),
        "day_target": target.map_or(Value::Null, |t| t.as_macros().to_json()),
        "remaining": target.map_or(Value::Null, |t| (t.as_macros() - day_total).to_json()),
    });
    Ok(ToolOutput {
        content,
        summary: ToolSummary::new("diet_meal_logged")
            .with("meal_type", meal.meal_type)
            .with("kcal", meal.total.kcal.round()),
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
        summary: ToolSummary::new("diet_intake_viewed").with("date", date),
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
    let per_100g = food_values(ctx, &args).await?;
    // User values always win (§16), so no kcal correction — but reject impossible ones.
    validate_per_100g(&per_100g).map_err(|e| PluginError::InvalidArgs(e.to_string()))?;

    let food = sources::upsert_manual_food(ctx, &args.food_name, per_100g).await?;
    Ok(ToolOutput {
        content: json!({
            "food_id": food.id,
            "name": food.name,
            "per_100g": food.per_100g.to_json(),
            "note": "Saved. Only meals logged from now on use these values; meals already \
                     logged keep their numbers.",
        }),
        summary: ToolSummary::new("diet_food_updated").with("name", &food.name),
    })
}

/// The per-100 g values `diet_update_food` saves. Without a new fiber value, the food keeps its
/// current chain-winner value (`docs/NUTRITION_SOURCES.md` §7.2).
async fn food_values(ctx: &PluginContext, args: &UpdateFoodArgs) -> Result<Macros, PluginError> {
    let fiber_g = match args.fiber_g {
        Some(f) => f,
        None => SourceChain::load(ctx)
            .await?
            .lookup(&args.food_name)
            .await?
            .map_or(0.0, |f| f.per_100g.fiber_g),
    };
    Ok(Macros {
        kcal: args.kcal,
        protein_g: args.protein_g,
        carbs_g: args.carbs_g,
        fat_g: args.fat_g,
        fiber_g,
    })
}

#[derive(Deserialize)]
struct UpdateDishArgs {
    dish_name: String,
    brand: Option<String>,
    kcal: f64,
    protein_g: f64,
    carbs_g: f64,
    fat_g: f64,
    fiber_g: Option<f64>,
    serving_g: Option<f64>,
}

async fn update_dish(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: UpdateDishArgs = parse_args(args)?;
    let per_serving = dish_values(ctx, &args).await?;
    sources::validate_dish_macros(&per_serving, args.serving_g)
        .map_err(|e| PluginError::InvalidArgs(e.to_string()))?;

    let dish = sources::upsert_manual_dish(
        ctx,
        &args.dish_name,
        args.brand.as_deref(),
        per_serving,
        args.serving_g,
    )
    .await?;
    Ok(ToolOutput {
        content: json!({
            "dish_id": dish.id,
            "name": dish.name,
            "brand": dish.brand,
            "per_serving": dish.per_serving.to_json(),
            "serving_g": dish.serving_g,
            "note": "Saved. Only meals logged from now on use these values; meals already \
                     logged keep their numbers.",
        }),
        summary: ToolSummary::new("diet_dish_updated").with("name", &dish.name),
    })
}

/// The per-serving values `diet_update_dish` saves. Without a new fiber value, the dish keeps
/// its current chain-winner value, as `food_values` does (`docs/NUTRITION_SOURCES.md` §7.2).
async fn dish_values(ctx: &PluginContext, args: &UpdateDishArgs) -> Result<Macros, PluginError> {
    let fiber_g = match args.fiber_g {
        Some(f) => f,
        None => SourceChain::load(ctx)
            .await?
            .lookup_dish(args.brand.as_deref().unwrap_or(""), &args.dish_name)
            .await?
            .map_or(0.0, |d| d.per_serving.fiber_g),
    };
    Ok(Macros {
        kcal: args.kcal,
        protein_g: args.protein_g,
        carbs_g: args.carbs_g,
        fat_g: args.fat_g,
        fiber_g,
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
        summary: ToolSummary::new("diet_alias_added")
            .with("alias", args.alias.trim())
            .with("food", &food.name),
    })
}

#[derive(Deserialize)]
struct RemoveAliasArgs {
    alias: String,
}

async fn remove_alias(ctx: &PluginContext, args: Value) -> Result<ToolOutput, PluginError> {
    let args: RemoveAliasArgs = parse_args(args)?;
    let alias = sources::remove_alias(ctx, &args.alias).await?;
    Ok(ToolOutput {
        content: json!({ "alias": alias }),
        summary: ToolSummary::new("diet_alias_removed").with("alias", args.alias.trim()),
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
            "dishes": [
                {"name": "熟米饭", "components": [{"name": "熟米饭", "grams": 200}]},
                {"name": "鸡蛋", "components": [{"name": "鸡蛋", "grams": 100}]}
            ]
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
        let today = env.ctx.today().await.unwrap();
        assert_eq!(out.content["date"], today.to_string());
        assert_eq!(out.content["items"][0]["estimated"], true);
        assert!(out.content["remaining"]["kcal"].is_number());
        assert_eq!(
            out.summary,
            ToolSummary::new("diet_meal_logged")
                .with("meal_type", "lunch")
                .with("kcal", 376.0)
        );

        let event = env.ctx.events.latest(MEAL_EVENT).await.unwrap().unwrap();
        let stored: MealLogged = serde_json::from_value(event.payload).unwrap();
        assert_eq!(stored.raw_text, "午饭一碗米饭两个鸡蛋");
        assert_eq!(stored.meal_type, MealType::Lunch);
        assert_eq!(stored.items.len(), 2);
    }

    #[tokio::test]
    async fn log_meal_matches_a_component_to_a_dataset_row_through_its_alt_names() {
        let env = env().await;
        seed_person(&env.ctx).await;
        crate::testutil::add_dataset(
            &env,
            "食物成分表",
            &[(
                "马铃薯",
                crate::model::Macros {
                    kcal: 77.0,
                    protein_g: 2.0,
                    carbs_g: 17.0,
                    fat_g: 0.2,
                    fiber_g: 2.2,
                },
            )],
        )
        .await;
        env.mock.push_response(json_response(json!({
            "meal_type": "lunch",
            "dishes": [{"name": "煮土豆", "components": [
                {"name": "土豆", "grams": 200, "alt_names": ["马铃薯"]}
            ]}]
        })));
        env.mock.push_response(json_response(
            json!({"matches": [{"food": 0, "candidate": 0}]}),
        ));

        let out = call(
            &env.ctx,
            "diet_log_meal",
            json!({"description": "午饭吃了煮土豆"}),
        )
        .await
        .unwrap();

        // 土豆 shares no character with 马铃薯; only the alternate name finds it.
        assert_eq!(out.content["items"][0]["name"], "马铃薯");
        // Shown as the user put it; the nutrition is still the matched row's.
        assert_eq!(out.content["items"][0]["display_name"], "土豆");
        assert_eq!(out.content["items"][0]["estimated"], false);
        assert_eq!(out.content["total"]["kcal"], 154.0);
        // The parser and the one match call: no estimate.
        assert_eq!(env.mock.received_requests().len(), 2);
    }

    #[tokio::test]
    async fn log_meal_uses_eaten_at_for_the_logical_date() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "熟米饭", RICE).await.unwrap();
        env.mock.push_response(json_response(json!({
            "meal_type": null,
            "dishes": [{"name": "熟米饭", "components": [{"name": "熟米饭", "grams": 150}]}]
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
    async fn log_meal_with_a_matched_dish_and_an_unmatched_home_dish() {
        let env = env().await;
        crate::sources::upsert_manual_dish(
            &env.ctx,
            "牛めし（並盛）",
            Some("松屋"),
            RICE,
            Some(350.0),
        )
        .await
        .unwrap();
        env.mock.push_response(json_response(json!({
            "meal_type": "lunch",
            "dishes": [
                {"name": "牛めし（並盛）", "brand": "松屋", "servings": 1,
                 "components": [{"name": "熟米饭", "grams": 250}, {"name": "牛肉(炖)", "grams": 85}]},
                {"name": "鸡蛋", "components": [{"name": "鸡蛋", "grams": 100}]}
            ]
        })));
        env.mock.push_closure(estimate_responder);

        let out = call(
            &env.ctx,
            "diet_log_meal",
            json!({"description": "松屋牛めし並盛和一个鸡蛋"}),
        )
        .await
        .unwrap();

        // Parser order preserved: the dish item first, then the home dish's food item.
        assert_eq!(out.content["items"][0]["kind"], "dish");
        assert_eq!(out.content["items"][0]["name"], "牛めし（並盛）");
        assert_eq!(out.content["items"][0]["brand"], "松屋");
        assert_eq!(out.content["items"][0]["servings"], 1.0);
        assert_eq!(out.content["items"][1]["kind"], "food");
        assert_eq!(out.content["items"][1]["name"], "鸡蛋");
        // Total is the dish's per-serving macros plus the egg's estimate.
        assert_eq!(out.content["total"]["kcal"], (RICE.kcal + EGG.kcal).round());

        let dish = SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup_dish("松屋", "牛めし（並盛）")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dish.use_count, 1);
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
    async fn only_large_or_inconsistent_nutrition_changes_need_approval() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "鸡蛋", EGG).await.unwrap();
        let preview = |name: &'static str, args: Value| {
            let ctx = &env.ctx;
            async move { approval_preview(ctx, name, &args).await.unwrap() }
        };
        let egg = |kcal: f64, protein_g: f64, carbs_g: f64, fat_g: f64| {
            json!({"food_name": "鸡蛋", "kcal": kcal, "protein_g": protein_g,
                   "carbs_g": carbs_g, "fat_g": fat_g})
        };

        // A label correction runs right away.
        assert_eq!(
            preview("diet_update_food", egg(150.0, 13.0, 1.0, 10.0)).await,
            None
        );
        // kcal 144 → 60 with matching macros: large.
        assert_eq!(
            preview("diet_update_food", egg(60.0, 6.0, 1.0, 4.0)).await,
            Some(json!({
                "kind": "update_nutrition", "name": "鸡蛋", "brand": null, "basis": "per_100g",
                "before": EGG.to_json(),
                "after": {"kcal": 60.0, "protein_g": 6.0, "carbs_g": 1.0, "fat_g": 4.0,
                          "fiber_g": 0.0},
                "reasons": ["large_change"],
            }))
        );
        // Same macros, kcal 144 → 110: under 30%, but no longer matches the macros.
        assert_eq!(
            preview("diet_update_food", egg(110.0, 13.3, 2.8, 8.8))
                .await
                .unwrap()["reasons"],
            json!(["kcal_mismatch"])
        );
        // A new food is only checked for consistency.
        let new_food = |kcal: f64| {
            json!({"food_name": "新食品", "kcal": kcal, "protein_g": 10, "carbs_g": 10,
                   "fat_g": 0})
        };
        assert_eq!(preview("diet_update_food", new_food(80.0)).await, None);
        assert_eq!(
            preview("diet_update_food", new_food(10.0)).await.unwrap()["reasons"],
            json!(["kcal_mismatch"])
        );
        // Impossible values are refused by the tool without asking.
        assert_eq!(
            preview("diet_update_food", egg(100.0, 60.0, 60.0, 0.0)).await,
            None
        );

        // Dishes use a larger kcal floor: 700 → 740 runs, 700 → 300 waits.
        let dish = |kcal: f64, carbs_g: f64| {
            json!({"dish_name": "牛めし（並盛）", "brand": "松屋", "kcal": kcal,
                   "protein_g": 25, "carbs_g": carbs_g, "fat_g": 28})
        };
        call(&env.ctx, "diet_update_dish", dish(700.0, 90.0))
            .await
            .unwrap();
        assert_eq!(preview("diet_update_dish", dish(740.0, 100.0)).await, None);
        let big = preview("diet_update_dish", dish(300.0, 0.0)).await.unwrap();
        assert_eq!(big["brand"], "松屋");
        assert_eq!(big["basis"], "per_serving");
        assert_eq!(big["before"]["kcal"], 700.0);
        assert_eq!(big["reasons"], json!(["large_change"]));
    }

    #[tokio::test]
    async fn adding_or_removing_a_name_redirect_needs_approval() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "卤鸡蛋", EGG).await.unwrap();
        upsert_manual_food(&env.ctx, "鸡蛋", EGG).await.unwrap();
        let preview = |name: &'static str, args: Value| {
            let ctx = &env.ctx;
            async move { approval_preview(ctx, name, &args).await.unwrap() }
        };

        assert_eq!(
            preview(
                "diet_add_alias",
                json!({"food_name": "卤鸡蛋", "alias": " 卤蛋 "})
            )
            .await,
            Some(json!({"kind": "add_food_alias", "alias": "卤蛋", "food": "卤鸡蛋"}))
        );
        // Unknown food, the food's own name, or a name that already resolves: no approval.
        for args in [
            json!({"food_name": "不存在", "alias": "卤蛋"}),
            json!({"food_name": "卤鸡蛋", "alias": "卤鸡蛋"}),
            json!({"food_name": "卤鸡蛋", "alias": "鸡蛋"}),
        ] {
            assert_eq!(preview("diet_add_alias", args).await, None);
        }

        call(
            &env.ctx,
            "diet_add_alias",
            json!({"food_name": "卤鸡蛋", "alias": "卤蛋"}),
        )
        .await
        .unwrap();
        assert_eq!(
            preview("diet_remove_alias", json!({"alias": "卤蛋"})).await,
            Some(json!({"kind": "remove_food_alias", "alias": "卤蛋", "target": "卤鸡蛋"}))
        );
        assert_eq!(
            preview("diet_remove_alias", json!({"alias": "没有"})).await,
            None
        );
        assert_eq!(preview("diet_get_intake", json!({})).await, None);
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
    async fn update_dish_creates_a_manual_dish_that_shadows_a_dataset_copy() {
        let env = env().await;
        crate::sources::upsert_manual_dish(&env.ctx, "牛めし（並盛）", Some("松屋"), EGG, None)
            .await
            .unwrap();

        let out = call(
            &env.ctx,
            "diet_update_dish",
            json!({
                "dish_name": "牛めし（並盛）", "brand": "松屋",
                "kcal": 733, "protein_g": 25, "carbs_g": 90, "fat_g": 28, "serving_g": 350
            }),
        )
        .await
        .unwrap();

        assert_eq!(out.content["per_serving"]["kcal"], 733.0);
        assert_eq!(
            out.summary,
            ToolSummary::new("diet_dish_updated").with("name", "牛めし（並盛）")
        );
        let dish = SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup_dish("松屋", "牛めし（並盛）")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dish.per_serving.kcal, 733.0);
        assert_eq!(dish.serving_g, Some(350.0));
    }

    #[tokio::test]
    async fn update_dish_rejects_impossible_values() {
        let env = env().await;
        assert!(matches!(
            call(
                &env.ctx,
                "diet_update_dish",
                json!({
                    "dish_name": "怪菜", "kcal": 100, "protein_g": 10, "carbs_g": 10, "fat_g": 10,
                    "serving_g": 1
                }),
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

        assert_eq!(
            out.summary,
            ToolSummary::new("diet_alias_added")
                .with("alias", "卤蛋")
                .with("food", "卤鸡蛋")
        );
        assert!(SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup("卤蛋")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn remove_alias_undoes_a_wrong_match_and_stops_it_coming_back() {
        let env = env().await;
        crate::testutil::add_dataset(&env, "表", &[("稻米饭(蒸)", RICE)]).await;
        env.mock.push_response(json_response(
            json!({"matches": [{"food": 0, "candidate": 0}]}),
        ));
        crate::resolver::resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(env.mock.received_requests().len(), 1);
        assert_eq!(
            sources::lookup_redirect(&env.ctx, "熟米饭")
                .await
                .unwrap()
                .map(|r| (r.target, r.origin)),
            Some(("稻米饭(蒸)".into(), "match".into()))
        );

        let out = call(&env.ctx, "diet_remove_alias", json!({"alias": " 熟米饭。"}))
            .await
            .unwrap();
        assert_eq!(out.content["alias"], "熟米饭");
        assert_eq!(
            out.summary,
            ToolSummary::new("diet_alias_removed").with("alias", "熟米饭。")
        );
        assert!(sources::lookup_redirect(&env.ctx, "熟米饭")
            .await
            .unwrap()
            .is_none());

        // The name is estimated now, and the same row is not matched again: one estimate
        // request, no second match request.
        env.mock.push_closure(estimate_responder);
        let food = crate::resolver::resolve(&env.ctx, "熟米饭").await.unwrap();
        assert_eq!(food.source_kind, crate::model::SourceKind::LlmEstimate);
        assert_eq!(env.mock.received_requests().len(), 2);
    }

    #[tokio::test]
    async fn remove_alias_removes_a_user_alias_too_and_rejects_unknown_names() {
        let env = env().await;
        upsert_manual_food(&env.ctx, "卤鸡蛋", EGG).await.unwrap();
        call(
            &env.ctx,
            "diet_add_alias",
            json!({"food_name": "卤鸡蛋", "alias": "卤蛋"}),
        )
        .await
        .unwrap();

        call(&env.ctx, "diet_remove_alias", json!({"alias": "卤蛋"}))
            .await
            .unwrap();

        assert!(SourceChain::load(&env.ctx)
            .await
            .unwrap()
            .lookup("卤蛋")
            .await
            .unwrap()
            .is_none());
        assert!(matches!(
            call(&env.ctx, "diet_remove_alias", json!({"alias": "卤蛋"})).await,
            Err(PluginError::NotFound(_))
        ));
        assert!(matches!(
            call(&env.ctx, "diet_remove_alias", json!({"alias": "  "})).await,
            Err(PluginError::InvalidArgs(_))
        ));
    }
}
