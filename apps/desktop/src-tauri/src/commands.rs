//! Tauri commands (`get_app_state`, `send_message`, `get_today`, ...), per
//! `docs/PHASE1_SPEC.md` §10.2.
//!
//! Every command is a thin wrapper: extract `CoreApp` from managed state, call an existing (or,
//! where none existed, a small new) `CoreApp`/service method, map the error through
//! `CommandError`. No business logic lives here beyond the odd default (e.g. `generate_plan`'s
//! date) already established by the CLI in M6.
//!
//! **`autostart`**: `save_settings` only persists the `autostart` flag. Actually
//! registering/unregistering the OS autostart entry via `tauri_plugin_autostart` is M7.5 work,
//! done alongside the tray (see `docs/M7_SCOPE.md`) — it needs the `AppHandle`-level autostart
//! manager, not just `CoreApp`.

use std::time::Instant;

use chrono::{NaiveDate, Utc};
use lifeagent_core::app::{ChatTurn, CoreApp, HistoryMessage, TodayView};
use lifeagent_core::error::CoreError;
use lifeagent_core::goals::WeightGoal;
use lifeagent_core::memory::MemoryNote;
use lifeagent_core::plan::{PlanTrigger, StoredPlan};
use lifeagent_core::profile::Profile;
use lifeagent_core::time::{age_years, format_hhmm, parse_hhmm};
use lifeagent_core::usage::RoleUsage;
use lifeagent_llm::router::{ModelsConfig, RoleRequest};
use lifeagent_llm::types::{Message, ResponseFormat, Role};
use lifeagent_plugin_diet::targets::{self, NutritionTargets};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::CommandError;

/// `generate_plan`'s `date` if given (`YYYY-MM-DD`), else tomorrow — the plan a user is about
/// to eat, not today's already-settled one. Mirrors `lifeagent-cli`'s own `resolve_date` (M6).
async fn resolve_plan_date(app: &CoreApp, date: Option<String>) -> Result<NaiveDate, CommandError> {
    match date {
        Some(s) => parse_date(&s),
        None => {
            let today = app.logical_today().await?;
            today
                .succ_opt()
                .ok_or_else(|| CoreError::Validation("date out of range".into()).into())
        }
    }
}

fn parse_date(s: &str) -> Result<NaiveDate, CommandError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| CoreError::Validation(format!("invalid date {s:?}: {e}")).into())
}

#[derive(Debug, Clone, Serialize)]
pub struct AppState {
    onboarding_complete: bool,
    models_configured: bool,
    logical_date: NaiveDate,
}

#[tauri::command]
pub async fn get_app_state(state: State<'_, CoreApp>) -> Result<AppState, CommandError> {
    Ok(AppState {
        onboarding_complete: state.settings().onboarding_complete().await?,
        models_configured: state.models_configured(),
        logical_date: state.logical_today().await?,
    })
}

#[tauri::command]
pub async fn complete_onboarding(state: State<'_, CoreApp>) -> Result<(), CommandError> {
    state
        .settings()
        .set_onboarding_complete(true, Utc::now())
        .await?;
    Ok(())
}

// ---- Chat --------------------------------------------------------------------------------

#[tauri::command]
pub async fn send_message(
    state: State<'_, CoreApp>,
    text: String,
) -> Result<ChatTurn, CommandError> {
    Ok(state.send_message(&text).await?)
}

#[tauri::command]
pub async fn get_history(
    state: State<'_, CoreApp>,
    limit: u32,
) -> Result<Vec<HistoryMessage>, CommandError> {
    Ok(state.get_history(limit).await?)
}

// ---- Today / plan --------------------------------------------------------------------------

#[tauri::command]
pub async fn get_today(state: State<'_, CoreApp>) -> Result<TodayView, CommandError> {
    Ok(state.today_view().await?)
}

#[tauri::command]
pub async fn get_plan(
    state: State<'_, CoreApp>,
    date: String,
) -> Result<Option<StoredPlan>, CommandError> {
    let date = parse_date(&date)?;
    Ok(state.get_plan(date).await?)
}

#[tauri::command]
pub async fn generate_plan(
    state: State<'_, CoreApp>,
    date: Option<String>,
) -> Result<StoredPlan, CommandError> {
    let date = resolve_plan_date(&state, date).await?;
    Ok(state.generate_plan(date, PlanTrigger::Manual).await?)
}

#[tauri::command]
pub async fn accept_plan(state: State<'_, CoreApp>, date: String) -> Result<(), CommandError> {
    let date = parse_date(&date)?;
    Ok(state.accept_plan(date).await?)
}

#[tauri::command]
pub async fn delete_entry(state: State<'_, CoreApp>, id: String) -> Result<(), CommandError> {
    Ok(state.delete_entry(&id).await?)
}

// ---- Profile / goals / weight ---------------------------------------------------------------

#[tauri::command]
pub async fn get_profile(state: State<'_, CoreApp>) -> Result<Option<Profile>, CommandError> {
    Ok(state.profile().get().await?)
}

#[tauri::command]
pub async fn save_profile(state: State<'_, CoreApp>, profile: Profile) -> Result<(), CommandError> {
    Ok(state.profile().set(&profile).await?)
}

#[tauri::command]
pub async fn get_weight_goal(
    state: State<'_, CoreApp>,
) -> Result<Option<WeightGoal>, CommandError> {
    Ok(state.goals().get_active_weight_goal().await?)
}

#[tauri::command]
pub async fn save_weight_goal(
    state: State<'_, CoreApp>,
    goal: WeightGoal,
) -> Result<(), CommandError> {
    Ok(state.goals().save_weight_goal(&goal).await?)
}

#[tauri::command]
pub async fn log_weight(state: State<'_, CoreApp>, kg: f64) -> Result<(), CommandError> {
    Ok(state.log_weight(kg).await?)
}

/// Pure computation (no DB write): the desktop crate already depends on
/// `lifeagent-plugin-diet`, so it calls `targets::compute` directly rather than going through
/// `CoreApp`, per the spec's own note for this command.
#[tauri::command]
pub async fn preview_targets(
    state: State<'_, CoreApp>,
    profile: Profile,
    weight_kg: f64,
    goal: Option<WeightGoal>,
) -> Result<NutritionTargets, CommandError> {
    let today = state.logical_today().await?;
    let age = age_years(profile.birth_date, today);
    Ok(targets::compute(&profile, weight_kg, age, goal.as_ref()))
}

// ---- Settings -------------------------------------------------------------------------------

/// All of §7.2's settings keys except `models` (its own `get_models`/`save_models` pair) and
/// `onboarding_complete` (its own `complete_onboarding`, which only ever sets it to `true` — the
/// Settings page never exposes a way to un-complete onboarding, so it isn't part of this
/// general-purpose settings form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsDto {
    plan_time: String,
    day_boundary: String,
    notifications_enabled: bool,
    autostart: bool,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, CoreApp>) -> Result<SettingsDto, CommandError> {
    let settings = state.settings();
    Ok(SettingsDto {
        plan_time: format_hhmm(settings.plan_time().await?),
        day_boundary: format_hhmm(settings.day_boundary().await?),
        notifications_enabled: settings.notifications_enabled().await?,
        autostart: settings.autostart().await?,
    })
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, CoreApp>,
    settings: SettingsDto,
) -> Result<(), CommandError> {
    let now = Utc::now();
    let plan_time = parse_hhmm(&settings.plan_time)?;
    let day_boundary = parse_hhmm(&settings.day_boundary)?;
    let s = state.settings();
    s.set_plan_time_and_day_boundary(plan_time, day_boundary, now)
        .await?;
    s.set_notifications_enabled(settings.notifications_enabled, now)
        .await?;
    s.set_autostart(settings.autostart, now).await?;
    Ok(())
}

// ---- Models / API keys -----------------------------------------------------------------------

#[tauri::command]
pub async fn get_models(state: State<'_, CoreApp>) -> Result<Option<ModelsConfig>, CommandError> {
    Ok(state.settings().models().await?)
}

#[tauri::command]
pub async fn save_models(
    state: State<'_, CoreApp>,
    models: ModelsConfig,
) -> Result<(), CommandError> {
    state.settings().set_models(&models, Utc::now()).await?;
    // Takes effect on this already-running `CoreApp` immediately, not just after a restart —
    // see `docs/DECISIONS.md`'s M7.1 `reload_models` entry.
    state.reload_models().await?;
    Ok(())
}

#[tauri::command]
pub async fn set_api_key(
    state: State<'_, CoreApp>,
    provider_id: String,
    key: String,
) -> Result<(), CommandError> {
    state.secrets().set(&provider_id, &key)?;
    // A key change only matters once a `models` config actually references `provider_id`;
    // `reload_models` is a no-op (still "not configured") until `save_models` has run, and a
    // harmless rebuild if it already has — either way, safe to call unconditionally.
    state.reload_models().await?;
    Ok(())
}

#[tauri::command]
pub async fn has_api_key(
    state: State<'_, CoreApp>,
    provider_id: String,
) -> Result<bool, CommandError> {
    Ok(state.secrets().has(&provider_id)?)
}

#[derive(Debug, Clone, Serialize)]
pub struct TestModelResult {
    ok: bool,
    latency_ms: u64,
    error: Option<String>,
}

/// Sends "Reply with OK." on `role`'s currently-bound model (§10.2), for onboarding's and
/// Settings' 测试连接 buttons. Bypasses the orchestrator entirely — no history, no tools, no
/// message persisted — since this is a connectivity check, not a conversation turn.
#[tauri::command]
pub async fn test_model(
    state: State<'_, CoreApp>,
    role: Role,
) -> Result<TestModelResult, CommandError> {
    let llm = state.plugin_context().llm;
    let start = Instant::now();
    let result = llm
        .complete(
            role,
            RoleRequest {
                messages: vec![Message::User {
                    content: "Reply with OK.".into(),
                }],
                tools: vec![],
                max_tokens: Some(16),
                response_format: ResponseFormat::Text,
            },
        )
        .await;
    let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(match result {
        Ok(_) => TestModelResult {
            ok: true,
            latency_ms,
            error: None,
        },
        Err(e) => {
            let message = CommandError::from(CoreError::Llm(e)).message;
            TestModelResult {
                ok: false,
                latency_ms,
                error: Some(message),
            }
        }
    })
}

// ---- Memory ---------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_memory(
    state: State<'_, CoreApp>,
    limit: u32,
) -> Result<Vec<MemoryNote>, CommandError> {
    Ok(state.memory().list(limit).await?)
}

#[tauri::command]
pub async fn delete_memory(state: State<'_, CoreApp>, id: String) -> Result<(), CommandError> {
    Ok(state.memory().forget(&id).await?)
}

// ---- Usage ----------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_usage_summary(state: State<'_, CoreApp>) -> Result<Vec<RoleUsage>, CommandError> {
    Ok(state.usage_summary().await?)
}
