//! `lifeagent-plugin-diet`: meal logging, nutrition targets and the diet plan section.
//!
//! Depends on `lifeagent-core` and `lifeagent-llm`; implements the `Plugin` trait from
//! `lifeagent-core`. Registered by the composition root (`cli`, `desktop`) — `core` never
//! depends on this crate. Logging (`model`, `foods`, `resolver`, `parser`, `targets`,
//! `intake`, `tools`) landed in M4 and the plan section (`plan`, `validate`) in M5, per
//! `docs/PHASE1_SPEC.md` §8.

pub mod dish_resolver;
pub mod food_matcher;
pub mod foods;
pub mod import;
pub mod intake;
pub mod model;
pub mod parser;
pub mod plan;
pub mod prompts;
pub mod resolver;
pub mod sources;
pub mod targets;
pub mod tools;
pub mod validate;

use async_trait::async_trait;
use chrono::NaiveDate;
use lifeagent_core::plugin::{
    PlanInput, PlanSection, Plugin, PluginContext, PluginError, PluginManifest, PluginMigration,
    ToolOutput,
};
use lifeagent_llm::types::ToolSpec;
use serde_json::Value;

pub struct DietPlugin;

#[async_trait]
impl Plugin for DietPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "diet",
            name: "Diet",
            version: env!("CARGO_PKG_VERSION"),
            event_types: vec![intake::MEAL_EVENT],
            migrations: vec![PluginMigration {
                version: 1,
                sql: sources::MIGRATION_1,
            }],
        }
    }

    fn tools(&self) -> Vec<ToolSpec> {
        tools::specs()
    }

    async fn call_tool(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: Value,
    ) -> Result<ToolOutput, PluginError> {
        tools::call(ctx, name, args).await
    }

    async fn context_snippet(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
    ) -> Result<Option<String>, PluginError> {
        Ok(Some(intake::context_snippet(ctx, date).await?))
    }

    async fn today_card(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
    ) -> Result<Option<Value>, PluginError> {
        Ok(Some(intake::today_card(ctx, date).await?))
    }

    async fn plan_input(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
    ) -> Result<Option<PlanInput>, PluginError> {
        plan::plan_input(ctx, date).await
    }

    async fn finalize_plan_section(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        raw: Value,
    ) -> Result<PlanSection, PluginError> {
        validate::finalize(ctx, date, raw).await
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use chrono::{Local, NaiveDate, TimeZone, Utc};
    use lifeagent_core::clock::Clock;
    use lifeagent_core::events::EventSource;
    use lifeagent_core::goals::{GoalDirection, WeightGoal};
    use lifeagent_core::plugin::{apply_migrations, Plugin, PluginContext};
    use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
    use lifeagent_core::test_support::TestEnv;
    use lifeagent_llm::error::LlmError;
    use lifeagent_llm::types::{ChatRequest, ChatResponse, Message, Usage};
    use serde_json::{json, Value};

    use crate::intake::MEAL_EVENT;
    use crate::model::{Macros, MealLogged, MealType};
    use crate::DietPlugin;

    /// Per 100 g; both pass the §8.3 check unchanged.
    pub const RICE: Macros = Macros {
        kcal: 116.0,
        protein_g: 2.6,
        carbs_g: 25.9,
        fat_g: 0.3,
        fiber_g: 0.3,
    };
    pub const EGG: Macros = Macros {
        kcal: 144.0,
        protein_g: 13.3,
        carbs_g: 2.8,
        fat_g: 8.8,
        fiber_g: 0.0,
    };

    /// A core test environment with the diet plugin's tables created.
    pub async fn env() -> TestEnv {
        let env = lifeagent_core::test_support::env().await;
        apply_migrations(&env.ctx.db, env.clock.as_ref(), &DietPlugin.manifest())
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

    /// Answers a §9.3 food-estimate request for 熟米饭 or 鸡蛋 by reading the food name out of
    /// the prompt, so concurrent estimates can arrive in any order.
    pub fn estimate_responder(req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let prompt = match req.messages.first() {
            Some(Message::User { content }) => content.as_str(),
            _ => "",
        };
        let m = if prompt.ends_with("Food: 熟米饭") {
            RICE
        } else if prompt.ends_with("Food: 鸡蛋") {
            EGG
        } else {
            return Err(LlmError::Config(format!(
                "unexpected estimate prompt: {prompt}"
            )));
        };
        Ok(json_response(json!({
            "kcal": m.kcal, "protein_g": m.protein_g, "carbs_g": m.carbs_g,
            "fat_g": m.fat_g, "fiber_g": m.fiber_g
        })))
    }

    /// Creates an enabled dataset source at the lowest priority so far, holding `foods`, and
    /// returns its id. Rows are inserted directly, so a test needs no import file.
    pub async fn add_dataset(env: &TestEnv, name: &str, foods: &[(&str, Macros)]) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let now = env.clock.now().to_rfc3339();
        let priority: (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(priority), 0) + 1 FROM diet_food_sources WHERE kind = 'dataset'",
        )
        .fetch_one(&env.ctx.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO diet_food_sources
                (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
             VALUES ($1, 'dataset', $2, $3, 1, '{}', $4, $5)",
        )
        .bind(&id)
        .bind(name)
        .bind(priority.0)
        .bind(&now)
        .bind(&now)
        .execute(&env.ctx.db)
        .await
        .unwrap();
        for (food, m) in foods {
            sqlx::query(
                "INSERT INTO diet_foods
                    (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
                     use_count, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, $10, $11)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&id)
            .bind(*food)
            .bind(crate::foods::normalize(food))
            .bind(m.kcal)
            .bind(m.protein_g)
            .bind(m.carbs_g)
            .bind(m.fat_g)
            .bind(m.fiber_g)
            .bind(&now)
            .bind(&now)
            .execute(&env.ctx.db)
            .await
            .unwrap();
        }
        id
    }

    /// Bumps a dataset's `updated_at` to the clock's "now", as an import, replace, enable or
    /// reorder does.
    pub async fn touch_dataset(env: &TestEnv, id: &str) {
        sqlx::query("UPDATE diet_food_sources SET updated_at = $1 WHERE id = $2")
            .bind(env.clock.now().to_rfc3339())
            .bind(id)
            .execute(&env.ctx.db)
            .await
            .unwrap();
    }

    /// A fixed local date at `hour:minute`, as a UTC instant.
    pub fn local_instant(hour: u32, minute: u32) -> chrono::DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(2026, 9, 24)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap();
        Local
            .from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Profile, weight and a Lose goal, so targets can be computed.
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
                dislikes: vec![],
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
        ctx.goals
            .save_weight_goal(&WeightGoal {
                direction: GoalDirection::Lose,
                target_weight_kg: Some(58.0),
                rate_kg_per_week: 0.5,
            })
            .await
            .unwrap();
    }

    /// Appends a `diet.meal_logged` event at the clock's "now" with the given totals; returns
    /// its entry id.
    pub async fn log_meal_event(
        ctx: &PluginContext,
        meal_type: MealType,
        kcal: f64,
        protein_g: f64,
    ) -> String {
        let meal = MealLogged {
            meal_type,
            raw_text: "test".into(),
            items: vec![],
            total: Macros {
                kcal,
                protein_g,
                ..Macros::default()
            },
        };
        ctx.events
            .append(
                MEAL_EVENT,
                ctx.clock.now(),
                serde_json::to_value(&meal).unwrap(),
                EventSource::Chat,
            )
            .await
            .unwrap()
            .id
    }
}
