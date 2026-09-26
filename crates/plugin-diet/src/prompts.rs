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
  appears in KNOWN_FOODS. A dish name is not an item by itself: "牛肉拉面" becomes
  "拉面(熟)", "牛肉(熟)", "牛骨汤" and "青菜"; "番茄炒蛋" becomes "番茄", "鸡蛋" and "食用油".
- Include cooking oil for fried or stir-fried dishes as a separate item ("食用油") with a
  typical amount.
- Use grams exactly as stated in the description. Use typical portions when not stated.
  Counts ("两个鸡蛋") become grams.
- When a food's nutrition differs a lot between dry/raw and cooked, the name must say which
  state the grams are for: "燕麦片(干)" 40 g but "燕麦粥" 250 g; "大米(生)" but "熟米饭".
- Drinks with calories count as items. Plain water, black coffee and tea without sugar are
  omitted.
- meal_type only if the text states or clearly implies it; otherwise null.

KNOWN_FOODS: {known_foods}
Meal time: {eaten_at_local}
Description: {description}"#;

/// §9.3 — role Parser, JSON. Placeholder: `{name}`.
pub const FOOD_ESTIMATE: &str = r#"Give typical nutrition values per 100 g of the edible portion of this food, as eaten
(cooked if it is normally eaten cooked). If the name states a state such as 干, 生 or 熟, give
values for that state. Use values consistent with standard food composition tables.

`carbs_g` is total carbohydrate, fiber included — the same convention standard food composition
tables use. Do not subtract fiber from it.

Return JSON: {"kcal": number, "protein_g": number, "carbs_g": number, "fat_g": number, "fiber_g": number}

Food: {name}"#;
