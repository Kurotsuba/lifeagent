//! The `stock` plan section: the planner sees what's at home, builds the diet section's meals
//! from it, and lists what it took from stock plus what is left to buy. Rust checks every stock
//! use against what's actually there (a use of something not in stock, or more than is left,
//! is a violation the planner gets one retry to fix) and warns about soon-expiring food the
//! plan leaves unused.

use chrono::NaiveDate;
use lifeagent_core::plugin::{
    PlanDraft, PlanInput, PlanNote, PlanSection, PluginContext, PluginError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::{name_key, round_amount, StockItem, Unit};
use crate::{settings, store};

pub const SECTION: &str = "stock";
/// The multimember plugin's computed section, read only for whether it is there.
pub const MULTIMEMBER_SECTION: &str = "multimember";

/// An item expiring within this many days of the plan date, left unused, gets a warning.
const EXPIRY_WARNING_DAYS: i64 = 1;

pub fn output_schema() -> Value {
    let line = json!({
        "type": "object",
        "required": ["name", "amount", "unit"],
        "properties": {
            "name": { "type": "string" },
            "amount": { "type": "number" },
            "unit": { "enum": ["g", "ml", "piece"] }
        }
    });
    json!({
        "type": "object",
        "required": ["uses", "shopping_list"],
        "properties": {
            "uses": { "type": "array", "items": line },
            "shopping_list": { "type": "array", "items": line }
        }
    })
}

/// `None` when planning from stock is switched off, or when there's no profile and weight yet:
/// without those the diet section can't be planned, and a shopping list for no meals is
/// meaningless (and would stop core from reporting "nothing to plan").
pub async fn plan_input(
    ctx: &PluginContext,
    date: NaiveDate,
    prior: &PlanDraft,
) -> Result<Option<PlanInput>, PluginError> {
    if !settings::get(ctx).await?.plan_from_stock
        || ctx.profile.get().await?.is_none()
        || ctx.events.latest("body.weight_logged").await?.is_none()
    {
        return Ok(None);
    }
    let stock = store::list(ctx).await?;
    let stock_lines = if stock.is_empty() {
        "(nothing in stock)".to_string()
    } else {
        stock
            .iter()
            .map(|s| {
                let expiry = s
                    .expires_on
                    .map(|d| format!(", best before {d}"))
                    .unwrap_or_default();
                format!("- {}: {} {}{expiry}", s.name, s.quantity, s.unit.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut brief = format!(
        "Food the user already has at home on {date} (name: amount left, unit):\n{stock_lines}\n\
         - Build the diet section's meals from this stock first, soonest best-before first, as \
           long as the meals stay balanced and meet the diet targets. When a meal uses a stocked \
           item, name it in the diet items exactly as written here.\n\
         - \"uses\": each stocked item the plan uses and how much, in that item's unit and name; \
           never more than is in stock.\n\
         - \"shopping_list\": only what the plan needs beyond the stock, with the amount to buy \
           in g, ml or piece. The user wants to buy as little as possible: prefer meals that need \
           few purchases, but never at the cost of a balanced diet."
    );
    if prior.contains_key(MULTIMEMBER_SECTION) {
        brief.push_str(
            "\n- Some meals are shared with other members: \"uses\" and              \"shopping_list\" must cover every portion of those meals (the diet brief gives the              portion count), not only the user's own.",
        );
    }
    Ok(Some(PlanInput {
        section: SECTION.into(),
        brief,
        targets: json!({}),
        output_schema: output_schema(),
    }))
}

#[derive(Debug, Deserialize)]
struct RawSection {
    #[serde(default)]
    uses: Vec<RawLine>,
    #[serde(default)]
    shopping_list: Vec<RawLine>,
}

#[derive(Debug, Deserialize)]
struct RawLine {
    name: String,
    amount: f64,
    unit: Unit,
}

/// One line of the finalized section. `expires_on` is filled in from stock for uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanLine {
    pub name: String,
    pub amount: f64,
    pub unit: Unit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_on: Option<NaiveDate>,
}

/// `StoredPlan.content.sections["stock"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockPlanSection {
    pub uses: Vec<PlanLine>,
    pub shopping_list: Vec<PlanLine>,
}

pub async fn finalize(
    ctx: &PluginContext,
    date: NaiveDate,
    raw: Value,
) -> Result<PlanSection, PluginError> {
    let raw: RawSection = match serde_json::from_value(raw) {
        Ok(r) => r,
        Err(e) => {
            return Ok(PlanSection {
                data: Value::Null,
                warnings: Vec::new(),
                violations: vec![PlanNote::new(
                    "stock_invalid_structure",
                    format!("The stock section has the wrong shape: {e}"),
                )
                .with("reason", e.to_string())],
            })
        }
    };
    let stock = store::list(ctx).await?;
    let (section, warnings, violations) = check(&stock, date, raw);
    Ok(PlanSection {
        data: serde_json::to_value(&section).map_err(|e| PluginError::Other(e.to_string()))?,
        warnings,
        violations,
    })
}

fn check(
    stock: &[StockItem],
    date: NaiveDate,
    raw: RawSection,
) -> (StockPlanSection, Vec<PlanNote>, Vec<PlanNote>) {
    let mut warnings = Vec::new();
    let mut violations = Vec::new();

    let mut uses: Vec<PlanLine> = Vec::new();
    for line in merge(raw.uses) {
        let Some(item) = stock
            .iter()
            .find(|s| name_key(&s.name) == name_key(&line.name))
        else {
            violations.push(
                PlanNote::new(
                    "stock_not_in_stock",
                    format!("\"{}\" is not in stock, so it cannot be in uses", line.name),
                )
                .with("name", &line.name),
            );
            continue;
        };
        if item.unit != line.unit {
            violations.push(
                PlanNote::new(
                    "stock_wrong_unit",
                    format!(
                        "\"{}\" is stocked in {}, not {}",
                        item.name,
                        item.unit.as_str(),
                        line.unit.as_str()
                    ),
                )
                .with("name", &item.name)
                .with("unit", item.unit)
                .with("planned_unit", line.unit),
            );
            continue;
        }
        if line.amount > item.quantity + 1e-6 {
            violations.push(
                PlanNote::new(
                    "stock_over_quantity",
                    format!(
                        "\"{}\" uses {} {} but only {} {} is in stock",
                        item.name,
                        line.amount,
                        item.unit.as_str(),
                        item.quantity,
                        item.unit.as_str()
                    ),
                )
                .with("name", &item.name)
                .with("amount", line.amount)
                .with("quantity", item.quantity)
                .with("unit", item.unit),
            );
        }
        uses.push(PlanLine {
            name: item.name.clone(),
            amount: line.amount.min(item.quantity),
            unit: item.unit,
            expires_on: item.expires_on,
        });
    }

    for item in stock {
        let Some(expires) = item.expires_on else {
            continue;
        };
        if (expires - date).num_days() <= EXPIRY_WARNING_DAYS
            && !uses
                .iter()
                .any(|u| name_key(&u.name) == name_key(&item.name))
        {
            warnings.push(
                PlanNote::new(
                    "stock_expiring_unused",
                    format!(
                        "\"{}\" in stock expires on {expires} and the plan doesn't use it",
                        item.name
                    ),
                )
                .with("name", &item.name)
                .with("expires_on", expires),
            );
        }
    }

    let shopping_list = merge(raw.shopping_list)
        .into_iter()
        .map(|l| PlanLine {
            name: l.name,
            amount: l.amount,
            unit: l.unit,
            expires_on: None,
        })
        .collect();

    (
        StockPlanSection {
            uses,
            shopping_list,
        },
        warnings,
        violations,
    )
}

/// Trims names, drops empty names and non-positive amounts, and sums lines for the same item
/// and unit, keeping first-seen order.
fn merge(lines: Vec<RawLine>) -> Vec<RawLine> {
    let mut out: Vec<RawLine> = Vec::new();
    for line in lines {
        let name = line.name.trim();
        if name.is_empty() || !line.amount.is_finite() || line.amount <= 0.0 {
            continue;
        }
        match out
            .iter_mut()
            .find(|o| o.unit == line.unit && name_key(&o.name) == name_key(name))
        {
            Some(o) => o.amount = round_amount(o.amount + line.amount),
            None => out.push(RawLine {
                name: name.to_string(),
                amount: round_amount(line.amount),
                unit: line.unit,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::StockSettings;
    use crate::store::NewStock;
    use crate::testutil::{env, seed_person};

    const DAY: NaiveDate = match NaiveDate::from_ymd_opt(2026, 10, 4) {
        Some(d) => d,
        None => panic!("valid date"),
    };

    fn item(name: &str, quantity: f64, unit: Unit, expires_on: Option<&str>) -> StockItem {
        StockItem {
            id: name.into(),
            name: name.into(),
            quantity,
            unit,
            expires_on: expires_on.map(|d| d.parse().unwrap()),
            updated_at: String::new(),
        }
    }

    fn raw(value: Value) -> RawSection {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn valid_uses_pass_and_pick_up_the_stock_name_and_expiry() {
        let stock = [item("鸡蛋", 6.0, Unit::Piece, Some("2026-10-08"))];
        let (section, warnings, violations) = check(
            &stock,
            DAY,
            raw(json!({
                "uses": [{"name": " 鸡蛋", "amount": 2, "unit": "piece"}, {"name": "鸡蛋", "amount": 1, "unit": "piece"}],
                "shopping_list": [{"name": "西兰花", "amount": 300, "unit": "g"}, {"name": "x", "amount": 0, "unit": "g"}]
            })),
        );
        assert!(violations.is_empty(), "{violations:?}");
        assert!(warnings.is_empty());
        assert_eq!(
            section.uses,
            vec![PlanLine {
                name: "鸡蛋".into(),
                amount: 3.0,
                unit: Unit::Piece,
                expires_on: "2026-10-08".parse().ok(),
            }]
        );
        assert_eq!(section.shopping_list.len(), 1);
    }

    #[test]
    fn unknown_items_wrong_units_and_overuse_are_violations() {
        let stock = [
            item("米", 500.0, Unit::G, None),
            item("牛奶", 200.0, Unit::Ml, None),
        ];
        let (section, _, violations) = check(
            &stock,
            DAY,
            raw(json!({
                "uses": [
                    {"name": "面包", "amount": 1, "unit": "piece"},
                    {"name": "米", "amount": 1, "unit": "piece"},
                    {"name": "牛奶", "amount": 400, "unit": "ml"}
                ],
                "shopping_list": []
            })),
        );
        assert_eq!(violations.len(), 3, "{violations:?}");
        // Overuse is kept, capped, so the section still renders if the retry fails too.
        assert_eq!(section.uses.len(), 1);
        assert_eq!(section.uses[0].amount, 200.0);
    }

    #[test]
    fn warns_about_soon_expiring_stock_the_plan_leaves_unused() {
        let stock = [
            item("豆腐", 1.0, Unit::Piece, Some("2026-10-05")),
            item("酸奶", 2.0, Unit::Piece, Some("2026-10-04")),
            item("米", 500.0, Unit::G, Some("2027-01-01")),
        ];
        let (_, warnings, _) = check(
            &stock,
            DAY,
            raw(
                json!({"uses": [{"name": "酸奶", "amount": 1, "unit": "piece"}], "shopping_list": []}),
            ),
        );
        assert_eq!(
            warnings,
            vec![PlanNote::new(
                "stock_expiring_unused",
                "\"豆腐\" in stock expires on 2026-10-05 and the plan doesn't use it"
            )
            .with("name", "豆腐")
            .with("expires_on", "2026-10-05")]
        );
    }

    #[tokio::test]
    async fn plan_input_needs_a_profile_and_weight_and_the_switch_on() {
        let env = env().await;
        assert!(plan_input(&env.ctx, DAY, &PlanDraft::new())
            .await
            .unwrap()
            .is_none());

        seed_person(&env.ctx).await;
        store::add(
            &env.ctx,
            &NewStock {
                name: "鸡蛋".into(),
                quantity: 6.0,
                unit: Unit::Piece,
                expires_on: "2026-10-08".parse().ok(),
            },
        )
        .await
        .unwrap();
        let input = plan_input(&env.ctx, DAY, &PlanDraft::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(input.section, "stock");
        assert!(input
            .brief
            .contains("- 鸡蛋: 6 piece, best before 2026-10-08"));
        assert!(!input.brief.contains("shared"));
        let shared = PlanDraft::from([(MULTIMEMBER_SECTION.to_string(), json!({"meals": []}))]);
        let input = plan_input(&env.ctx, DAY, &shared).await.unwrap().unwrap();
        assert!(input.brief.contains("cover every portion"));

        settings::set(
            &env.ctx,
            &StockSettings {
                auto_consume: true,
                plan_from_stock: false,
            },
            env.ctx.clock.now(),
        )
        .await
        .unwrap();
        assert!(plan_input(&env.ctx, DAY, &PlanDraft::new())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn finalize_reports_a_malformed_section_as_a_violation() {
        let env = env().await;
        let section = finalize(&env.ctx, DAY, json!({"uses": "lots"}))
            .await
            .unwrap();
        assert_eq!(section.violations.len(), 1);
    }
}
