//! Members, dislikes and attendance as data, the validation rules of
//! `docs/MULTIMEMBER_SPEC.md` §5, and the plugin's error type.

use chrono::{DateTime, NaiveDate, Utc};
use lifeagent_core::error::CoreError;
use lifeagent_core::plugin::PluginError;
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Sex};
use lifeagent_core::time::age_years;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MIN_AGE: i32 = 16;
pub const MAX_AGE: i32 = 70;
/// Non-archived members besides the owner.
pub const MAX_MEMBERS: usize = 7;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_ALLERGIES: usize = 20;
pub const MAX_ALLERGY_CHARS: usize = 40;
pub const MAX_DISLIKE_CHARS: usize = 100;
pub const MAX_NOTE_CHARS: usize = 100;

/// A member's profile: everything the owner enters about them. Weight is a plain field, edited
/// by hand; members have no logged history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberProfile {
    pub display_name: String,
    pub sex: Sex,
    pub birth_date: NaiveDate,
    pub height_cm: f64,
    pub weight_kg: f64,
    pub activity_level: ActivityLevel,
    pub dietary_pattern: DietaryPattern,
    #[serde(default)]
    pub allergies: Vec<String>,
}

impl MemberProfile {
    /// Trims the name and allergies, drops blank allergies and checks every field-level rule
    /// (§5). Name uniqueness and the member count need the db and are checked by the store.
    pub fn normalized(&self, today: NaiveDate) -> Result<MemberProfile, MemberRule> {
        let display_name = self.display_name.trim().to_string();
        let name_len = display_name.chars().count();
        if name_len == 0 || name_len > MAX_NAME_CHARS {
            return Err(MemberRule::NameInvalid);
        }
        let age = age_years(self.birth_date, today);
        if !(MIN_AGE..=MAX_AGE).contains(&age) {
            return Err(MemberRule::AgeOutOfRange { got: age });
        }
        if !(120.0..=230.0).contains(&self.height_cm) {
            return Err(MemberRule::HeightOutOfRange {
                got: self.height_cm,
            });
        }
        if !(30.0..=300.0).contains(&self.weight_kg) {
            return Err(MemberRule::WeightOutOfRange {
                got: self.weight_kg,
            });
        }
        let allergies: Vec<String> = self
            .allergies
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        if allergies.len() > MAX_ALLERGIES
            || allergies
                .iter()
                .any(|a| a.chars().count() > MAX_ALLERGY_CHARS)
        {
            return Err(MemberRule::AllergiesInvalid);
        }
        Ok(MemberProfile {
            display_name,
            allergies,
            ..self.clone()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Member {
    pub id: String,
    #[serde(flatten)]
    pub profile: MemberProfile,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Member {
    pub fn age_on(&self, date: NaiveDate) -> i32 {
        age_years(self.profile.birth_date, date)
    }

    /// Whether the member may be planned for on `date` (ages move on after validation).
    pub fn in_age_range_on(&self, date: NaiveDate) -> bool {
        (MIN_AGE..=MAX_AGE).contains(&self.age_on(date))
    }
}

/// A main meal: the only meals other members can share (snacks are the owner's alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MealType {
    Breakfast,
    Lunch,
    Dinner,
}

impl MealType {
    pub const ALL: [MealType; 3] = [MealType::Breakfast, MealType::Lunch, MealType::Dinner];

    pub fn as_str(self) -> &'static str {
        match self {
            MealType::Breakfast => "breakfast",
            MealType::Lunch => "lunch",
            MealType::Dinner => "dinner",
        }
    }

    pub fn parse(s: &str) -> Option<MealType> {
        MealType::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dislike {
    pub id: String,
    pub member_id: String,
    pub text: String,
    pub valid_from: Option<NaiveDate>,
    pub valid_until: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

impl Dislike {
    pub fn is_active_on(&self, date: NaiveDate) -> bool {
        self.valid_from.is_none_or(|f| f <= date) && self.valid_until.is_none_or(|u| date <= u)
    }
}

/// One weekly default: whether a member usually eats `meal_type` on `weekday` (0 = Monday).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultSlot {
    pub weekday: u8,
    pub meal_type: MealType,
    pub present: bool,
}

/// A one-off change to the weekly default for one date and meal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttendanceOverride {
    pub member_id: String,
    pub plan_date: NaiveDate,
    pub meal_type: MealType,
    pub present: bool,
    pub note: Option<String>,
}

/// A rule a member, dislike or attendance change broke (§5). Leaves Rust as data so the UI words
/// it (`ErrorDetail` in `lifeagent-api`); the English `Display` is for the LLM and logs.
#[derive(Debug, Clone, Copy, PartialEq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemberRule {
    #[error("display_name must be 1–{MAX_NAME_CHARS} characters")]
    NameInvalid,
    #[error("another member already has this display_name")]
    NameTaken,
    #[error("members must be aged {MIN_AGE}–{MAX_AGE}; this birth_date gives {got}")]
    AgeOutOfRange { got: i32 },
    #[error("height_cm must be within 120..=230, got {got}")]
    HeightOutOfRange { got: f64 },
    #[error("weight_kg must be within 30..=300, got {got}")]
    WeightOutOfRange { got: f64 },
    #[error("at most {MAX_ALLERGIES} allergies, each 1–{MAX_ALLERGY_CHARS} characters")]
    AllergiesInvalid,
    #[error("at most {max} members besides the owner")]
    TooManyMembers { max: usize },
    #[error("dislike text must not be empty")]
    DislikeEmpty,
    #[error("dislike text must be at most {max} characters, got {got}")]
    DislikeTooLong { got: usize, max: usize },
    #[error("valid_from {valid_from} must not be later than valid_until {valid_until}")]
    DatesReversed {
        valid_from: NaiveDate,
        valid_until: NaiveDate,
    },
    #[error(
        "attendance needs a main meal (breakfast, lunch, dinner), a weekday 0–6, a date not in \
         the past, and a note of at most {MAX_NOTE_CHARS} characters"
    )]
    AttendanceInvalid,
}

#[derive(Debug, Error)]
pub enum MemberError {
    #[error("{0}")]
    Invalid(MemberRule),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Plugin(#[from] PluginError),
}

impl From<MemberRule> for MemberError {
    fn from(rule: MemberRule) -> Self {
        MemberError::Invalid(rule)
    }
}

impl From<sqlx::Error> for MemberError {
    fn from(e: sqlx::Error) -> Self {
        MemberError::Plugin(PluginError::Db(e))
    }
}

impl From<CoreError> for MemberError {
    fn from(e: CoreError) -> Self {
        MemberError::Plugin(e.into())
    }
}

/// For tools: a broken rule is an argument the model can fix.
impl From<MemberError> for PluginError {
    fn from(e: MemberError) -> Self {
        match e {
            MemberError::Invalid(rule) => PluginError::InvalidArgs(rule.to_string()),
            MemberError::NotFound(m) => PluginError::NotFound(m),
            MemberError::Plugin(p) => p,
        }
    }
}

/// `snake_case` name of a serde unit enum (`Sex`, `ActivityLevel`, ...), as stored in TEXT.
pub(crate) fn enum_str<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        // Only called with unit enums, which always serialize to a string.
        _ => String::new(),
    }
}

pub(crate) fn parse_enum<T: for<'de> Deserialize<'de>>(s: &str) -> Result<T, PluginError> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|e| PluginError::Other(format!("bad stored value {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    pub(crate) fn profile() -> MemberProfile {
        MemberProfile {
            display_name: " Aki ".into(),
            sex: Sex::Female,
            birth_date: d(1992, 5, 10),
            height_cm: 160.0,
            weight_kg: 52.0,
            activity_level: ActivityLevel::Light,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec![" えび ".into(), "  ".into()],
        }
    }

    #[test]
    fn normalizes_name_and_allergies() {
        let p = profile().normalized(d(2026, 10, 4)).unwrap();
        assert_eq!(p.display_name, "Aki");
        assert_eq!(p.allergies, vec!["えび".to_string()]);
    }

    #[test]
    fn age_bounds_are_16_and_70_inclusive_on_the_birthday() {
        let today = d(2026, 10, 4);
        let born = |y| MemberProfile {
            birth_date: d(y, 10, 4),
            ..profile()
        };
        let day_after = |y| MemberProfile {
            birth_date: d(y, 10, 5),
            ..profile()
        };
        // Turns 16 today: ok. Turns 16 tomorrow: 15.
        assert!(born(2010).normalized(today).is_ok());
        assert_eq!(
            day_after(2010).normalized(today),
            Err(MemberRule::AgeOutOfRange { got: 15 })
        );
        // Still 70 until the 71st birthday tomorrow; 71 from that day.
        assert!(day_after(1955).normalized(today).is_ok());
        assert_eq!(
            born(1955).normalized(today),
            Err(MemberRule::AgeOutOfRange { got: 71 })
        );
    }

    #[test]
    fn field_rules() {
        let today = d(2026, 10, 4);
        let cases = [
            (
                MemberProfile {
                    display_name: "  ".into(),
                    ..profile()
                },
                MemberRule::NameInvalid,
            ),
            (
                MemberProfile {
                    display_name: "名".repeat(MAX_NAME_CHARS + 1),
                    ..profile()
                },
                MemberRule::NameInvalid,
            ),
            (
                MemberProfile {
                    height_cm: 119.9,
                    ..profile()
                },
                MemberRule::HeightOutOfRange { got: 119.9 },
            ),
            (
                MemberProfile {
                    weight_kg: 300.5,
                    ..profile()
                },
                MemberRule::WeightOutOfRange { got: 300.5 },
            ),
            (
                MemberProfile {
                    allergies: vec!["x".into(); MAX_ALLERGIES + 1],
                    ..profile()
                },
                MemberRule::AllergiesInvalid,
            ),
            (
                MemberProfile {
                    allergies: vec!["长".repeat(MAX_ALLERGY_CHARS + 1)],
                    ..profile()
                },
                MemberRule::AllergiesInvalid,
            ),
        ];
        for (p, rule) in cases {
            assert_eq!(p.normalized(today), Err(rule));
        }
    }

    #[test]
    fn rules_serialize_with_a_kind_tag() {
        assert_eq!(
            serde_json::to_value(MemberRule::AgeOutOfRange { got: 71 }).unwrap(),
            serde_json::json!({"kind": "age_out_of_range", "got": 71})
        );
    }
}
