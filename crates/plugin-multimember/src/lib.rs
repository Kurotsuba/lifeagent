//! `lifeagent-plugin-multimember`: the other people the owner cooks for, and shared meals.
//!
//! Optional and off by default (`docs/MULTIMEMBER_SPEC.md`). The owner describes household
//! members (16–70, no goals, no logins) and who eats which main meal; the plugin turns that into
//! a computed `multimember` plan section that the diet plugin reads to plan shared dishes with a
//! portion for each guest. Like every plugin, it depends only on `lifeagent-core` and
//! `lifeagent-llm`; diet knows it only through that section's JSON.

pub mod context;
pub mod model;
pub mod section;
pub mod store;
pub mod tools;

use async_trait::async_trait;
use chrono::NaiveDate;
use lifeagent_core::plugin::{
    PlanDraft, PlanSection, Plugin, PluginContext, PluginError, PluginManifest, ToolOutput,
};
use lifeagent_llm::types::ToolSpec;
use serde_json::Value;

/// Plugin id, tool and table prefix, and plan section name.
pub const SECTION: &str = "multimember";

pub struct MultimemberPlugin;

#[async_trait]
impl Plugin for MultimemberPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: SECTION,
            name: "Multimember",
            version: env!("CARGO_PKG_VERSION"),
            event_types: vec![],
            migrations: store::MIGRATIONS.into(),
        }
    }

    fn optional(&self) -> bool {
        true
    }

    fn computes_plan_section(&self) -> bool {
        true
    }

    async fn computed_plan_section(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        _prior: &PlanDraft,
    ) -> Result<Option<PlanSection>, PluginError> {
        section::computed_section(ctx, date).await
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
        Ok(Some(context::context_snippet(ctx, date).await?))
    }

    async fn today_card(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
    ) -> Result<Option<Value>, PluginError> {
        Ok(Some(context::today_card(ctx, date).await?))
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use chrono::NaiveDate;
    use lifeagent_core::plugin::{apply_migrations, Plugin, PluginContext};
    use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Sex};
    use lifeagent_core::test_support::TestEnv;

    use crate::model::MemberProfile;
    use crate::MultimemberPlugin;

    /// A core test environment with this plugin's tables created.
    pub async fn env() -> TestEnv {
        let env = lifeagent_core::test_support::env().await;
        apply_migrations(
            &env.ctx.db,
            env.clock.as_ref(),
            &MultimemberPlugin.manifest(),
        )
        .await
        .unwrap();
        env
    }

    pub async fn today(ctx: &PluginContext) -> NaiveDate {
        ctx.today().await.unwrap()
    }

    /// A valid member, born 1992.
    pub fn aki() -> MemberProfile {
        MemberProfile {
            display_name: "Aki".into(),
            sex: Sex::Female,
            birth_date: NaiveDate::from_ymd_opt(1992, 5, 10).unwrap(),
            height_cm: 160.0,
            weight_kg: 52.0,
            activity_level: ActivityLevel::Light,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec!["えび".into()],
        }
    }
}
