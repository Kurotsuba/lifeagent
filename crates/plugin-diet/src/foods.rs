//! Name normalization and the per-100 g nutrition sanity check, per `docs/PHASE1_SPEC.md` §8.2,
//! §8.3, extended by `docs/NUTRITION_SOURCES.md` §4. Row-level storage now lives in
//! `sources.rs` (`TableSource`, `SourceChain`), which is the pluggable-source chain this module
//! feeds `normalize`/`check_nutrition` into.

use crate::model::Macros;

/// Diet plugin migration 1 (`docs/PHASE1_SPEC.md` §5). Kept verbatim for installs that still
/// need to apply it before migration 2 (`sources::MIGRATION_2`) rebuilds it.
pub const MIGRATION_1: &str = "CREATE TABLE diet_foods (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  name_key      TEXT NOT NULL UNIQUE,
  aliases_json  TEXT NOT NULL DEFAULT '[]',
  kcal          REAL NOT NULL,
  protein_g     REAL NOT NULL,
  carbs_g       REAL NOT NULL,
  fat_g         REAL NOT NULL,
  fiber_g       REAL NOT NULL DEFAULT 0,
  source        TEXT NOT NULL,
  use_count     INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
)";

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
pub fn validate_per_100g(n: &Macros) -> Result<(), String> {
    let values = [n.kcal, n.protein_g, n.carbs_g, n.fat_g, n.fiber_g];
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("all values must be numbers ≥ 0".into());
    }
    if n.fiber_g > n.carbs_g {
        return Err(format!(
            "fiber_g ({}) must not exceed carbs_g ({}): carbs_g is total carbohydrate, fiber included",
            n.fiber_g, n.carbs_g
        ));
    }
    let sum = n.protein_g + n.carbs_g + n.fat_g;
    if sum > 100.0 {
        return Err(format!(
            "protein_g + carbs_g + fat_g must be ≤ 100 per 100 g, got {sum}"
        ));
    }
    if n.kcal > 900.0 {
        return Err(format!("kcal must be ≤ 900 per 100 g, got {}", n.kcal));
    }
    Ok(())
}

/// The full §8.3 check for estimated values: the validity rules, then `kcal` replaced by the
/// macro-derived energy when the two disagree by more than `max(20, 15%)`.
pub fn check_nutrition(n: &Macros) -> Result<Macros, String> {
    validate_per_100g(n)?;
    let e = 4.0 * n.protein_g + 4.0 * (n.carbs_g - n.fiber_g) + 9.0 * n.fat_g + 2.0 * n.fiber_g;
    let mut checked = *n;
    if (n.kcal - e).abs() > f64::max(20.0, 0.15 * e) {
        tracing::debug!(
            given = n.kcal,
            computed = e,
            "replacing kcal with macro-derived energy"
        );
        checked.kcal = e;
    }
    Ok(checked)
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
