//! What the user did with each line of a plan's shopping list: bought it ("done", which also
//! puts the line's amount into stock) or dropped it ("cancelled"). Lines start open. A mark
//! belongs to one plan version, so a regenerated plan starts with a fresh list.

use chrono::NaiveDate;
use lifeagent_core::plugin::{PluginContext, PluginError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::model::{name_key, Unit};
use crate::plan::{PlanLine, StockPlanSection, SECTION};
use crate::store::{self, NewStock};

pub const MIGRATION_2: &str = "
CREATE TABLE stock_shopping_marks (
  plan_date     TEXT NOT NULL,
  plan_version  BIGINT NOT NULL,
  name_key      TEXT NOT NULL,
  unit          TEXT NOT NULL,
  status        TEXT NOT NULL,
  marked_at     TEXT NOT NULL,
  PRIMARY KEY (plan_date, plan_version, name_key, unit)
);
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShoppingStatus {
    Open,
    Done,
    Cancelled,
}

impl ShoppingStatus {
    fn as_str(&self) -> &'static str {
        match self {
            ShoppingStatus::Open => "open",
            ShoppingStatus::Done => "done",
            ShoppingStatus::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(ShoppingStatus::Open),
            "done" => Some(ShoppingStatus::Done),
            "cancelled" => Some(ShoppingStatus::Cancelled),
            _ => None,
        }
    }
}

/// One line of one plan version's shopping list, as the UI names it.
#[derive(Debug, Clone, Deserialize)]
pub struct ShoppingLineRef {
    pub plan_date: NaiveDate,
    pub plan_version: u32,
    pub name: String,
    pub unit: Unit,
}

/// The marks on one plan version's list, by (name key, unit). Lines without one are open.
pub async fn marks(
    ctx: &PluginContext,
    plan_date: NaiveDate,
    plan_version: u32,
) -> Result<HashMap<(String, Unit), ShoppingStatus>, PluginError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT name_key, unit, status FROM stock_shopping_marks
         WHERE plan_date = $1 AND plan_version = $2",
    )
    .bind(plan_date.to_string())
    .bind(i64::from(plan_version))
    .fetch_all(&ctx.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(key, unit, status)| {
            Some(((key, Unit::parse(&unit)?), ShoppingStatus::parse(&status)?))
        })
        .collect())
}

/// Sets a line's status. `Done` adds the line's amount (from the stored plan, never from the
/// caller) to stock, once: marking a line done again changes nothing. Reopening or cancelling
/// a done line leaves the stock as it is. A line that isn't on that plan version's list (it
/// was regenerated since) is `NotFound`.
pub async fn set_status(
    ctx: &PluginContext,
    line: &ShoppingLineRef,
    status: ShoppingStatus,
) -> Result<(), PluginError> {
    let planned = find_line(ctx, line).await?;
    let key = name_key(&planned.name);
    let date = line.plan_date.to_string();
    let version = i64::from(line.plan_version);

    if status == ShoppingStatus::Open {
        sqlx::query(
            "DELETE FROM stock_shopping_marks
             WHERE plan_date = $1 AND plan_version = $2 AND name_key = $3 AND unit = $4",
        )
        .bind(&date)
        .bind(version)
        .bind(&key)
        .bind(planned.unit.as_str())
        .execute(&ctx.db)
        .await?;
        return Ok(());
    }

    // Claims the change atomically: no row back means the line already had this status, so
    // a double click can't add the amount to stock twice.
    let changed: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO stock_shopping_marks
            (plan_date, plan_version, name_key, unit, status, marked_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (plan_date, plan_version, name_key, unit) DO UPDATE SET
            status = EXCLUDED.status,
            marked_at = EXCLUDED.marked_at
         WHERE stock_shopping_marks.status <> EXCLUDED.status
         RETURNING 1",
    )
    .bind(&date)
    .bind(version)
    .bind(&key)
    .bind(planned.unit.as_str())
    .bind(status.as_str())
    .bind(ctx.clock.now().to_rfc3339())
    .fetch_optional(&ctx.db)
    .await?;

    if changed.is_some() && status == ShoppingStatus::Done {
        let added = store::add(
            ctx,
            &NewStock {
                name: planned.name.clone(),
                quantity: planned.amount,
                unit: planned.unit,
                expires_on: None,
            },
        )
        .await;
        if let Err(e) = added {
            // Undo the claim so the line can be marked done again.
            sqlx::query(
                "DELETE FROM stock_shopping_marks
                 WHERE plan_date = $1 AND plan_version = $2 AND name_key = $3 AND unit = $4",
            )
            .bind(&date)
            .bind(version)
            .bind(&key)
            .bind(planned.unit.as_str())
            .execute(&ctx.db)
            .await?;
            return Err(e);
        }
    }
    Ok(())
}

async fn find_line(ctx: &PluginContext, line: &ShoppingLineRef) -> Result<PlanLine, PluginError> {
    let not_found = || {
        PluginError::NotFound(format!(
            "shopping line {:?} ({}) on plan {} v{}",
            line.name,
            line.unit.as_str(),
            line.plan_date,
            line.plan_version
        ))
    };
    let plan = ctx
        .plans
        .current(line.plan_date)
        .await?
        .filter(|p| p.version == line.plan_version)
        .ok_or_else(not_found)?;
    let Some(section) = plan.content.sections.get(SECTION) else {
        return Err(not_found());
    };
    let section: StockPlanSection = serde_json::from_value(section.clone())
        .map_err(|e| PluginError::Other(format!("stored stock section: {e}")))?;
    let key = name_key(&line.name);
    section
        .shopping_list
        .into_iter()
        .find(|l| l.unit == line.unit && name_key(&l.name) == key)
        .ok_or_else(not_found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env;
    use crate::view::stock_view;
    use lifeagent_core::plan::{PlanContent, PlanTrigger};
    use serde_json::json;
    use std::collections::BTreeMap;

    async fn plan_with_list(ctx: &PluginContext, date: NaiveDate) -> u32 {
        let content = PlanContent {
            summary: "s".into(),
            sections: BTreeMap::from([(
                SECTION.to_string(),
                json!({"uses": [], "shopping_list": [
                    {"name": "西兰花", "amount": 300.0, "unit": "g"},
                    {"name": "Beer", "amount": 2.0, "unit": "piece"}
                ]}),
            )]),
            disclaimer: None,
        };
        ctx.plans
            .insert(date, PlanTrigger::Manual, None, &content, &[])
            .await
            .unwrap()
            .version
    }

    fn line(date: NaiveDate, version: u32, name: &str, unit: Unit) -> ShoppingLineRef {
        ShoppingLineRef {
            plan_date: date,
            plan_version: version,
            name: name.into(),
            unit,
        }
    }

    fn statuses(view: &crate::view::StockView) -> Vec<ShoppingStatus> {
        view.shopping
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|i| i.status)
            .collect()
    }

    #[tokio::test]
    async fn done_adds_the_planned_amount_to_stock_once() {
        let env = env().await;
        let date = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let v = plan_with_list(&env.ctx, date).await;
        let broccoli = line(date, v, "西兰花", Unit::G);

        set_status(&env.ctx, &broccoli, ShoppingStatus::Done)
            .await
            .unwrap();
        set_status(&env.ctx, &broccoli, ShoppingStatus::Done)
            .await
            .unwrap();

        let view = stock_view(&env.ctx).await.unwrap();
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].quantity, 300.0);
        assert_eq!(
            statuses(&view),
            [ShoppingStatus::Done, ShoppingStatus::Open]
        );
    }

    #[tokio::test]
    async fn cancel_and_reopen_leave_stock_alone() {
        let env = env().await;
        let date = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let v = plan_with_list(&env.ctx, date).await;
        // The name matches case-insensitively, like stock items.
        let beer = line(date, v, "beer", Unit::Piece);

        set_status(&env.ctx, &beer, ShoppingStatus::Cancelled)
            .await
            .unwrap();
        let view = stock_view(&env.ctx).await.unwrap();
        assert!(view.items.is_empty());
        assert_eq!(
            statuses(&view),
            [ShoppingStatus::Open, ShoppingStatus::Cancelled]
        );

        set_status(&env.ctx, &beer, ShoppingStatus::Open)
            .await
            .unwrap();
        let view = stock_view(&env.ctx).await.unwrap();
        assert_eq!(
            statuses(&view),
            [ShoppingStatus::Open, ShoppingStatus::Open]
        );
    }

    #[tokio::test]
    async fn marks_belong_to_one_plan_version() {
        let env = env().await;
        let date = env.ctx.today().await.unwrap().succ_opt().unwrap();
        let v1 = plan_with_list(&env.ctx, date).await;
        set_status(
            &env.ctx,
            &line(date, v1, "Beer", Unit::Piece),
            ShoppingStatus::Cancelled,
        )
        .await
        .unwrap();

        let v2 = plan_with_list(&env.ctx, date).await;
        let view = stock_view(&env.ctx).await.unwrap();
        assert_eq!(view.shopping.as_ref().unwrap().plan_version, v2);
        assert_eq!(
            statuses(&view),
            [ShoppingStatus::Open, ShoppingStatus::Open]
        );

        // The old version, and lines not on the list, are gone.
        let stale = line(date, v1, "Beer", Unit::Piece);
        let stale = set_status(&env.ctx, &stale, ShoppingStatus::Done).await;
        assert!(matches!(stale, Err(PluginError::NotFound(_))));
        let missing = line(date, v2, "Beer", Unit::G);
        let missing = set_status(&env.ctx, &missing, ShoppingStatus::Done).await;
        assert!(matches!(missing, Err(PluginError::NotFound(_))));
    }
}
