//! Name normalization and the per-100 g nutrition sanity check, per `docs/PHASE1_SPEC.md` §8.2,
//! §8.3, extended by `docs/NUTRITION_SOURCES.md` §4. Row-level storage now lives in
//! `sources.rs` (`TableSource`, `SourceChain`), which is the pluggable-source chain this module
//! feeds `normalize`/`check_nutrition` into.

use serde::Serialize;

use crate::model::Macros;

/// The first nutrition validity rule a row broke (§8.3 per 100 g, §7.2 per serving). `Display`
/// is the English text the conversation model sees in a rejected tool call; the import preview
/// sends the variant itself, so the UI can word it in the user's language.
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum MacroViolation {
    #[error("all values must be numbers ≥ 0")]
    Negative,
    #[error(
        "fiber_g ({fiber_g}) must not exceed carbs_g ({carbs_g}): carbs_g is total carbohydrate, fiber included"
    )]
    FiberExceedsCarbs { fiber_g: f64, carbs_g: f64 },
    #[error("protein_g + carbs_g + fat_g must be ≤ 100 per 100 g, got {sum}")]
    #[serde(rename = "macros_exceed_100g")]
    MacrosExceed100g { sum: f64 },
    #[error("kcal must be ≤ 900 per 100 g, got {got}")]
    #[serde(rename = "kcal_per_100g_too_high")]
    KcalPer100gTooHigh { got: f64 },
    #[error("kcal must be ≤ 5000 per serving, got {got}")]
    KcalPerServingTooHigh { got: f64 },
    #[error("serving_g must be > 0")]
    ServingNotPositive,
    #[error("protein_g + carbs_g + fat_g ({sum}) must not exceed serving_g ({serving_g})")]
    MacrosExceedServing { sum: f64, serving_g: f64 },
    #[error("kcal ({kcal}) must not exceed 9 × serving_g ({serving_g})")]
    KcalExceedsServing { kcal: f64, serving_g: f64 },
}

/// Punctuation stripped from the end of a name. Closing brackets are deliberately not in the
/// list: names like "牛肉(熟)" end in one.
const TRAILING_PUNCTUATION: &[char] = &[
    '.', ',', '!', '?', ';', ':', '。', '，', '！', '？', '；', '：', '、', '…', '~', '～',
];

/// Lookup key for a food name (§8.2): trimmed, ASCII-lowercased, whitespace collapsed,
/// trailing punctuation removed.
pub fn normalize(name: &str) -> String {
    let collapsed = name
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    collapsed
        .trim_end_matches(TRAILING_PUNCTUATION)
        .trim_end()
        .to_string()
}

/// The hard validity rules of §8.3, without the kcal correction. `Err` describes the first
/// rule that failed.
pub fn validate_per_100g(n: &Macros) -> Result<(), MacroViolation> {
    let values = [n.kcal, n.protein_g, n.carbs_g, n.fat_g, n.fiber_g];
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(MacroViolation::Negative);
    }
    if n.fiber_g > n.carbs_g {
        return Err(MacroViolation::FiberExceedsCarbs {
            fiber_g: n.fiber_g,
            carbs_g: n.carbs_g,
        });
    }
    let sum = n.protein_g + n.carbs_g + n.fat_g;
    if sum > 100.0 {
        return Err(MacroViolation::MacrosExceed100g { sum });
    }
    if n.kcal > 900.0 {
        return Err(MacroViolation::KcalPer100gTooHigh { got: n.kcal });
    }
    Ok(())
}

/// The full §8.3 check for estimated values: the validity rules, then `kcal` replaced by the
/// macro-derived energy when the two disagree by more than `max(20, 15%)`.
pub fn check_nutrition(n: &Macros) -> Result<Macros, String> {
    validate_per_100g(n).map_err(|e| e.to_string())?;
    let e = macro_energy(n);
    let mut checked = *n;
    if kcal_disagrees_with_macros(n) {
        tracing::debug!(
            given = n.kcal,
            computed = e,
            "replacing kcal with macro-derived energy"
        );
        checked.kcal = e;
    }
    Ok(checked)
}

/// Energy from the macros: 4 kcal/g protein and digestible carbs, 9 kcal/g fat, 2 kcal/g fiber.
pub fn macro_energy(n: &Macros) -> f64 {
    4.0 * n.protein_g + 4.0 * (n.carbs_g - n.fiber_g) + 9.0 * n.fat_g + 2.0 * n.fiber_g
}

/// Whether `kcal` and the macro-derived energy disagree by more than `max(20, 15%)` (§8.3).
pub fn kcal_disagrees_with_macros(n: &Macros) -> bool {
    let e = macro_energy(n);
    (n.kcal - e).abs() > f64::max(20.0, 0.15 * e)
}

/// Character bigrams of `s`; empty for strings under 2 characters.
pub fn bigrams(s: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 {
        return Vec::new();
    }
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Dice coefficient over character bigrams: `2 * |A ∩ B| / (|A| + |B|)`, matched as a multiset.
pub fn dice_score(a: &str, b: &str) -> f64 {
    let (ba, bb) = (bigrams(a), bigrams(b));
    if ba.is_empty() || bb.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    let mut pool = bb.clone();
    let mut matches = 0usize;
    for bg in &ba {
        if let Some(pos) = pool.iter().position(|x| x == bg) {
            pool.remove(pos);
            matches += 1;
        }
    }
    (2 * matches) as f64 / (ba.len() + bb.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macros(kcal: f64, protein_g: f64, carbs_g: f64, fat_g: f64, fiber_g: f64) -> Macros {
        Macros {
            kcal,
            protein_g,
            carbs_g,
            fat_g,
            fiber_g,
        }
    }

    fn rice() -> Macros {
        macros(116.0, 2.6, 25.9, 0.3, 0.3)
    }

    #[test]
    fn normalize_trims_lowercases_collapses_and_strips_trailing_punctuation() {
        assert_eq!(normalize("  Greek   Yogurt. "), "greek yogurt");
        assert_eq!(normalize("熟米饭。"), "熟米饭");
        assert_eq!(normalize("卤蛋，，"), "卤蛋");
        // Closing brackets are part of the name, not punctuation to strip.
        assert_eq!(normalize("牛肉(熟)"), "牛肉(熟)");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn check_accepts_consistent_values_unchanged() {
        assert_eq!(check_nutrition(&rice()), Ok(rice()));
    }

    #[test]
    fn check_accepts_high_fiber_food_with_fiber_inside_carbs() {
        // Wheat bran: rejected under the old fiber-as-separate-addend rule (B2).
        let bran = macros(216.0, 15.5, 64.5, 4.3, 42.8);
        assert!(check_nutrition(&bran).is_ok());
    }

    #[test]
    fn check_rejects_fiber_above_carbs() {
        assert!(check_nutrition(&macros(50.0, 1.0, 5.0, 0.0, 10.0)).is_err());
    }

    #[test]
    fn check_rejects_negative_values_sum_over_100_and_kcal_over_900() {
        assert!(check_nutrition(&macros(100.0, -1.0, 10.0, 1.0, 0.0)).is_err());
        assert!(check_nutrition(&macros(500.0, 40.0, 40.0, 30.0, 0.0)).is_err());
        assert!(check_nutrition(&macros(950.0, 0.0, 0.0, 100.0, 0.0)).is_err());
    }

    #[test]
    fn check_replaces_kcal_with_macro_energy_when_far_off() {
        // e = 4·2.6 + 4·(25.9 − 0.3) + 9·0.3 + 2·0.3 = 116.1; 300 is far off.
        let checked = check_nutrition(&macros(300.0, 2.6, 25.9, 0.3, 0.3)).unwrap();
        assert!((checked.kcal - 116.1).abs() < 1e-9);
    }

    #[test]
    fn check_keeps_kcal_within_tolerance() {
        // 130 vs e = 116.1: within max(20, 17.4).
        let checked = check_nutrition(&macros(130.0, 2.6, 25.9, 0.3, 0.3)).unwrap();
        assert_eq!(checked.kcal, 130.0);
    }
}
