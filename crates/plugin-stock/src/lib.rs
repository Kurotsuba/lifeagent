//! `lifeagent-plugin-stock`: the food the user has at home.
//!
//! Stock is added from chat (`stock_add`) or the Stock tab, taken out automatically when a meal
//! is logged (`consume`, reacting to the diet plugin's `diet.meal_logged` event), put back when
//! that meal is deleted, and fed to the planner as its own `stock` section (`plan`) so plans use
//! what's at home and come with a shopping list for the rest. Hackathon build only; see
//! `docs/DECISIONS.md`, "Stock plugin".
//!
//! Depends on `lifeagent-core` and `lifeagent-llm` only. It knows the diet plugin solely through
//! that event's JSON, so it can be registered or left out without either plugin changing, and
//! both switches in [`settings`] turn its reads and writes off per account. The whole plugin can
//! also be switched off in Settings (`Plugin::optional`); it starts on.

pub mod consume;
pub mod model;
pub mod plan;
pub mod settings;
pub mod shopping;
pub mod store;
pub mod tools;
pub mod view;

use async_trait::async_trait;
use chrono::NaiveDate;
use lifeagent_core::events::Event;
use lifeagent_core::plugin::{
    PlanDraft, PlanInput, PlanSection, Plugin, PluginContext, PluginError, PluginManifest,
    PluginMigration, ToolOutput,
};
use lifeagent_llm::types::ToolSpec;
use serde_json::Value;

/// Items listed in the chat context; the rest are a `stock_list` call away.
const CONTEXT_ITEMS: usize = 30;

pub struct StockPlugin;

#[async_trait]
impl Plugin for StockPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "stock",
            name: "Stock",
            version: env!("CARGO_PKG_VERSION"),
            event_types: vec![],
            migrations: vec![
                PluginMigration {
                    version: 1,
                    sql: store::MIGRATION_1,
                },
                PluginMigration {
                    version: 2,
                    sql: shopping::MIGRATION_2,
                },
            ],
        }
    }

    /// Switchable in Settings, and on until the user switches it off (accounts made before
    /// the switch existed keep it).
    fn optional(&self) -> bool {
        true
    }

    fn enabled_by_default(&self) -> bool {
        true
    }

    fn tools(&self) -> Vec<ToolSpec> {
        tools::specs()
    }

    fn is_read_only_tool(&self, name: &str) -> bool {
        name == "stock_list"
    }

    async fn call_tool(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: Value,
    ) -> Result<ToolOutput, PluginError> {
        tools::call(ctx, name, args).await
    }

    async fn on_event(&self, ctx: &PluginContext, e: &Event) -> Result<(), PluginError> {
        if e.event_type == consume::MEAL_EVENT {
            consume::on_meal_logged(ctx, e).await?;
        }
        Ok(())
    }

    async fn on_event_deleted(&self, ctx: &PluginContext, e: &Event) -> Result<(), PluginError> {
        if e.event_type == consume::MEAL_EVENT {
            store::restore(ctx, &e.id).await?;
        }
        Ok(())
    }

    async fn context_snippet(
        &self,
        ctx: &PluginContext,
        _date: NaiveDate,
    ) -> Result<Option<String>, PluginError> {
        let items = store::list(ctx).await?;
        let settings = settings::get(ctx).await?;
        let mut lines = vec![if items.is_empty() {
            "Nothing in stock.".to_string()
        } else {
            format!("In stock ({} items):", items.len())
        }];
        for item in items.iter().take(CONTEXT_ITEMS) {
            let expiry = item
                .expires_on
                .map(|d| format!(", best before {d}"))
                .unwrap_or_default();
            lines.push(format!(
                "- {}: {} {}{expiry}",
                item.name,
                item.quantity,
                item.unit.as_str()
            ));
        }
        if settings.auto_consume {
            lines.push(
                "Meals logged with diet_log_meal take what they used out of stock by \
                 themselves; don't adjust stock for them."
                    .into(),
            );
        }
        Ok(Some(lines.join("\n")))
    }

    /// Stock builds the diet section's meals in the same planner call, so it goes in diet's
    /// stage: after the day's training and the multimember section, when those plugins are on.
    fn plan_after(&self) -> &'static [&'static str] {
        &["training", plan::MULTIMEMBER_SECTION]
    }

    async fn plan_input(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        prior: &PlanDraft,
    ) -> Result<Option<PlanInput>, PluginError> {
        plan::plan_input(ctx, date, prior).await
    }

    async fn finalize_plan_section(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        raw: Value,
        _prior: &PlanDraft,
    ) -> Result<PlanSection, PluginError> {
        plan::finalize(ctx, date, raw).await
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use chrono::NaiveDate;
    use lifeagent_core::events::{Event, EventSource};
    use lifeagent_core::plugin::{apply_migrations, Plugin, PluginContext};
    use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
    use lifeagent_core::test_support::TestEnv;
    use lifeagent_llm::types::{ChatResponse, Usage};
    use serde_json::{json, Value};

    use crate::consume::MEAL_EVENT;
    use crate::StockPlugin;

    /// A core test environment with the stock plugin's tables created.
    pub async fn env() -> TestEnv {
        let env = lifeagent_core::test_support::env().await;
        apply_migrations(&env.ctx.db, env.clock.as_ref(), &StockPlugin.manifest())
            .await
            .unwrap();
        env
    }

    pub fn json_response(value: Value) -> ChatResponse {
        ChatResponse {
            content: Some(value.to_string()),
            tool_calls: vec![],
            usage: Usage::default(),
            finish_reason: "stop".into(),
        }
    }

    /// Appends a `diet.meal_logged` event with the given payload, as the diet plugin would.
    pub async fn event_with_payload(ctx: &PluginContext, payload: Value) -> Event {
        ctx.events
            .append(MEAL_EVENT, ctx.clock.now(), payload, EventSource::Chat)
            .await
            .unwrap()
    }

    /// A `diet.meal_logged` event of food items `(name, grams)`.
    pub async fn meal_event(ctx: &PluginContext, raw_text: &str, foods: &[(&str, f64)]) -> Event {
        let items: Vec<Value> = foods
            .iter()
            .map(|(name, grams)| json!({"name": name, "kind": "food", "grams": grams}))
            .collect();
        event_with_payload(
            ctx,
            json!({"meal_type": "lunch", "raw_text": raw_text, "items": items}),
        )
        .await
    }

    /// A profile and a weight, so the stock section is planned.
    pub async fn seed_person(ctx: &PluginContext) {
        ctx.profile
            .set(&Profile {
                display_name: None,
                sex: Sex::Female,
                birth_date: NaiveDate::from_ymd_opt(1994, 3, 1).unwrap(),
                height_cm: 165.0,
                activity_level: ActivityLevel::Light,
                dietary_pattern: DietaryPattern::Omnivore,
                allergies: vec![],
                medical_notes: None,
                meals_per_day: 3,
            })
            .await
            .unwrap();
        ctx.events
            .append(
                "body.weight_logged",
                ctx.clock.now(),
                json!({"kg": 62.0}),
                EventSource::Ui,
            )
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Unit;
    use crate::store::NewStock;
    use crate::testutil::{env, json_response, meal_event};
    use serde_json::json;

    #[tokio::test]
    async fn a_deleted_meal_puts_its_stock_back() {
        let env = env().await;
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
        let event = meal_event(&env.ctx, "两个鸡蛋", &[("鸡蛋", 100.0)]).await;
        env.mock
            .push_response(json_response(json!({"uses": [{"stock": 0, "amount": 2}]})));

        StockPlugin.on_event(&env.ctx, &event).await.unwrap();
        assert_eq!(store::list(&env.ctx).await.unwrap()[0].quantity, 4.0);

        env.ctx.events.soft_delete(&event.id).await.unwrap();
        StockPlugin
            .on_event_deleted(&env.ctx, &event)
            .await
            .unwrap();
        assert_eq!(store::list(&env.ctx).await.unwrap()[0].quantity, 6.0);
    }

    #[tokio::test]
    async fn ignores_other_events() {
        let env = env().await;
        let weight = env
            .ctx
            .events
            .append(
                "body.weight_logged",
                env.ctx.clock.now(),
                json!({"kg": 60.0}),
                lifeagent_core::events::EventSource::Ui,
            )
            .await
            .unwrap();
        StockPlugin.on_event(&env.ctx, &weight).await.unwrap();
        StockPlugin
            .on_event_deleted(&env.ctx, &weight)
            .await
            .unwrap();
        assert!(env.mock.received_requests().is_empty());
    }

    #[tokio::test]
    async fn context_snippet_lists_stock_and_the_auto_consume_note() {
        let env = env().await;
        store::add(
            &env.ctx,
            &NewStock {
                name: "豆腐".into(),
                quantity: 2.0,
                unit: Unit::Piece,
                expires_on: "2026-10-06".parse().ok(),
            },
        )
        .await
        .unwrap();
        let today = env.ctx.today().await.unwrap();
        let snippet = StockPlugin
            .context_snippet(&env.ctx, today)
            .await
            .unwrap()
            .unwrap();
        assert!(snippet.contains("- 豆腐: 2 piece, best before 2026-10-06"));
        assert!(snippet.contains("diet_log_meal"));
    }
}
