//! `lifeagent-plugin-training`: workout logging and the daily training plan section.
//!
//! Optional (`Plugin::optional`): off until the user enables it in Settings. The user logs
//! sessions in chat (`training_log_session`); the plan for a day follows their standing
//! preferences, an optional request for that day, and the last 7 days' log. Its section is
//! planned before the diet section, which reads the session's computed `exercise_kcal` and
//! timing to fuel it (`docs/DECISIONS.md`, "Training plugin"). Like every plugin, it depends
//! only on `lifeagent-core` and `lifeagent-llm`.

pub mod energy;
pub mod log;
pub mod model;
pub mod plan;
pub mod prefs;
pub mod tools;
pub mod validate;
pub mod view;

use async_trait::async_trait;
use chrono::NaiveDate;
use lifeagent_core::plugin::{
    PlanDraft, PlanInput, PlanSection, Plugin, PluginContext, PluginError, PluginManifest,
    PluginMigration, ToolOutput,
};
use lifeagent_llm::types::ToolSpec;
use serde_json::Value;

pub struct TrainingPlugin;

#[async_trait]
impl Plugin for TrainingPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: log::SECTION,
            name: "Training",
            version: env!("CARGO_PKG_VERSION"),
            event_types: vec![log::SESSION_EVENT],
            migrations: vec![PluginMigration {
                version: 1,
                sql: prefs::MIGRATION_1,
            }],
        }
    }

    fn optional(&self) -> bool {
        true
    }

    fn tools(&self) -> Vec<ToolSpec> {
        tools::specs()
    }

    async fn approval_preview(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: &Value,
    ) -> Result<Option<Value>, PluginError> {
        tools::approval_preview(ctx, name, args).await
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
        Ok(Some(log::context_snippet(ctx, date).await?))
    }

    async fn today_card(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
    ) -> Result<Option<Value>, PluginError> {
        Ok(Some(log::today_card(ctx, date).await?))
    }

    async fn plan_input(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        _prior: &PlanDraft,
    ) -> Result<Option<PlanInput>, PluginError> {
        plan::plan_input(ctx, date).await
    }

    async fn finalize_plan_section(
        &self,
        ctx: &PluginContext,
        date: NaiveDate,
        raw: Value,
        _prior: &PlanDraft,
    ) -> Result<PlanSection, PluginError> {
        validate::finalize(ctx, date, raw).await
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use chrono::{Local, NaiveDate, TimeZone, Utc};
    use lifeagent_core::events::EventSource;
    use lifeagent_core::plugin::{apply_migrations, Plugin, PluginContext};
    use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
    use lifeagent_core::test_support::TestEnv;
    use serde_json::json;

    use crate::log::SESSION_EVENT;
    use crate::model::{Exercise, ExerciseKind, Intensity, SessionLogged};
    use crate::TrainingPlugin;

    /// A core test environment with the training plugin's tables created.
    pub async fn env() -> TestEnv {
        let env = lifeagent_core::test_support::env().await;
        apply_migrations(&env.ctx.db, env.clock.as_ref(), &TrainingPlugin.manifest())
            .await
            .unwrap();
        env
    }

    /// A profile and a 62 kg weight.
    pub async fn seed_profile(ctx: &PluginContext) {
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

    fn session(
        title: &str,
        intensity: Intensity,
        minutes: f64,
        kcal: Option<f64>,
    ) -> SessionLogged {
        SessionLogged {
            title: title.into(),
            raw_text: String::new(),
            exercises: vec![Exercise {
                name: title.into(),
                kind: ExerciseKind::Cardio,
                intensity,
                sets: None,
                reps: None,
                weight_kg: None,
                duration_min: Some(minutes),
                distance_km: None,
                note: None,
            }],
            duration_min: minutes,
            exercise_kcal: kcal,
            intensity,
            rpe: None,
            notes: None,
        }
    }

    async fn append(ctx: &PluginContext, at: chrono::DateTime<Utc>, s: &SessionLogged) -> String {
        ctx.events
            .append(
                SESSION_EVENT,
                at,
                serde_json::to_value(s).unwrap(),
                EventSource::Chat,
            )
            .await
            .unwrap()
            .id
    }

    /// Appends a moderate session at the clock's "now"; returns its entry id.
    pub async fn log_session(
        ctx: &PluginContext,
        title: &str,
        minutes: f64,
        kcal: Option<f64>,
    ) -> String {
        let s = session(title, Intensity::Moderate, minutes, kcal);
        append(ctx, ctx.clock.now(), &s).await
    }

    /// Appends a session at noon (local) on `date`, so its logical date is `date`.
    pub async fn log_session_with(
        ctx: &PluginContext,
        date: NaiveDate,
        intensity: Intensity,
        minutes: f64,
    ) -> String {
        let noon = Local
            .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .with_timezone(&Utc);
        append(ctx, noon, &session("训练", intensity, minutes, None)).await
    }
}
