//! Diet prompts, verbatim from `docs/PHASE1_SPEC.md` §9.2/§9.3. `{placeholders}` are filled
//! by plain string replacement.

/// §9.2 — role Parser, JSON. Placeholders: `{known_foods}`, `{eaten_at_local}`,
/// `{description}`.
pub const MEAL_PARSER: &str = r#"Extract the foods from the user's meal description and estimate the edible weight in grams
of each item as eaten.

Return JSON: {"meal_type": "breakfast|lunch|dinner|snack|null", "items": [{"name": string, "grams": number}]}

Rules:
- Use a name from KNOWN_FOODS when it refers to the same food. Otherwise use a short,
  specific, generic food name in the user's language (e.g. "熟米饭", "卤蛋", "牛肉(熟)").
- Split composite dishes into main components with separate grams, unless the whole dish
  appears in KNOWN_FOODS.
- Include cooking oil for fried or stir-fried dishes as a separate item ("食用油") with a
  typical amount.
- Use typical portions when not stated. Counts ("两个鸡蛋") become grams.
- Drinks with calories count as items. Plain water, black coffee and tea without sugar are
  omitted.
- meal_type only if the text states or clearly implies it; otherwise null.

KNOWN_FOODS: {known_foods}
Meal time: {eaten_at_local}
Description: {description}"#;

/// §9.3 — role Parser, JSON. Placeholder: `{name}`.
pub const FOOD_ESTIMATE: &str = r#"Give typical nutrition values per 100 g of the edible portion of this food, as eaten
(cooked if it is normally eaten cooked). Use values consistent with standard food
composition tables.

`carbs_g` is total carbohydrate, fiber included — the same convention standard food composition
tables use. Do not subtract fiber from it.

Return JSON: {"kcal": number, "protein_g": number, "carbs_g": number, "fat_g": number, "fiber_g": number}

Food: {name}"#;
