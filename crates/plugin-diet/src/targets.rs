//! Nutrition targets engine — pure functions, per `docs/PHASE1_SPEC.md` §8.6 (as amended by
//! B1 in `docs/DECISIONS.md`).

use lifeagent_core::goals::{GoalDirection, WeightGoal};
use lifeagent_core::profile::{ActivityLevel, Profile, Sex};
use serde::{Deserialize, Serialize};

use crate::model::{round1, Macros};

/// Carbs never planned below this (g), rather than let protein and fat crowd them out.
const MIN_CARBS_G: f64 = 100.0;

/// Share of a planned session's net exercise energy added to the day's kcal target. Only half:
/// the activity factor in the TDEE already counts part of the user's usual exercise, and eating
/// back half of it is the common conservative rule (`docs/DECISIONS.md`, "Training plugin").
pub const EXERCISE_CREDIT: f64 = 0.5;
/// Cap on the kcal added for one day's training.
pub const MAX_EXERCISE_CREDIT_KCAL: f64 = 600.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NutritionTargets {
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
    pub bmr: f64,
    pub tdee: f64,
    pub delta: f64,
    /// `kcal` was raised above `round_up_to_10(tdee + delta)` by a safety floor.
    pub floor_applied: bool,
}

impl NutritionTargets {
    pub fn as_macros(&self) -> Macros {
        Macros {
            kcal: self.kcal,
            protein_g: self.protein_g,
            carbs_g: self.carbs_g,
            fat_g: self.fat_g,
            fiber_g: self.fiber_g,
        }
    }
}

fn activity_factor(level: ActivityLevel) -> f64 {
    match level {
        ActivityLevel::Sedentary => 1.2,
        ActivityLevel::Light => 1.375,
        ActivityLevel::Moderate => 1.55,
        ActivityLevel::Active => 1.725,
        ActivityLevel::VeryActive => 1.9,
    }
}

/// Rounds up to the next multiple of 10. The epsilon keeps float noise (2010.0000000001)
/// from bumping an exact multiple up a step.
fn round_up_to_10(x: f64) -> f64 {
    ((x / 10.0) - 1e-9).ceil() * 10.0
}

/// Daily targets for a person of `weight_kg` and `age` (§8.6). No active goal is treated as
/// Maintain with no target weight. Outputs are rounded: kcal values to 1, grams to 0.1.
pub fn compute(
    profile: &Profile,
    weight_kg: f64,
    age: i32,
    goal: Option<&WeightGoal>,
) -> NutritionTargets {
    let w = weight_kg;
    let sex_term = match profile.sex {
        Sex::Male => 5.0,
        Sex::Female => -161.0,
    };
    let bmr = 10.0 * w + 6.25 * profile.height_cm - 5.0 * f64::from(age) + sex_term;
    let tdee = bmr * activity_factor(profile.activity_level);

    let (direction, target, rate) = match goal {
        Some(g) => (g.direction, g.target_weight_kg, g.rate_kg_per_week),
        None => (GoalDirection::Maintain, None, 0.0),
    };
    let rate = rate.min(0.01 * w);
    let effective = match direction {
        GoalDirection::Lose if target.is_some_and(|t| w <= t) => GoalDirection::Maintain,
        GoalDirection::Gain if target.is_some_and(|t| w >= t) => GoalDirection::Maintain,
        d => d,
    };

    let delta = match effective {
        GoalDirection::Lose => -(rate * 7700.0 / 7.0).min(0.25 * tdee),
        GoalDirection::Gain => (rate * 7700.0 / 7.0).min(500.0),
        GoalDirection::Maintain => 0.0,
    };

    let floor = bmr.max(match profile.sex {
        Sex::Male => 1500.0,
        Sex::Female => 1200.0,
    });
    let unfloored_kcal = round_up_to_10(tdee + delta);
    let mut kcal = round_up_to_10((tdee + delta).max(floor));

    let anchor = target.unwrap_or(w);
    let protein_factor = match effective {
        GoalDirection::Lose => 1.8,
        GoalDirection::Gain => 1.6,
        GoalDirection::Maintain => 1.2,
    };
    let protein = (anchor * protein_factor).max(0.8 * w);
    let fat_floor = 0.6 * w;
    let mut fat = (0.27 * kcal / 9.0).max(fat_floor);
    let mut carbs = (kcal - 4.0 * protein - 9.0 * fat) / 4.0;

    if carbs < MIN_CARBS_G {
        fat = fat_floor.max((kcal - 4.0 * protein - 4.0 * MIN_CARBS_G) / 9.0);
        carbs = (kcal - 4.0 * protein - 9.0 * fat) / 4.0;
    }
    if carbs < MIN_CARBS_G {
        // The floors alone exceed the budget: raise kcal to fit all three. Carbs are then
        // recomputed from the rounded-up kcal, so they come out ≥ 100 g and the macros still
        // add up to kcal exactly.
        kcal = round_up_to_10(4.0 * protein + 9.0 * fat_floor + 4.0 * MIN_CARBS_G);
        fat = fat_floor;
        carbs = (kcal - 4.0 * protein - 9.0 * fat) / 4.0;
    }

    NutritionTargets {
        kcal,
        protein_g: round1(protein),
        carbs_g: round1(carbs),
        fat_g: round1(fat),
        fiber_g: round1(14.0 * kcal / 1000.0),
        bmr: bmr.round(),
        tdee: tdee.round(),
        delta: delta.round(),
        floor_applied: kcal > unfloored_kcal,
    }
}

/// The kcal added to the target for `exercise_kcal` of planned net exercise energy: the
/// [`EXERCISE_CREDIT`] share, capped, rounded up to 10. 0 for no (or a negative) amount.
pub fn exercise_credit(exercise_kcal: f64) -> f64 {
    let credit = (exercise_kcal * EXERCISE_CREDIT).min(MAX_EXERCISE_CREDIT_KCAL);
    if credit > 0.0 {
        round_up_to_10(credit)
    } else {
        0.0
    }
}

/// `t` raised for a day with `exercise_kcal` of planned training: the credit goes to carbs, the
/// session's fuel, and fiber follows kcal as in [`compute`]. Protein and fat are unchanged.
pub fn with_exercise(t: NutritionTargets, exercise_kcal: f64) -> NutritionTargets {
    let add = exercise_credit(exercise_kcal);
    if add == 0.0 {
        return t;
    }
    let kcal = t.kcal + add;
    NutritionTargets {
        kcal,
        carbs_g: round1(t.carbs_g + add / 4.0),
        fiber_g: round1(14.0 * kcal / 1000.0),
        ..t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn exercise_credit_is_half_capped_and_rounded_up() {
        assert_eq!(exercise_credit(0.0), 0.0);
        assert_eq!(exercise_credit(-50.0), 0.0);
        assert_eq!(exercise_credit(301.0), 160.0);
        assert_eq!(exercise_credit(5000.0), MAX_EXERCISE_CREDIT_KCAL);
    }

    #[test]
    fn with_exercise_adds_the_credit_to_kcal_and_carbs() {
        let base = compute(
            &profile(Sex::Male, 180.0, ActivityLevel::Moderate),
            80.0,
            30,
            Some(&goal(GoalDirection::Lose, Some(75.0), 0.5)),
        );
        let t = with_exercise(base, 400.0);
        assert_eq!(t.kcal, base.kcal + 200.0);
        assert_eq!(t.carbs_g, round1(base.carbs_g + 50.0));
        assert_eq!(t.protein_g, base.protein_g);
        assert_eq!(t.fat_g, base.fat_g);
        assert_eq!(t.fiber_g, round1(14.0 * t.kcal / 1000.0));
        assert_eq!(with_exercise(base, 0.0), base);
    }
    use lifeagent_core::profile::DietaryPattern;

    fn profile(sex: Sex, height_cm: f64, activity_level: ActivityLevel) -> Profile {
        Profile {
            display_name: None,
            sex,
            birth_date: NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(),
            height_cm,
            activity_level,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec![],
            medical_notes: None,
            meals_per_day: 3,
        }
    }

    fn goal(direction: GoalDirection, target: Option<f64>, rate: f64) -> WeightGoal {
        WeightGoal {
            direction,
            target_weight_kg: target,
            rate_kg_per_week: rate,
        }
    }

    #[test]
    fn hand_computed_male_lose() {
        // Male, 80 kg, 180 cm, 30 y, Moderate, Lose 0.5 kg/week to 75 kg:
        // BMR = 800 + 1125 − 150 + 5 = 1780; TDEE = 1780 × 1.55 = 2759
        // delta = −min(0.5·7700/7 = 550, 0.25·2759 = 689.75) = −550
        // kcal = ceil10(max(2209, 1780)) = 2210
        // protein = max(75 × 1.8, 0.8 × 80) = 135; fat = max(0.27·2210/9 = 66.3, 48) = 66.3
        // carbs = (2210 − 540 − 596.7) / 4 = 268.325; fiber = 14 × 2.21 = 30.94
        let t = compute(
            &profile(Sex::Male, 180.0, ActivityLevel::Moderate),
            80.0,
            30,
            Some(&goal(GoalDirection::Lose, Some(75.0), 0.5)),
        );
        assert_eq!(
            t,
            NutritionTargets {
                kcal: 2210.0,
                protein_g: 135.0,
                carbs_g: 268.3,
                fat_g: 66.3,
                fiber_g: 30.9,
                bmr: 1780.0,
                tdee: 2759.0,
                delta: -550.0,
                floor_applied: false,
            }
        );
    }

    #[test]
    fn female_bmr_uses_minus_161() {
        let t = compute(
            &profile(Sex::Female, 165.0, ActivityLevel::Sedentary),
            60.0,
            30,
            None,
        );
        // 600 + 1031.25 − 150 − 161 = 1320.25
        assert_eq!(t.bmr, 1320.0);
        assert_eq!(t.delta, 0.0);
    }

    #[test]
    fn gain_delta_is_capped_at_500() {
        let t = compute(
            &profile(Sex::Male, 180.0, ActivityLevel::Moderate),
            70.0,
            30,
            Some(&goal(GoalDirection::Gain, Some(80.0), 0.5)),
        );
        assert_eq!(t.delta, 500.0);
        // Gain protein anchors to the 80 kg goal weight: 80 × 1.6.
        assert_eq!(t.protein_g, 128.0);
    }

    #[test]
    fn lose_rate_is_capped_at_1_percent_of_body_weight() {
        // 50 kg, 1.0 kg/week requested → capped at 0.5 kg/week → −550.
        let t = compute(
            &profile(Sex::Male, 175.0, ActivityLevel::VeryActive),
            50.0,
            30,
            Some(&goal(GoalDirection::Lose, Some(45.0), 1.0)),
        );
        assert_eq!(t.delta, -550.0);
    }

    #[test]
    fn goal_reached_is_treated_as_maintain() {
        let reached_lose = compute(
            &profile(Sex::Male, 180.0, ActivityLevel::Moderate),
            74.0,
            30,
            Some(&goal(GoalDirection::Lose, Some(75.0), 0.5)),
        );
        assert_eq!(reached_lose.delta, 0.0);
        // Maintain protein factor 1.2, anchored to the 75 kg target.
        assert_eq!(reached_lose.protein_g, 90.0);

        let reached_gain = compute(
            &profile(Sex::Male, 180.0, ActivityLevel::Moderate),
            81.0,
            30,
            Some(&goal(GoalDirection::Gain, Some(80.0), 0.5)),
        );
        assert_eq!(reached_gain.delta, 0.0);
    }

    #[test]
    fn female_1200_floor_is_applied() {
        // 45 kg, 150 cm, 30 y, sedentary: BMR 1076.5, TDEE 1291.8, delta −322.95
        // → 968.85, below the 1200 floor.
        let t = compute(
            &profile(Sex::Female, 150.0, ActivityLevel::Sedentary),
            45.0,
            30,
            Some(&goal(GoalDirection::Lose, Some(42.0), 0.5)),
        );
        assert_eq!(t.kcal, 1200.0);
        assert!(t.floor_applied);
    }

    #[test]
    fn heavy_user_with_goal_weight_keeps_positive_carbs() {
        // The B1 case: female, 150 kg, 155 cm, 60 y, sedentary, Lose to 65 kg.
        // kcal floored to 2010 (BMR 2007.75). Old formula: protein 300 g + fat 90 g = 2010
        // kcal → carbs 0. Now protein = max(65 × 1.8, 0.8 × 150) = 120 g.
        let t = compute(
            &profile(Sex::Female, 155.0, ActivityLevel::Sedentary),
            150.0,
            60,
            Some(&goal(GoalDirection::Lose, Some(65.0), 0.5)),
        );
        assert_eq!(t.kcal, 2010.0);
        assert_eq!(t.protein_g, 120.0);
        assert_eq!(t.fat_g, 90.0);
        assert_eq!(t.carbs_g, 180.0);
    }

    #[test]
    fn infeasible_floors_raise_kcal_and_set_floor_applied() {
        // Same person with no target: protein anchors to 150 kg → 270 g. 270·4 + 90·9 = 1890
        // of 2010 kcal leaves 30 g carbs; fat is already at its floor, so kcal is raised to
        // 1080 + 810 + 400 = 2290.
        let t = compute(
            &profile(Sex::Female, 155.0, ActivityLevel::Sedentary),
            150.0,
            60,
            Some(&goal(GoalDirection::Lose, None, 0.5)),
        );
        assert_eq!(t.kcal, 2290.0);
        assert_eq!(t.protein_g, 270.0);
        assert_eq!(t.fat_g, 90.0);
        assert_eq!(t.carbs_g, 100.0);
        assert!(t.floor_applied);
    }

    #[test]
    fn low_carbs_are_restored_by_trading_fat_when_fat_is_above_its_floor() {
        // Female, 50 kg, 150 cm, 60 y, sedentary, Gain to 150 kg (anchor 150):
        // BMR 976.5, TDEE 1171.8, delta +500 → kcal 1680. Protein 150 × 1.6 = 240 (960 kcal);
        // fat max(0.27·1680/9 = 50.4, 30) = 50.4 → carbs 66.6 < 100.
        // Trade: fat = max(30, (1680 − 960 − 400)/9 = 35.56) → carbs 100, kcal unchanged.
        let t = compute(
            &profile(Sex::Female, 150.0, ActivityLevel::Sedentary),
            50.0,
            60,
            Some(&goal(GoalDirection::Gain, Some(150.0), 0.5)),
        );
        assert_eq!(t.kcal, 1680.0);
        assert_eq!(t.fat_g, 35.6);
        assert_eq!(t.carbs_g, 100.0);
        assert!(!t.floor_applied);
    }

    #[test]
    fn macros_add_up_to_kcal() {
        for (w, goal) in [
            (80.0, goal(GoalDirection::Lose, Some(75.0), 0.5)),
            (150.0, goal(GoalDirection::Lose, None, 0.5)),
            (60.0, goal(GoalDirection::Gain, Some(65.0), 0.25)),
        ] {
            let t = compute(
                &profile(Sex::Female, 160.0, ActivityLevel::Light),
                w,
                40,
                Some(&goal),
            );
            let energy = 4.0 * t.protein_g + 9.0 * t.fat_g + 4.0 * t.carbs_g;
            assert!((energy - t.kcal).abs() < 1.0, "{t:?}");
        }
    }

    #[test]
    fn no_goal_is_maintain_on_current_weight() {
        let t = compute(
            &profile(Sex::Male, 180.0, ActivityLevel::Moderate),
            80.0,
            30,
            None,
        );
        assert_eq!(t.delta, 0.0);
        assert_eq!(t.protein_g, 96.0);
    }

    #[test]
    fn round_up_to_10_never_rounds_down() {
        assert_eq!(round_up_to_10(2007.75), 2010.0);
        assert_eq!(round_up_to_10(2010.0), 2010.0);
        assert_eq!(round_up_to_10(2010.0000001), 2020.0);
    }
}
