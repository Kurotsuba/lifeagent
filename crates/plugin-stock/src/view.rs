//! What the Stock tab shows, in one read.

use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::Serialize;

use crate::model::{name_key, StockItem, StockUse, Unit};
use crate::plan::{StockPlanSection, SECTION};
use crate::settings::{self, StockSettings};
use crate::store;

/// Recent meal uses shown under the stock list.
const RECENT_USES: u32 = 20;

#[derive(Debug, Serialize)]
pub struct StockView {
    /// Today's logical date, for the UI to mark items past their best-before.
    pub logical_date: chrono::NaiveDate,
    pub items: Vec<StockItem>,
    pub recent_uses: Vec<StockUse>,
    pub settings: StockSettings,
    /// From the current plan for tomorrow, or else today; `None` without such a plan or when
    /// it has no stock section.
    pub shopping: Option<Shopping>,
}

#[derive(Debug, Serialize)]
pub struct Shopping {
    pub plan_date: chrono::NaiveDate,
    pub items: Vec<ShoppingItem>,
}

#[derive(Debug, Serialize)]
pub struct ShoppingItem {
    pub name: String,
    pub amount: f64,
    pub unit: Unit,
    /// How much of it is in stock now, so items bought since the plan was made show as such.
    pub in_stock: f64,
}

pub async fn stock_view(ctx: &PluginContext) -> Result<StockView, PluginError> {
    let items = store::list(ctx).await?;
    let today = ctx.today().await?;
    let mut shopping = None;
    for date in [today.succ_opt(), Some(today)].into_iter().flatten() {
        let Some(plan) = ctx.plans.current(date).await? else {
            continue;
        };
        let Some(section) = plan.content.sections.get(SECTION) else {
            continue;
        };
        let section: StockPlanSection = serde_json::from_value(section.clone())
            .map_err(|e| PluginError::Other(format!("stored stock section: {e}")))?;
        shopping = Some(Shopping {
            plan_date: date,
            items: section
                .shopping_list
                .into_iter()
                .map(|line| ShoppingItem {
                    in_stock: in_stock(&items, &line.name, line.unit),
                    name: line.name,
                    amount: line.amount,
                    unit: line.unit,
                })
                .collect(),
        });
        break;
    }
    Ok(StockView {
        logical_date: today,
        recent_uses: store::recent_uses(ctx, RECENT_USES).await?,
        settings: settings::get(ctx).await?,
        shopping,
        items,
    })
}

fn in_stock(items: &[StockItem], name: &str, unit: Unit) -> f64 {
    let key = name_key(name);
    items
        .iter()
        .filter(|i| i.unit == unit && name_key(&i.name) == key)
        .map(|i| i.quantity)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewStock;
    use crate::testutil::env;
    use lifeagent_core::plan::{PlanContent, PlanTrigger};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn shopping_list_comes_from_tomorrows_plan_with_current_stock() {
        let env = env().await;
        let view = stock_view(&env.ctx).await.unwrap();
        assert!(view.shopping.is_none());
        assert_eq!(view.settings, StockSettings::default());

        let tomorrow = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let content = PlanContent {
            summary: "s".into(),
            sections: BTreeMap::from([(
                SECTION.to_string(),
                json!({"uses": [], "shopping_list": [
                    {"name": "西兰花", "amount": 300.0, "unit": "g"},
                    {"name": "鸡蛋", "amount": 4.0, "unit": "piece"}
                ]}),
            )]),
            disclaimer: None,
        };
        env.ctx
            .plans
            .insert(tomorrow, PlanTrigger::Manual, None, &content, &[])
            .await
            .unwrap();
        store::add(
            &env.ctx,
            &NewStock {
                name: "鸡蛋".into(),
                quantity: 6.0,
                unit: Unit::Piece,
                expires_on: None,
            },
        )
        .await
        .unwrap();

        let shopping = stock_view(&env.ctx).await.unwrap().shopping.unwrap();
        assert_eq!(shopping.plan_date, tomorrow);
        assert_eq!(shopping.items[0].in_stock, 0.0);
        assert_eq!(shopping.items[1].in_stock, 6.0);
    }
}
