//! Shared meals (`docs/MULTIMEMBER_SPEC.md` §6.2–6.4): the diet side of the multimember plugin.
//!
//! The multimember plugin plans first and leaves a computed `multimember` section in the draft:
//! which main meals other members share with the owner, and each guest's profile. Diet never
//! depends on that plugin; it reads the section's JSON. Guests have no goals: each gets a maintenance
//! estimate from this crate's own targets engine (goal `None`), and a portion `scale` relative
//! to the owner's, computed here. The planner only picks the dishes; `items` stay the owner's
//! portion, so the owner's targets, validation and plan-vs-actual don't change.

use chrono::NaiveDate;
use lifeagent_core::plugin::PlanDraft;
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::model::{round1, Macros, MealType};
use crate::targets::{self, NutritionTargets};

pub const MULTIMEMBER_SECTION: &str = "multimember";
pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 2.0;

#[derive(Debug, Clone, Deserialize)]
pub struct Guest {
    pub member_id: String,
    pub name: String,
    pub sex: Sex,
    pub age: i32,
    pub height_cm: f64,
    pub weight_kg: f64,
    pub activity_level: ActivityLevel,
    pub dietary_pattern: DietaryPattern,
    #[serde(default)]
    pub allergies: Vec<String>,
    #[serde(default)]
    pub dislikes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedMeal {
    pub meal_type: MealType,
    pub guest_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Section {
    meals: Vec<SharedMeal>,
    guests: Vec<Guest>,
}

/// A guest planned for: their maintenance estimate and portion scale.
#[derive(Debug, Clone)]
pub struct GuestPlan {
    pub guest: Guest,
    pub estimate: NutritionTargets,
    pub scale: f64,
    /// The unclamped ratio was outside `MIN_SCALE..=MAX_SCALE`.
    pub clamped: bool,
}

/// Who shares which meals on the plan date, with each guest's estimate and scale.
#[derive(Debug, Clone)]
pub struct SharedMeals {
    pub meals: Vec<SharedMeal>,
    pub guests: Vec<GuestPlan>,
}

impl SharedMeals {
    /// From the draft's `multimember` section, given the owner's day kcal target. `None` when
    /// the section is absent, malformed, or shares no meal: diet then plans for the owner alone.
    pub fn from_draft(prior: &PlanDraft, owner_kcal: f64) -> Option<SharedMeals> {
        let raw = prior.get(MULTIMEMBER_SECTION)?;
        let section: Section = match serde_json::from_value(raw.clone()) {
            Ok(s) => s,
            Err(e) => {
                // The error text can quote member data, so it stays at DEBUG.
                tracing::debug!(error = %e, "malformed multimember section");
                tracing::warn!("ignoring a malformed multimember section");
                return None;
            }
        };
        let guests: Vec<GuestPlan> = section
            .guests
            .into_iter()
            .map(|guest| {
                let estimate = maintenance(&guest);
                let (scale, clamped) = scale(estimate.kcal, owner_kcal);
                GuestPlan {
                    guest,
                    estimate,
                    scale,
                    clamped,
                }
            })
            .collect();
        let meals: Vec<SharedMeal> = section
            .meals
            .into_iter()
            .filter(|m| m.meal_type != MealType::Snack)
            .map(|m| SharedMeal {
                guest_ids: m
                    .guest_ids
                    .into_iter()
                    .filter(|id| guests.iter().any(|g| &g.guest.member_id == id))
                    .collect(),
                ..m
            })
            .filter(|m| !m.guest_ids.is_empty())
            .collect();
        if meals.is_empty() {
            return None;
        }
        Some(SharedMeals { meals, guests })
    }

    pub fn meal(&self, meal_type: MealType) -> Option<&SharedMeal> {
        self.meals.iter().find(|m| m.meal_type == meal_type)
    }

    pub fn guests_at(&self, meal: &SharedMeal) -> Vec<&GuestPlan> {
        meal.guest_ids
            .iter()
            .filter_map(|id| self.guests.iter().find(|g| &g.guest.member_id == id))
            .collect()
    }

    /// Total portions cooked for `meal`, the owner's included: `1 + Σ scale`, to 0.1.
    pub fn portions(&self, meal: &SharedMeal) -> f64 {
        round1(1.0 + self.guests_at(meal).iter().map(|g| g.scale).sum::<f64>())
    }

    /// `{member_id: estimate}`, for display.
    pub fn estimates_json(&self) -> Value {
        let map: serde_json::Map<String, Value> = self
            .guests
            .iter()
            .filter(|g| {
                self.meals
                    .iter()
                    .any(|m| m.guest_ids.contains(&g.guest.member_id))
            })
            .map(|g| (g.guest.member_id.clone(), g.estimate.as_macros().to_json()))
            .collect();
        Value::Object(map)
    }

    /// `servings` for one meal: each guest's scale and their share of `meal_total`.
    pub fn servings_json(&self, meal: &SharedMeal, meal_total: Macros) -> Value {
        let servings: Vec<Value> = self
            .guests_at(meal)
            .into_iter()
            .map(|g| {
                json!({
                    "member_id": g.guest.member_id,
                    "name": g.guest.name,
                    "scale": g.scale,
                    "total": meal_total.times(g.scale).to_json(),
                })
            })
            .collect();
        json!(servings)
    }
}

/// A guest's daily needs to keep their weight: the targets engine with no goal (= Maintain).
pub fn maintenance(guest: &Guest) -> NutritionTargets {
    let profile = Profile {
        display_name: None,
        sex: guest.sex,
        // Unused by the engine, which takes the age directly.
        birth_date: NaiveDate::MIN,
        height_cm: guest.height_cm,
        activity_level: guest.activity_level,
        dietary_pattern: guest.dietary_pattern,
        allergies: guest.allergies.clone(),
        medical_notes: None,
        meals_per_day: 3,
    };
    targets::compute(&profile, guest.weight_kg, guest.age, None)
}

/// `guest_kcal / owner_kcal`, clamped to `MIN_SCALE..=MAX_SCALE`, to 2 decimals; and whether it
/// was clamped (§6.4).
pub fn scale(guest_kcal: f64, owner_kcal: f64) -> (f64, bool) {
    let ratio = if owner_kcal > 0.0 {
        guest_kcal / owner_kcal
    } else {
        1.0
    };
    let clamped = ratio.clamp(MIN_SCALE, MAX_SCALE);
    ((clamped * 100.0).round() / 100.0, clamped != ratio)
}

fn strictness(p: DietaryPattern) -> u8 {
    match p {
        DietaryPattern::Omnivore => 0,
        DietaryPattern::Pescatarian => 1,
        DietaryPattern::Vegetarian => 2,
        DietaryPattern::Vegan => 3,
    }
}

/// The strictest pattern among `patterns`: vegan > vegetarian > pescatarian > omnivore.
pub fn strictest(patterns: impl IntoIterator<Item = DietaryPattern>) -> DietaryPattern {
    patterns
        .into_iter()
        .max_by_key(|p| strictness(*p))
        .unwrap_or(DietaryPattern::Omnivore)
}

pub fn pattern_name(p: DietaryPattern) -> &'static str {
    match p {
        DietaryPattern::Omnivore => "omnivore",
        DietaryPattern::Pescatarian => "pescatarian",
        DietaryPattern::Vegetarian => "vegetarian",
        DietaryPattern::Vegan => "vegan",
    }
}

fn pattern_zh(p: DietaryPattern) -> &'static str {
    match p {
        DietaryPattern::Omnivore => "杂食",
        DietaryPattern::Pescatarian => "鱼素",
        DietaryPattern::Vegetarian => "素食",
        DietaryPattern::Vegan => "纯素",
    }
}

const MEAT_TERMS: &[&str] = &[
    "肉",
    "ベーコン",
    "ハム",
    "ソーセージ",
    "チキン",
    "ささみ",
    "培根",
    "火腿",
    "香肠",
    "beef",
    "pork",
    "chicken",
    "bacon",
    "ham",
    "sausage",
    "lamb",
];

const SEAFOOD_TERMS: &[&str] = &[
    "魚",
    "鱼",
    "鮭",
    "サーモン",
    "鮪",
    "まぐろ",
    "ツナ",
    "鯖",
    "さば",
    "えび",
    "海老",
    "エビ",
    "虾",
    "蟹",
    "かに",
    "カニ",
    "いか",
    "イカ",
    "たこ",
    "タコ",
    "貝",
    "贝",
    "しらす",
    "fish",
    "salmon",
    "tuna",
    "shrimp",
    "prawn",
    "crab",
    "squid",
    "octopus",
];

const ANIMAL_PRODUCT_TERMS: &[&str] = &[
    "卵",
    "玉子",
    "たまご",
    "鸡蛋",
    "蛋黄",
    "蛋清",
    "牛乳",
    "チーズ",
    "ヨーグルト",
    "バター",
    "生クリーム",
    "牛奶",
    "奶酪",
    "酸奶",
    "黄油",
    "蜂蜜",
    "はちみつ",
    "egg",
    "milk",
    "cheese",
    "yogurt",
    "butter",
    "honey",
];

/// The built-in terms a food name must not contain under `pattern` (§6.3): a small list,
/// matched like allergies.
pub fn forbidden_terms(pattern: DietaryPattern) -> Vec<&'static str> {
    let lists: &[&[&str]] = match pattern {
        DietaryPattern::Omnivore => &[],
        DietaryPattern::Pescatarian => &[MEAT_TERMS],
        DietaryPattern::Vegetarian => &[MEAT_TERMS, SEAFOOD_TERMS],
        DietaryPattern::Vegan => &[MEAT_TERMS, SEAFOOD_TERMS, ANIMAL_PRODUCT_TERMS],
    };
    lists.iter().flat_map(|l| l.iter().copied()).collect()
}

/// The planner brief's lines for shared meals (§6.2). `owner_allergies` and `owner_pattern`
/// are the owner's, who is at every meal.
pub fn brief_lines(
    shared: &SharedMeals,
    owner_allergies: &[String],
    owner_pattern: DietaryPattern,
) -> Vec<String> {
    let mut lines = vec![
        "Shared meals: other people eat some main meals with the owner. Plan ONE \
         set of dishes for everyone at such a meal; `items` stay the owner's own portion in \
         grams (the app sizes the others' portions). The others aim to keep their current \
         weight: keep shared meals balanced (protein, vegetables, moderate fat and salt)."
            .to_string(),
    ];
    for meal in &shared.meals {
        let guests = shared.guests_at(meal);
        let names: Vec<&str> = guests.iter().map(|g| g.guest.name.as_str()).collect();
        let mut allergies: Vec<String> = owner_allergies.to_vec();
        let mut dislikes: Vec<String> = Vec::new();
        for g in &guests {
            for a in &g.guest.allergies {
                if !allergies.contains(a) {
                    allergies.push(a.clone());
                }
            }
            for d in &g.guest.dislikes {
                if !dislikes.contains(d) {
                    dislikes.push(d.clone());
                }
            }
        }
        let pattern = strictest(
            std::iter::once(owner_pattern).chain(guests.iter().map(|g| g.guest.dietary_pattern)),
        );
        let meal_name = format!("{:?}", meal.meal_type).to_lowercase();
        let mut line = format!(
            "- Shared {meal_name} with {}: cook about {:.1} portions in total. Dietary pattern \
             at this meal: {}.",
            names.join(", "),
            shared.portions(meal),
            pattern_name(pattern)
        );
        if !allergies.is_empty() {
            line.push_str(&format!(
                " Never include (allergies at this table, hard constraint): {}.",
                allergies.join("、")
            ));
        }
        if !dislikes.is_empty() {
            line.push_str(&format!(
                " Avoid (guests' dislikes): {}.",
                dislikes.join("、")
            ));
        }
        lines.push(line);
    }
    lines
}

/// The checks for one item of a shared meal (§6.3), as `(violations, warnings)`: guests'
/// allergies and the table's dietary pattern are violations; guests' dislikes are warnings.
/// `names` are the planned and resolved names of the item.
pub fn check_item(
    guests: &[&GuestPlan],
    table_pattern: DietaryPattern,
    meal_label: &str,
    item_name: &str,
    names: &[&str],
    matches_term: impl Fn(&str, &str) -> bool,
) -> (Vec<String>, Vec<String>) {
    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    for g in guests {
        for allergy in &g.guest.allergies {
            if names.iter().any(|n| matches_term(n, allergy)) {
                violations.push(format!(
                    "{meal_label}中的“{item_name}”含{}的过敏原“{allergy}”",
                    g.guest.name
                ));
            }
        }
        for dislike in &g.guest.dislikes {
            if names.iter().any(|n| matches_term(n, dislike)) {
                warnings.push(format!(
                    "“{item_name}”含{}不喜欢的“{dislike}”",
                    g.guest.name
                ));
            }
        }
    }
    if let Some(term) = forbidden_terms(table_pattern)
        .into_iter()
        .find(|t| names.iter().any(|n| matches_term(n, t)))
    {
        violations.push(format!(
            "{meal_label}是共餐，需符合{}：“{item_name}”含“{term}”",
            pattern_zh(table_pattern)
        ));
    }
    (violations, warnings)
}

/// One warning per guest whose scale was clamped.
pub fn clamp_warnings(shared: &SharedMeals) -> Vec<String> {
    shared
        .guests
        .iter()
        .filter(|g| g.clamped)
        .map(|g| {
            format!(
                "{}的份量倍数已限制在 {MIN_SCALE}–{MAX_SCALE} 之间（{:.2}）",
                g.guest.name, g.scale
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest(id: &str, sex: Sex, weight: f64) -> Value {
        json!({
            "member_id": id, "name": id, "sex": sex, "age": 34, "height_cm": 165,
            "weight_kg": weight, "activity_level": "light", "dietary_pattern": "omnivore",
            "allergies": [], "dislikes": []
        })
    }

    fn draft(section: Value) -> PlanDraft {
        PlanDraft::from([(MULTIMEMBER_SECTION.to_string(), section)])
    }

    #[test]
    fn maintenance_is_the_engine_with_no_goal() {
        let g: Guest = serde_json::from_value(guest("a", Sex::Female, 55.0)).unwrap();
        let profile = Profile {
            display_name: None,
            sex: Sex::Female,
            birth_date: NaiveDate::MIN,
            height_cm: 165.0,
            activity_level: ActivityLevel::Light,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec![],
            medical_notes: None,
            meals_per_day: 3,
        };
        let expected = targets::compute(&profile, 55.0, 34, None);
        assert_eq!(maintenance(&g), expected);
        assert_eq!(expected.delta, 0.0);
    }

    #[test]
    fn scale_is_the_kcal_ratio_clamped_and_rounded() {
        assert_eq!(scale(1800.0, 2000.0), (0.9, false));
        assert_eq!(scale(2000.0, 1500.0), (1.33, false));
        assert_eq!(scale(5000.0, 2000.0), (2.0, true));
        assert_eq!(scale(500.0, 2000.0), (0.5, true));
    }

    #[test]
    fn strictest_pattern_wins() {
        use DietaryPattern::*;
        assert_eq!(strictest([Omnivore, Pescatarian]), Pescatarian);
        assert_eq!(strictest([Pescatarian, Vegetarian, Omnivore]), Vegetarian);
        assert_eq!(strictest([Vegan, Vegetarian]), Vegan);
        assert_eq!(strictest([]), Omnivore);
    }

    #[test]
    fn from_draft_drops_snacks_unknown_guests_and_empty_meals() {
        let section = json!({
            "meals": [
                {"meal_type": "breakfast", "guest_ids": ["nobody"]},
                {"meal_type": "snack", "guest_ids": ["a"]},
                {"meal_type": "dinner", "guest_ids": ["a", "b"]}
            ],
            "guests": [guest("a", Sex::Female, 55.0), guest("b", Sex::Male, 70.0)]
        });
        let h = SharedMeals::from_draft(&draft(section), 2000.0).unwrap();
        assert_eq!(h.meals.len(), 1);
        let dinner = h.meal(MealType::Dinner).unwrap();
        let scales: f64 = h.guests_at(dinner).iter().map(|g| g.scale).sum();
        assert_eq!(h.portions(dinner), round1(1.0 + scales));
        assert!(h.meal(MealType::Breakfast).is_none());

        assert!(SharedMeals::from_draft(&PlanDraft::new(), 2000.0).is_none());
        assert!(
            SharedMeals::from_draft(&draft(json!({"meals": [], "guests": []})), 2000.0).is_none()
        );
        assert!(SharedMeals::from_draft(&draft(json!({"bad": true})), 2000.0).is_none());
    }

    #[test]
    fn servings_scale_the_meal_total() {
        let section = json!({
            "meals": [{"meal_type": "dinner", "guest_ids": ["a"]}],
            "guests": [guest("a", Sex::Female, 55.0)]
        });
        let h = SharedMeals::from_draft(&draft(section), 2000.0).unwrap();
        let s = h.guests[0].scale;
        let total = Macros {
            kcal: 600.0,
            protein_g: 40.0,
            carbs_g: 70.0,
            fat_g: 15.0,
            fiber_g: 8.0,
        };
        let servings = h.servings_json(&h.meals[0], total);
        assert_eq!(servings[0]["scale"], s);
        assert_eq!(servings[0]["total"]["kcal"], (600.0 * s).round());
        assert_eq!(servings[0]["name"], "a");
        assert!(h.estimates_json()["a"]["kcal"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn check_item_flags_guest_allergies_table_pattern_and_dislikes() {
        let mut veg = guest("v", Sex::Female, 55.0);
        veg["dietary_pattern"] = json!("vegetarian");
        veg["allergies"] = json!(["くるみ"]);
        veg["dislikes"] = json!(["セロリ"]);
        let section = json!({
            "meals": [{"meal_type": "dinner", "guest_ids": ["v"]}],
            "guests": [veg]
        });
        let h = SharedMeals::from_draft(&draft(section), 2000.0).unwrap();
        let guests = h.guests_at(&h.meals[0]);
        let contains = |n: &str, t: &str| n.contains(t);
        let check = |name: &str| {
            check_item(
                &guests,
                DietaryPattern::Vegetarian,
                "晚餐“x”",
                name,
                &[name],
                contains,
            )
        };

        let (v, w) = check("鶏むね肉");
        assert_eq!(v, vec!["晚餐“x”是共餐，需符合素食：“鶏むね肉”含“肉”"]);
        assert!(w.is_empty());
        let (v, _) = check("くるみパン");
        assert_eq!(v, vec!["晚餐“x”中的“くるみパン”含v的过敏原“くるみ”"]);
        let (v, w) = check("セロリ炒め");
        assert!(v.is_empty());
        assert_eq!(w, vec!["“セロリ炒め”含v不喜欢的“セロリ”"]);
        assert_eq!(check("豆腐"), (vec![], vec![]));
    }

    #[test]
    fn clamped_scales_are_warned() {
        let section = json!({
            "meals": [{"meal_type": "dinner", "guest_ids": ["big"]}],
            "guests": [guest("big", Sex::Male, 150.0)]
        });
        let h = SharedMeals::from_draft(&draft(section), 1200.0).unwrap();
        assert_eq!(h.guests[0].scale, MAX_SCALE);
        assert_eq!(clamp_warnings(&h).len(), 1);
    }

    #[test]
    fn pattern_terms_nest() {
        use DietaryPattern::*;
        assert!(forbidden_terms(Omnivore).is_empty());
        assert!(forbidden_terms(Pescatarian).contains(&"肉"));
        assert!(!forbidden_terms(Pescatarian).contains(&"鮭"));
        assert!(forbidden_terms(Vegetarian).contains(&"鮭"));
        assert!(!forbidden_terms(Vegetarian).contains(&"卵"));
        assert!(forbidden_terms(Vegan).contains(&"卵"));
        // Soy milk is fine for vegans: no bare "乳"/"奶" term.
        assert!(!forbidden_terms(Vegan)
            .iter()
            .any(|t| "豆乳".contains(t) || "豆奶".contains(t)));
    }
}
