//! Diet prompts, verbatim from `docs/PHASE1_SPEC.md` §9.2/§9.3, extended by
//! `docs/DISH_NUTRITION.md` §5.1/§5.3 with dishes and the pick step. `{placeholders}` are filled
//! by plain string replacement.

/// §5.1 (replacing §9.2's flat-item shape) — role Parser, JSON. Placeholders: `{known_foods}`,
/// `{known_brands}`, `{known_dishes}`, `{eaten_at_local}`, `{description}`.
pub const MEAL_PARSER: &str = r#"Extract the dishes from the user's meal description. Each dish is one thing the user ate as a
unit: a menu item, a home-cooked dish, or a single food. Every dish must carry its ingredient
breakdown as components with estimated edible weight in grams as eaten, even when it also has a
brand/size — the breakdown is the fallback when the brand/size doesn't match a known record.

Return JSON: {"meal_type": "breakfast|lunch|dinner|snack|null", "dishes": [
  {"name": string, "brand": string|null, "size": string|null, "servings": number,
   "grams": number|null, "components": [{"name": string, "grams": number, "alt_names": [string]}]}
]}

Rules:
- One entry per thing the user ate as a unit. A single food is a dish whose only component is
  itself.
- `brand`: only if the user names the shop or the item is unmistakably that chain's product. Use
  the exact spelling from KNOWN_BRANDS when it's the same brand. Otherwise null.
- `size`: the size or variant as the user said it (並盛, 大盛, M, 2ピース), else null.
- A brandless item that is one of KNOWN_DISHES: copy that name exactly (it already includes any
  size, e.g. "A定食（大）") and set size to null.
- `servings`: the count of that item (default 1). `grams`: only if the user states the weight of
  the whole item.
- `components`: what the item consists of, with grams for everything eaten (servings already
  applied):
  - Use a name from KNOWN_FOODS when a component refers to the same food. Otherwise use a short,
    specific, generic food name in the user's language (e.g. "熟米饭", "卤蛋", "牛肉(熟)").
  - `alt_names`: up to 2 other common or standard Chinese names for the same food in the same
    state (e.g. 土豆 → 马铃薯, 西红柿 → 番茄, 熟米饭 → 米饭(蒸)). Omit it when there are none.
    Never use it for a different food or a different state.
  - Split composite dishes into main components with separate grams: "牛肉拉面" becomes
    "拉面(熟)", "牛肉(熟)", "牛骨汤" and "青菜"; "番茄炒蛋" becomes "番茄", "鸡蛋" and "食用油".
  - Include cooking oil for fried or stir-fried dishes as a separate component ("食用油") with a
    typical amount.
  - Use grams exactly as stated in the description. Use typical portions when not stated.
    Counts ("两个鸡蛋") become grams.
  - When a food's nutrition differs a lot between dry/raw and cooked, the name must say which
    state the grams are for: "燕麦片(干)" 40 g but "燕麦粥" 250 g; "大米(生)" but "熟米饭".
  - Drinks with calories count as components. Plain water, black coffee and tea without sugar
    are omitted.
- meal_type only if the text states or clearly implies it; otherwise null.

KNOWN_FOODS: {known_foods}
KNOWN_BRANDS: {known_brands}
KNOWN_DISHES: {known_dishes}
Meal time: {eaten_at_local}
Description: {description}"#;

/// §5.3 step 2 — role Parser, JSON. Placeholder: `{pending_dishes}`. Runs once per meal, listing
/// every branded dish that missed the exact stage-2 match against its local candidate rows.
pub const DISH_PICK: &str = r#"For each pending dish, choose which of its candidates is the same menu item the user meant, or
null if none of them match. Each candidate is {"index": number, "name": string}.

Rules:
- Choose only a candidate that is the same item as the user described, just a different size or
  spelling.
- When the user stated no size, choose the regular size (並/M/レギュラー/标准, or however the
  chain names its default).
- Size words may be in another language or generic (小/中/大碗, 小/中/大份, small/medium/large,
  普通): map them to the chain's own naming. 中/medium/普通 means the regular size; 小/大 mean the
  next size down/up that exists among the candidates. A stated size with no reasonable
  equivalent among the candidates is null.
- An item that isn't among the candidates at all is null. Never invent a candidate index.

Return JSON: {"picks": [{"dish": number, "candidate": number|null}, ...]}, one entry per pending
dish: "dish" is the pending dish's own "dish" value, and "candidate" is the "index" value of the
chosen candidate, copied exactly (or null).

Pending dishes: {pending_dishes}"#;

/// `docs/FOOD_MATCHING.md` §5 — role Parser, JSON. Placeholder: `{pending_foods}`. Runs once per
/// `resolve_many`, listing every food name that missed the exact lookup and has candidates.
pub const FOOD_MATCH: &str = r#"For each pending food, choose which of its candidates is the same food in the same state, or
null if none of them is. Each candidate is {"index": number, "name": string}.

Rules:
- Choose a candidate only if it is the same food in the same state. Raw/dry vs cooked is a
  different food: "大米(生)" is not "熟米饭", and raw "牛肉(瘦)" is not "牛肉(熟)", because the
  per-100 g values differ a lot. A candidate with no stated state counts as the table's usual
  form, which is raw for meat, grains and vegetables in a food composition table.
- A more specific variety of the same food is fine: "稻米饭(蒸)" for "熟米饭". A different food
  that merely shares characters is not: "米粉" for "米饭".
- When unsure, answer null. An estimate is better than a wrong row.
- Copy the candidate's "index" value exactly. Never invent an index.

Return JSON: {"matches": [{"food": number, "candidate": number|null}, ...]}, one entry per
pending food: "food" is the pending food's own "food" value, and "candidate" is the "index"
value of the chosen candidate, copied exactly (or null).

Pending foods: {pending_foods}"#;

/// §9.3 — role Parser, JSON. Placeholder: `{name}`.
pub const FOOD_ESTIMATE: &str = r#"Give typical nutrition values per 100 g of the edible portion of this food, as eaten
(cooked if it is normally eaten cooked). If the name states a state such as 干, 生 or 熟, give
values for that state. Use values consistent with standard food composition tables.

`carbs_g` is total carbohydrate, fiber included — the same convention standard food composition
tables use. Do not subtract fiber from it.

Return JSON: {"kcal": number, "protein_g": number, "carbs_g": number, "fat_g": number, "fiber_g": number}

Food: {name}"#;
