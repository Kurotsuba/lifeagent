//! Diet data model, per `docs/PHASE1_SPEC.md` §8.1.

use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub};

use chrono::{NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Energy and macronutrients — per 100 g for a food, or for a portion, meal or day.
/// `carbs_g` is total carbohydrate, fiber included (§8.3).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Macros {
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    #[serde(default)]
    pub fiber_g: f64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NutritionSource {
    User,
    LlmEstimate,
}

impl NutritionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            NutritionSource::User => "user",
            NutritionSource::LlmEstimate => "llm_estimate",
        }
    }

    pub fn parse(s: &str) -> Option<NutritionSource> {
        match s {
            "user" => Some(NutritionSource::User),
            "llm_estimate" => Some(NutritionSource::LlmEstimate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MealItem {
    /// Canonical food name from the food table.
    pub name: String,
    pub grams: f64,
    pub food_id: String,
    pub source: NutritionSource,
    /// For this portion.
    pub macros: Macros,
}

impl MealItem {
    /// Display JSON: `{name, grams, kcal, protein_g, estimated}`.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "grams": round1(self.grams),
            "kcal": self.macros.kcal.round(),
            "protein_g": round1(self.macros.protein_g),
            "estimated": self.source == NutritionSource::LlmEstimate,
        })
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
}
