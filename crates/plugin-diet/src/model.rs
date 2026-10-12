//! Diet data model, per `docs/PHASE1_SPEC.md` §8.1.

use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub};

use chrono::{NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Energy and macronutrients — per 100 g (Food), per serving (Dish), or for a portion, meal or
/// day. `carbs_g` is total carbohydrate, fiber included (§8.3).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Macros {
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    #[serde(default)]
    pub fiber_g: f64,
}

/// Values explicitly supplied on a per-100 g label. Missing values remain unknown, rather
/// than becoming zero or being estimated from the other nutrients.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelNutrition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kcal: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein_g: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs_g: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_g: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fiber_g: Option<f64>,
}

impl LabelNutrition {
    /// Calculate a portion using the same scaling as logged nutrition, without persisting
    /// anything or filling in missing label values. All quantities are grams, energy is kcal.
    pub fn for_grams(&self, grams: f64) -> Result<Self, String> {
        if !grams.is_finite() || grams <= 0.0 {
            return Err("portion_grams must be a finite number greater than zero".into());
        }
        let values = [
            self.kcal,
            self.protein_g,
            self.carbs_g,
            self.fat_g,
            self.fiber_g,
        ];
        if values.iter().all(Option::is_none) {
            return Err("provide at least one nutrition value from the per-100 g label".into());
        }
        if values.iter().flatten().any(|v| !v.is_finite() || *v < 0.0) {
            return Err("label values must be finite numbers greater than or equal to zero".into());
        }
        if self.kcal.is_some_and(|v| v > 900.0)
            || self.fiber_g.is_some_and(|v| v > 100.0)
            || self.protein_g.unwrap_or(0.0)
                + self.carbs_g.unwrap_or(0.0)
                + self.fat_g.unwrap_or(0.0)
                > 100.0 + 1e-9
        {
            return Err("check the label units: per 100 g, energy cannot exceed 900 kcal and nutrients cannot exceed 100 g".into());
        }
        if let (Some(fiber), Some(carbs)) = (self.fiber_g, self.carbs_g) {
            if fiber > carbs {
                return Err("fiber_g cannot exceed carbs_g (total carbohydrate)".into());
            }
        }
        let scaled = Macros {
            kcal: self.kcal.unwrap_or(0.0),
            protein_g: self.protein_g.unwrap_or(0.0),
            carbs_g: self.carbs_g.unwrap_or(0.0),
            fat_g: self.fat_g.unwrap_or(0.0),
            fiber_g: self.fiber_g.unwrap_or(0.0),
        }
        .for_grams(grams);
        if [
            scaled.kcal,
            scaled.protein_g,
            scaled.carbs_g,
            scaled.fat_g,
            scaled.fiber_g,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err("portion calculation is too large; check the quantity and units".into());
        }
        Ok(Self {
            kcal: self.kcal.map(|_| scaled.kcal),
            protein_g: self.protein_g.map(|_| scaled.protein_g),
            carbs_g: self.carbs_g.map(|_| scaled.carbs_g),
            fat_g: self.fat_g.map(|_| scaled.fat_g),
            fiber_g: self.fiber_g.map(|_| scaled.fiber_g),
        })
    }
}

impl Macros {
    /// Scales per-100 g values to a portion of `grams`.
    pub fn for_grams(&self, grams: f64) -> Macros {
        let f = grams / 100.0;
        Macros {
            kcal: self.kcal * f,
            protein_g: self.protein_g * f,
            carbs_g: self.carbs_g * f,
            fat_g: self.fat_g * f,
            fiber_g: self.fiber_g * f,
        }
    }

    /// Scales per-serving values (a `Dish`) by a number of servings.
    pub fn times(&self, servings: f64) -> Macros {
        Macros {
            kcal: self.kcal * servings,
            protein_g: self.protein_g * servings,
            carbs_g: self.carbs_g * servings,
            fat_g: self.fat_g * servings,
            fiber_g: self.fiber_g * servings,
        }
    }

    /// Rounded for display: kcal to 1, grams to 0.1 (§8.1). Stored values keep full precision.
    pub fn rounded(&self) -> Macros {
        Macros {
            kcal: self.kcal.round(),
            protein_g: round1(self.protein_g),
            carbs_g: round1(self.carbs_g),
            fat_g: round1(self.fat_g),
            fiber_g: round1(self.fiber_g),
        }
    }

    /// The rounded values as JSON, the shape every tool result and card uses.
    pub fn to_json(&self) -> Value {
        let r = self.rounded();
        json!({
            "kcal": r.kcal,
            "protein_g": r.protein_g,
            "carbs_g": r.carbs_g,
            "fat_g": r.fat_g,
            "fiber_g": r.fiber_g,
        })
    }
}

/// Rounds to one decimal place.
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

impl Add for Macros {
    type Output = Macros;
    fn add(self, o: Macros) -> Macros {
        Macros {
            kcal: self.kcal + o.kcal,
            protein_g: self.protein_g + o.protein_g,
            carbs_g: self.carbs_g + o.carbs_g,
            fat_g: self.fat_g + o.fat_g,
            fiber_g: self.fiber_g + o.fiber_g,
        }
    }
}

impl AddAssign for Macros {
    fn add_assign(&mut self, o: Macros) {
        *self = *self + o;
    }
}

impl Sub for Macros {
    type Output = Macros;
    fn sub(self, o: Macros) -> Macros {
        Macros {
            kcal: self.kcal - o.kcal,
            protein_g: self.protein_g - o.protein_g,
            carbs_g: self.carbs_g - o.carbs_g,
            fat_g: self.fat_g - o.fat_g,
            fiber_g: self.fiber_g - o.fiber_g,
        }
    }
}

impl Sum for Macros {
    fn sum<I: Iterator<Item = Macros>>(iter: I) -> Macros {
        iter.fold(Macros::default(), Add::add)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MealType {
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

impl MealType {
    pub fn parse(s: &str) -> Option<MealType> {
        match s.trim().to_ascii_lowercase().as_str() {
            "breakfast" => Some(MealType::Breakfast),
            "lunch" => Some(MealType::Lunch),
            "dinner" => Some(MealType::Dinner),
            "snack" => Some(MealType::Snack),
            _ => None,
        }
    }

    /// The serde name, for English text.
    pub fn as_str(&self) -> &'static str {
        match self {
            MealType::Breakfast => "breakfast",
            MealType::Lunch => "lunch",
            MealType::Dinner => "dinner",
            MealType::Snack => "snack",
        }
    }

    pub fn zh_label(&self) -> &'static str {
        match self {
            MealType::Breakfast => "早餐",
            MealType::Lunch => "午餐",
            MealType::Dinner => "晚餐",
            MealType::Snack => "加餐",
        }
    }

    /// The meal type for a local time when neither the user nor the parser gives one (§8.4):
    /// 05:00–10:29 breakfast, 10:30–14:29 lunch, 14:30–16:59 snack, 17:00–21:29 dinner,
    /// otherwise snack.
    pub fn default_for(time: NaiveTime) -> MealType {
        match time.hour() * 60 + time.minute() {
            300..=629 => MealType::Breakfast,
            630..=869 => MealType::Lunch,
            870..=1019 => MealType::Snack,
            1020..=1289 => MealType::Dinner,
            _ => MealType::Snack,
        }
    }
}

/// Provenance kind of a food's nutrition data. Serialized names keep Phase 1's values so old
/// event payloads still parse: `Manual` is `"user"`, as `NutritionSource::User` was.
///
/// This is the JSON-facing representation; `diet_food_sources.kind` in the database uses its
/// own, unrelated set of strings (`"manual"` / `"dataset"` / `"llm_estimate"`, see
/// `SourceKind::db_str`), per `docs/NUTRITION_SOURCES.md` §3.1/§4.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    #[serde(rename = "user")]
    Manual,
    #[serde(rename = "dataset")]
    Dataset,
    #[serde(rename = "llm_estimate")]
    LlmEstimate,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Manual => "user",
            SourceKind::Dataset => "dataset",
            SourceKind::LlmEstimate => "llm_estimate",
        }
    }

    pub fn parse(s: &str) -> Option<SourceKind> {
        match s {
            "user" => Some(SourceKind::Manual),
            "dataset" => Some(SourceKind::Dataset),
            "llm_estimate" => Some(SourceKind::LlmEstimate),
            _ => None,
        }
    }

    /// The string stored in `diet_food_sources.kind` — distinct from the serde/JSON name above.
    pub fn db_str(&self) -> &'static str {
        match self {
            SourceKind::Manual => "manual",
            SourceKind::Dataset => "dataset",
            SourceKind::LlmEstimate => "llm_estimate",
        }
    }

    pub fn from_db_str(s: &str) -> Option<SourceKind> {
        match s {
            "manual" => Some(SourceKind::Manual),
            "dataset" => Some(SourceKind::Dataset),
            "llm_estimate" => Some(SourceKind::LlmEstimate),
            _ => None,
        }
    }
}

/// Whether a logged meal item came from a per-100 g `Food` or a per-serving `Dish`
/// (`docs/DISH_NUTRITION.md` §2, §6). `#[serde(default)]` on `MealItem::kind` means an event
/// logged before dishes existed deserializes as `Food`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    #[default]
    Food,
    Dish,
}

/// A per-serving nutrition row (`docs/DISH_NUTRITION.md` §2, §4.1): a chain restaurant menu item
/// or similar, identified by `(brand_key, name_key)` rather than by name alone. `brand == ""` is
/// a brandless dish. `source_kind` is never `LlmEstimate`: dishes are never AI-estimated (D3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dish {
    pub id: String,
    pub brand: String,
    pub brand_key: String,
    pub name: String,
    pub name_key: String,
    pub serving_g: Option<f64>,
    pub per_serving: Macros,
    pub source_id: String,
    pub source_kind: SourceKind,
    pub source_name: String,
    pub use_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MealItem {
    /// Canonical food name, or dish name including size, from the food/dish table.
    pub name: String,
    /// `#[serde(default)]`: absent in every event logged before dishes existed, which are all
    /// `Food`.
    #[serde(default)]
    pub kind: ItemKind,
    /// Dish items only.
    #[serde(default)]
    pub brand: Option<String>,
    /// Food: always. Dish: `serving_g * servings` when `serving_g` is known, else `None`.
    pub grams: Option<f64>,
    /// Dish items only.
    #[serde(default)]
    pub servings: Option<f64>,
    /// Food items. `None` for a dish item; always present on food items, including events
    /// logged before this field existed (so no `#[serde(default)]`).
    pub food_id: Option<String>,
    /// Dish items only.
    #[serde(default)]
    pub dish_id: Option<String>,
    pub source: SourceKind,
    /// Snapshot of the winning source's display name at logging time, so a later rename or
    /// delete of the source doesn't change history (`docs/NUTRITION_SOURCES.md` §5). `None` for
    /// events logged before this field existed.
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_name: Option<String>,
    /// Comes from a dish bought ready to eat (restaurant, takeout, convenience store) or one
    /// with a brand, including each component of such a dish when it had no per-serving
    /// record. Other plugins (stock) read it from the `diet.meal_logged` payload to tell meals
    /// eaten out from home cooking. `false` on events logged before this field existed.
    #[serde(default)]
    pub eaten_out: bool,
    /// For this portion.
    pub macros: Macros,
}

impl MealItem {
    /// Display JSON: `{name, kind, brand, grams, servings, kcal, protein_g, estimated,
    /// source_kind, source_name}`. `grams` is `null` for a dish item whose `serving_g` is unknown.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "kind": self.kind,
            "brand": self.brand,
            "grams": self.grams.map(round1),
            "servings": self.servings,
            "kcal": self.macros.kcal.round(),
            "protein_g": round1(self.macros.protein_g),
            "estimated": self.source == SourceKind::LlmEstimate,
            "source_kind": self.source,
            "source_name": self.source_name,
        })
    }

    /// One-line text for `intake::context_snippet`, the web UI and the CLI
    /// (`docs/DISH_NUTRITION.md` §6): a food reads "熟米饭 150g"; a dish reads
    /// "松屋 牛めし（並盛）×1" (no brand: "牛めし（並盛）×1").
    pub fn label(&self) -> String {
        match self.kind {
            ItemKind::Food => {
                let grams = self.grams.map(round1).unwrap_or(0.0);
                format!("{} {grams}g", self.name)
            }
            ItemKind::Dish => {
                let servings = self.servings.unwrap_or(1.0);
                match self.brand.as_deref() {
                    Some(brand) if !brand.is_empty() => {
                        format!("{brand} {}×{servings}", self.name)
                    }
                    _ => format!("{}×{servings}", self.name),
                }
            }
        }
    }
}

/// Payload of a `diet.meal_logged` event. Totals are computed and stored at logging time, so
/// later food corrections never rewrite history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MealLogged {
    pub meal_type: MealType,
    pub raw_text: String,
    pub items: Vec<MealItem>,
    pub total: Macros,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn for_grams_scales_per_100g_values() {
        let per_100g = Macros {
            kcal: 116.0,
            protein_g: 2.6,
            carbs_g: 25.9,
            fat_g: 0.3,
            fiber_g: 0.3,
        };
        let portion = per_100g.for_grams(200.0);
        assert!((portion.kcal - 232.0).abs() < 1e-9);
        assert!((portion.protein_g - 5.2).abs() < 1e-9);
    }

    #[test]
    fn rounded_rounds_kcal_to_1_and_grams_to_0_1() {
        let m = Macros {
            kcal: 231.6,
            protein_g: 5.26,
            carbs_g: 51.84,
            fat_g: 0.04,
            fiber_g: 0.55,
        };
        assert_eq!(
            m.rounded(),
            Macros {
                kcal: 232.0,
                protein_g: 5.3,
                carbs_g: 51.8,
                fat_g: 0.0,
                fiber_g: 0.6,
            }
        );
    }

    #[test]
    fn macros_add_sub_and_sum() {
        let a = Macros {
            kcal: 100.0,
            protein_g: 10.0,
            carbs_g: 5.0,
            fat_g: 2.0,
            fiber_g: 1.0,
        };
        let b = Macros {
            kcal: 50.0,
            protein_g: 1.0,
            carbs_g: 1.0,
            fat_g: 1.0,
            fiber_g: 0.0,
        };
        assert_eq!((a + b).kcal, 150.0);
        assert_eq!((a - b).protein_g, 9.0);
        assert_eq!([a, b, b].into_iter().sum::<Macros>().kcal, 200.0);
    }

    #[test]
    fn default_meal_type_by_time_boundaries() {
        assert_eq!(MealType::default_for(t(4, 59)), MealType::Snack);
        assert_eq!(MealType::default_for(t(5, 0)), MealType::Breakfast);
        assert_eq!(MealType::default_for(t(10, 29)), MealType::Breakfast);
        assert_eq!(MealType::default_for(t(10, 30)), MealType::Lunch);
        assert_eq!(MealType::default_for(t(14, 29)), MealType::Lunch);
        assert_eq!(MealType::default_for(t(14, 30)), MealType::Snack);
        assert_eq!(MealType::default_for(t(16, 59)), MealType::Snack);
        assert_eq!(MealType::default_for(t(17, 0)), MealType::Dinner);
        assert_eq!(MealType::default_for(t(21, 29)), MealType::Dinner);
        assert_eq!(MealType::default_for(t(21, 30)), MealType::Snack);
    }

    #[test]
    fn meal_type_parse_accepts_known_names_only() {
        assert_eq!(MealType::parse("Lunch"), Some(MealType::Lunch));
        assert_eq!(MealType::parse("null"), None);
        assert_eq!(MealType::parse("brunch"), None);
    }

    #[test]
    fn times_scales_per_serving_values() {
        let per_serving = Macros {
            kcal: 733.0,
            protein_g: 25.0,
            carbs_g: 90.0,
            fat_g: 28.0,
            fiber_g: 3.0,
        };
        let two_servings = per_serving.times(2.0);
        assert!((two_servings.kcal - 1466.0).abs() < 1e-9);
        assert!((two_servings.protein_g - 50.0).abs() < 1e-9);
    }

    /// `docs/DISH_NUTRITION.md` §6/§10: a Phase 1 `diet.meal_logged` payload — `grams`/`food_id`
    /// always bare values, no `kind`/`brand`/`servings`/`dish_id`/`source_id`/`source_name` keys
    /// at all — must still deserialize, defaulting to a food item.
    #[test]
    fn old_meal_item_payload_without_dish_fields_deserializes_as_food() {
        let raw = r#"{
            "name": "熟米饭",
            "grams": 150.0,
            "food_id": "id-rice",
            "source": "llm_estimate",
            "macros": {"kcal": 174.0, "protein_g": 3.9, "carbs_g": 38.85, "fat_g": 0.45, "fiber_g": 0.45}
        }"#;
        let item: MealItem = serde_json::from_str(raw).unwrap();
        assert_eq!(item.kind, ItemKind::Food);
        assert_eq!(item.brand, None);
        assert_eq!(item.grams, Some(150.0));
        assert_eq!(item.servings, None);
        assert_eq!(item.food_id, Some("id-rice".to_string()));
        assert_eq!(item.dish_id, None);
        assert_eq!(item.source_id, None);
        assert_eq!(item.source_name, None);
        assert_eq!(item.to_json()["estimated"], true);
    }

    fn dish_macros() -> Macros {
        Macros {
            kcal: 733.0,
            protein_g: 25.0,
            carbs_g: 90.0,
            fat_g: 28.0,
            fiber_g: 3.0,
        }
    }

    #[test]
    fn label_formats_a_food_item_with_grams() {
        let item = MealItem {
            name: "熟米饭".into(),
            kind: ItemKind::Food,
            brand: None,
            grams: Some(150.0),
            servings: None,
            food_id: Some("id".into()),
            dish_id: None,
            source: SourceKind::Manual,
            source_id: None,
            source_name: None,
            eaten_out: false,
            macros: Macros::default(),
        };
        assert_eq!(item.label(), "熟米饭 150g");
    }

    #[test]
    fn label_formats_a_dish_item_with_brand_and_servings() {
        let item = MealItem {
            name: "牛めし（並盛）".into(),
            kind: ItemKind::Dish,
            brand: Some("松屋".into()),
            grams: Some(350.0),
            servings: Some(1.0),
            food_id: None,
            dish_id: Some("id".into()),
            source: SourceKind::Dataset,
            source_id: None,
            source_name: None,
            eaten_out: false,
            macros: dish_macros(),
        };
        assert_eq!(item.label(), "松屋 牛めし（並盛）×1");
    }

    #[test]
    fn label_formats_a_brandless_dish_item() {
        let item = MealItem {
            name: "A定食（大）".into(),
            kind: ItemKind::Dish,
            brand: None,
            grams: None,
            servings: Some(1.0),
            food_id: None,
            dish_id: Some("id".into()),
            source: SourceKind::Dataset,
            source_id: None,
            source_name: None,
            eaten_out: false,
            macros: dish_macros(),
        };
        assert_eq!(item.label(), "A定食（大）×1");
        assert_eq!(item.to_json()["grams"], Value::Null);
        assert_eq!(item.to_json()["kind"], "dish");
    }

    /// A migration-2-era payload, which already has `source_id`/`source_name` but predates
    /// dishes, also deserializes as a food item.
    #[test]
    fn migration_2_era_meal_item_payload_deserializes_as_food() {
        let raw = r#"{
            "name": "熟米饭",
            "grams": 150.0,
            "food_id": "id-rice",
            "source": "user",
            "source_id": "manual",
            "source_name": "手动录入",
            "macros": {"kcal": 174.0, "protein_g": 3.9, "carbs_g": 38.85, "fat_g": 0.45, "fiber_g": 0.45}
        }"#;
        let item: MealItem = serde_json::from_str(raw).unwrap();
        assert_eq!(item.kind, ItemKind::Food);
        assert_eq!(item.source_id, Some("manual".to_string()));
        assert_eq!(item.source_name, Some("手动录入".to_string()));
    }
}
