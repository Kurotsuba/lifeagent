//! Stock items, units and name keys.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// How an item is counted. The model converts what the user says ("1 L 牛奶", "一盒鸡蛋") into
/// one of these when it adds stock; amounts taken out are always in the item's own unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    G,
    Ml,
    Piece,
}

impl Unit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Unit::G => "g",
            Unit::Ml => "ml",
            Unit::Piece => "piece",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "g" => Some(Unit::G),
            "ml" => Some(Unit::Ml),
            "piece" => Some(Unit::Piece),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StockItem {
    pub id: String,
    pub name: String,
    pub quantity: f64,
    pub unit: Unit,
    pub expires_on: Option<NaiveDate>,
    pub updated_at: String,
}

/// One amount a logged meal took out of stock. `restored` once the meal was deleted and the
/// amount put back.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StockUse {
    pub event_id: String,
    pub logical_date: NaiveDate,
    pub item_name: String,
    pub unit: Unit,
    pub amount: f64,
    pub used_at: String,
}

/// Longest item name accepted, in characters.
pub const MAX_NAME_CHARS: usize = 60;
/// Largest quantity accepted for one item (100 kg / 100 L / 100000 pieces is already absurd
/// for home use; anything above is a unit mix-up).
pub const MAX_QUANTITY: f64 = 100_000.0;

/// The lookup key for an item name: whitespace collapsed, ASCII lowercased and full-width ASCII
/// folded to half-width, so "Ｍilk ", "milk" and "MILK" are the same item.
pub fn name_key(name: &str) -> String {
    let folded: String = name
        .chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            c => c,
        })
        .collect();
    folded
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Amounts are kept to one decimal: finer precision is noise for a pantry.
pub fn round_amount(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_key_folds_case_width_and_spaces() {
        assert_eq!(name_key("  Ｍilk  Tea "), "milk tea");
        assert_eq!(name_key("鸡蛋"), "鸡蛋");
        assert_eq!(name_key("ＥＧＧ"), "egg");
    }

    #[test]
    fn units_round_trip() {
        for u in [Unit::G, Unit::Ml, Unit::Piece] {
            assert_eq!(Unit::parse(u.as_str()), Some(u));
        }
        assert_eq!(Unit::parse("kg"), None);
    }

    #[test]
    fn round_amount_keeps_one_decimal() {
        assert_eq!(round_amount(1.26), 1.3);
        assert_eq!(round_amount(89.99), 90.0);
    }
}
