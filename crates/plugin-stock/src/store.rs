//! Storage for stock items and the amounts logged meals took out of them.

use chrono::NaiveDate;
use lifeagent_core::events::Event;
use lifeagent_core::plugin::{PluginContext, PluginError};
use uuid::Uuid;

use crate::model::{
    name_key, round_amount, StockItem, StockUse, Unit, MAX_NAME_CHARS, MAX_QUANTITY,
};

/// One item per (name, unit): adding more of an item already in stock tops it up. A row whose
/// quantity reaches 0 is kept (hidden from lists) so a deleted meal can put its amount back.
pub const MIGRATION_1: &str = "
CREATE TABLE stock_items (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL,
  unit        TEXT NOT NULL,
  quantity    DOUBLE PRECISION NOT NULL,
  expires_on  TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (name_key, unit)
);

CREATE TABLE stock_uses (
  id            TEXT PRIMARY KEY,
  event_id      TEXT NOT NULL,
  item_id       TEXT REFERENCES stock_items(id) ON DELETE SET NULL,
  item_name     TEXT NOT NULL,
  unit          TEXT NOT NULL,
  amount        DOUBLE PRECISION NOT NULL,
  logical_date  TEXT NOT NULL,
  created_at    TEXT NOT NULL,
  restored_at   TEXT
);

CREATE INDEX stock_uses_event ON stock_uses(event_id);
";

/// Stock the user adds, from chat or the Stock tab.
#[derive(Debug, Clone)]
pub struct NewStock {
    pub name: String,
    pub quantity: f64,
    pub unit: Unit,
    pub expires_on: Option<NaiveDate>,
}

type ItemRow = (String, String, f64, String, Option<String>, String);

/// The columns `ItemRow` reads, as a macro so queries stay `&'static str` (sqlx rejects SQL
/// built at runtime).
macro_rules! item_columns {
    () => {
        "id, name, quantity, unit, expires_on, updated_at"
    };
}

/// `MAX_QUANTITY` as SQL, for the same reason.
macro_rules! max_quantity {
    () => {
        "100000"
    };
}

fn row_to_item(
    (id, name, quantity, unit, expires_on, updated_at): ItemRow,
) -> Result<StockItem, PluginError> {
    Ok(StockItem {
        id,
        name,
        quantity,
        unit: Unit::parse(&unit)
            .ok_or_else(|| PluginError::Other(format!("unknown stock unit {unit:?}")))?,
        expires_on: expires_on
            .map(|d| {
                NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                    .map_err(|e| PluginError::Other(format!("bad expires_on {d:?}: {e}")))
            })
            .transpose()?,
        updated_at,
    })
}

fn check_name(name: &str) -> Result<String, PluginError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PluginError::InvalidArgs(
            "item name must not be empty".into(),
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(PluginError::InvalidArgs(format!(
            "item name must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_string())
}

fn check_quantity(quantity: f64, allow_zero: bool) -> Result<f64, PluginError> {
    let ok = quantity.is_finite()
        && quantity <= MAX_QUANTITY
        && (quantity > 0.0 || (allow_zero && quantity == 0.0));
    if !ok {
        return Err(PluginError::InvalidArgs(format!(
            "quantity must be {} {MAX_QUANTITY}, got {quantity}",
            if allow_zero {
                "between 0 and"
            } else {
                "above 0 and at most"
            }
        )));
    }
    Ok(round_amount(quantity))
}

/// Items with something left, soonest best-before first (undated last), then by name.
pub async fn list(ctx: &PluginContext) -> Result<Vec<StockItem>, PluginError> {
    let rows: Vec<ItemRow> = sqlx::query_as(concat!(
        "SELECT ",
        item_columns!(),
        " FROM stock_items WHERE quantity > 0
         ORDER BY expires_on ASC NULLS LAST, name ASC"
    ))
    .fetch_all(&ctx.db)
    .await?;
    rows.into_iter().map(row_to_item).collect()
}

/// Adds `new` to stock, topping up the item with the same name and unit if there is one. A
/// top-up keeps the earlier best-before date, so the plan uses the older food first; an item
/// that had run out takes the new date.
pub async fn add(ctx: &PluginContext, new: &NewStock) -> Result<StockItem, PluginError> {
    let name = check_name(&new.name)?;
    let quantity = check_quantity(new.quantity, false)?;
    let now = ctx.clock.now().to_rfc3339();
    let row: ItemRow = sqlx::query_as(concat!(
        "INSERT INTO stock_items
            (id, name, name_key, unit, quantity, expires_on, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
         ON CONFLICT (name_key, unit) DO UPDATE SET
            name = EXCLUDED.name,
            quantity = LEAST(stock_items.quantity + EXCLUDED.quantity, ",
        max_quantity!(),
        "),
            expires_on = CASE
                WHEN stock_items.quantity <= 0 OR stock_items.expires_on IS NULL
                    THEN EXCLUDED.expires_on
                WHEN EXCLUDED.expires_on IS NULL THEN stock_items.expires_on
                ELSE LEAST(stock_items.expires_on, EXCLUDED.expires_on)
            END,
            updated_at = EXCLUDED.updated_at
         RETURNING ",
        item_columns!()
    ))
    .bind(Uuid::now_v7().to_string())
    .bind(&name)
    .bind(name_key(&name))
    .bind(new.unit.as_str())
    .bind(quantity)
    .bind(new.expires_on.map(|d| d.to_string()))
    .bind(&now)
    .fetch_one(&ctx.db)
    .await?;
    row_to_item(row)
}

/// Items with `name`, including run-out ones, in any unit.
pub async fn find_by_name(ctx: &PluginContext, name: &str) -> Result<Vec<StockItem>, PluginError> {
    let rows: Vec<ItemRow> = sqlx::query_as(concat!(
        "SELECT ",
        item_columns!(),
        " FROM stock_items WHERE name_key = $1 ORDER BY unit"
    ))
    .bind(name_key(name))
    .fetch_all(&ctx.db)
    .await?;
    rows.into_iter().map(row_to_item).collect()
}

/// Sets how much of an item is left; 0 means used up or thrown away.
pub async fn set_quantity(
    ctx: &PluginContext,
    id: &str,
    quantity: f64,
) -> Result<StockItem, PluginError> {
    let quantity = check_quantity(quantity, true)?;
    let row: Option<ItemRow> = sqlx::query_as(concat!(
        "UPDATE stock_items SET quantity = $1, updated_at = $2 WHERE id = $3
         RETURNING ",
        item_columns!()
    ))
    .bind(quantity)
    .bind(ctx.clock.now().to_rfc3339())
    .bind(id)
    .fetch_optional(&ctx.db)
    .await?;
    row.map(row_to_item)
        .transpose()?
        .ok_or_else(|| PluginError::NotFound(format!("stock item {id}")))
}

pub async fn delete(ctx: &PluginContext, id: &str) -> Result<(), PluginError> {
    let result = sqlx::query("DELETE FROM stock_items WHERE id = $1")
        .bind(id)
        .execute(&ctx.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(PluginError::NotFound(format!("stock item {id}")));
    }
    Ok(())
}

/// Whether `event_id` already took anything out of stock (so a redelivered event is a no-op).
pub async fn has_uses(ctx: &PluginContext, event_id: &str) -> Result<bool, PluginError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT id FROM stock_uses WHERE event_id = $1 LIMIT 1")
            .bind(event_id)
            .fetch_optional(&ctx.db)
            .await?;
    Ok(row.is_some())
}

/// Takes each `(item, amount)` out of stock for `event`, in one transaction. Amounts are capped
/// at what's left, so stock never goes negative even if it changed since `item` was read.
pub async fn consume(
    ctx: &PluginContext,
    event: &Event,
    uses: &[(StockItem, f64)],
) -> Result<Vec<StockUse>, PluginError> {
    let now = ctx.clock.now().to_rfc3339();
    let mut tx = ctx.db.begin().await?;
    let mut recorded = Vec::with_capacity(uses.len());
    for (item, amount) in uses {
        let left: Option<(f64,)> =
            sqlx::query_as("SELECT quantity FROM stock_items WHERE id = $1 FOR UPDATE")
                .bind(&item.id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((left,)) = left else { continue };
        let amount = round_amount(amount.min(left));
        if amount <= 0.0 {
            continue;
        }
        sqlx::query("UPDATE stock_items SET quantity = $1, updated_at = $2 WHERE id = $3")
            .bind(round_amount(left - amount))
            .bind(&now)
            .bind(&item.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO stock_uses
                (id, event_id, item_id, item_name, unit, amount, logical_date, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(&event.id)
        .bind(&item.id)
        .bind(&item.name)
        .bind(item.unit.as_str())
        .bind(amount)
        .bind(event.logical_date.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        recorded.push(StockUse {
            event_id: event.id.clone(),
            logical_date: event.logical_date,
            item_name: item.name.clone(),
            unit: item.unit,
            amount,
            used_at: now.clone(),
        });
    }
    tx.commit().await?;
    Ok(recorded)
}

/// Puts back what `event_id` took out (its meal was deleted). Items the user has since removed
/// stay removed. Returns how many uses were restored.
pub async fn restore(ctx: &PluginContext, event_id: &str) -> Result<usize, PluginError> {
    let now = ctx.clock.now().to_rfc3339();
    let mut tx = ctx.db.begin().await?;
    let uses: Vec<(String, Option<String>, f64)> = sqlx::query_as(
        "SELECT id, item_id, amount FROM stock_uses
         WHERE event_id = $1 AND restored_at IS NULL FOR UPDATE",
    )
    .bind(event_id)
    .fetch_all(&mut *tx)
    .await?;
    for (use_id, item_id, amount) in &uses {
        if let Some(item_id) = item_id {
            sqlx::query(concat!(
                "UPDATE stock_items
                 SET quantity = LEAST(quantity + $1, ",
                max_quantity!(),
                "), updated_at = $2
                 WHERE id = $3"
            ))
            .bind(amount)
            .bind(&now)
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE stock_uses SET restored_at = $1 WHERE id = $2")
            .bind(&now)
            .bind(use_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(uses.len())
}

/// The latest amounts logged meals took out (not put back), newest first.
pub async fn recent_uses(ctx: &PluginContext, limit: u32) -> Result<Vec<StockUse>, PluginError> {
    let rows: Vec<(String, String, String, String, f64, String)> = sqlx::query_as(
        "SELECT event_id, logical_date, item_name, unit, amount, created_at FROM stock_uses
         WHERE restored_at IS NULL ORDER BY created_at DESC, id DESC LIMIT $1",
    )
    .bind(i64::from(limit))
    .fetch_all(&ctx.db)
    .await?;
    rows.into_iter()
        .map(|(event_id, date, item_name, unit, amount, used_at)| {
            Ok(StockUse {
                event_id,
                logical_date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .map_err(|e| PluginError::Other(format!("bad logical_date {date:?}: {e}")))?,
                item_name,
                unit: Unit::parse(&unit)
                    .ok_or_else(|| PluginError::Other(format!("unknown stock unit {unit:?}")))?,
                amount,
                used_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{env, meal_event};

    fn new(name: &str, quantity: f64, unit: Unit, expires_on: Option<&str>) -> NewStock {
        NewStock {
            name: name.into(),
            quantity,
            unit,
            expires_on: expires_on.map(|d| d.parse().unwrap()),
        }
    }

    #[tokio::test]
    async fn add_tops_up_the_same_item_and_keeps_the_earlier_best_before() {
        let env = env().await;
        add(&env.ctx, &new("鸡蛋", 6.0, Unit::Piece, Some("2026-10-10")))
            .await
            .unwrap();
        let item = add(
            &env.ctx,
            &new(" 鸡蛋 ", 10.0, Unit::Piece, Some("2026-10-20")),
        )
        .await
        .unwrap();

        assert_eq!(item.quantity, 16.0);
        assert_eq!(item.expires_on, "2026-10-10".parse().ok());
        // A different unit is a separate item.
        add(&env.ctx, &new("鸡蛋", 300.0, Unit::G, None))
            .await
            .unwrap();
        assert_eq!(list(&env.ctx).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_run_out_item_is_hidden_and_takes_the_new_date_when_restocked() {
        let env = env().await;
        let item = add(&env.ctx, &new("牛奶", 1000.0, Unit::Ml, Some("2026-10-01")))
            .await
            .unwrap();
        set_quantity(&env.ctx, &item.id, 0.0).await.unwrap();
        assert!(list(&env.ctx).await.unwrap().is_empty());

        let item = add(&env.ctx, &new("牛奶", 500.0, Unit::Ml, Some("2026-10-12")))
            .await
            .unwrap();
        assert_eq!(item.quantity, 500.0);
        assert_eq!(item.expires_on, "2026-10-12".parse().ok());
    }

    #[tokio::test]
    async fn rejects_bad_names_and_quantities() {
        let env = env().await;
        for bad in [
            new(" ", 1.0, Unit::G, None),
            new("米", 0.0, Unit::G, None),
            new("米", -1.0, Unit::G, None),
            new("米", f64::NAN, Unit::G, None),
        ] {
            assert!(matches!(
                add(&env.ctx, &bad).await,
                Err(PluginError::InvalidArgs(_))
            ));
        }
        assert!(matches!(
            set_quantity(&env.ctx, "nope", 1.0).await,
            Err(PluginError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn consume_caps_at_what_is_left_and_restore_puts_it_back_once() {
        let env = env().await;
        let eggs = add(&env.ctx, &new("鸡蛋", 3.0, Unit::Piece, None))
            .await
            .unwrap();
        let event = meal_event(&env.ctx, "早饭煎了四个鸡蛋", &[("鸡蛋", 200.0)]).await;

        let used = consume(&env.ctx, &event, &[(eggs.clone(), 4.0)])
            .await
            .unwrap();
        assert_eq!(used[0].amount, 3.0);
        assert!(list(&env.ctx).await.unwrap().is_empty());
        assert!(has_uses(&env.ctx, &event.id).await.unwrap());
        assert_eq!(recent_uses(&env.ctx, 10).await.unwrap().len(), 1);

        assert_eq!(restore(&env.ctx, &event.id).await.unwrap(), 1);
        assert_eq!(restore(&env.ctx, &event.id).await.unwrap(), 0);
        assert_eq!(list(&env.ctx).await.unwrap()[0].quantity, 3.0);
        assert!(recent_uses(&env.ctx, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restore_skips_items_removed_since() {
        let env = env().await;
        let rice = add(&env.ctx, &new("米", 2000.0, Unit::G, None))
            .await
            .unwrap();
        let event = meal_event(&env.ctx, "米饭", &[("熟米饭", 200.0)]).await;
        consume(&env.ctx, &event, &[(rice.clone(), 90.0)])
            .await
            .unwrap();
        delete(&env.ctx, &rice.id).await.unwrap();

        assert_eq!(restore(&env.ctx, &event.id).await.unwrap(), 1);
        assert!(list(&env.ctx).await.unwrap().is_empty());
    }
}
