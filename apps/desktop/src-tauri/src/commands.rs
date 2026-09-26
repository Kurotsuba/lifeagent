//! Tauri commands (`get_app_state`, `send_message`, `get_today`, ...), per
//! `docs/PHASE1_SPEC.md` §10.2.
//!
//! Every command is a thin wrapper: extract `CoreApp` from managed state, call an existing (or,
//! where none existed, a small new) `CoreApp`/service method, map the error through
//! `CommandError`. No business logic lives here beyond the odd default (e.g. `generate_plan`'s
//! date) already established by the CLI in M6.
//!
//! **`autostart`**: `save_settings` persists the `autostart` flag and then syncs the OS-level
//! registration through `tauri_plugin_autostart` (`sync_autostart`), which is why it alone also
//! takes the `AppHandle`.

use std::sync::Mutex;
use std::time::Instant;

use chrono::NaiveDate;
use chrono::{DateTime, Utc};
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
use lifeagent_plugin_diet::import::{
    self, FoodDataError, ImportPreview, ImportReport, ImportTarget, ParsedImport,
};
use lifeagent_plugin_diet::sources::{self, Food, SourceInfo};
use lifeagent_plugin_diet::targets::{self, NutritionTargets};
use serde::{Deserialize, Serialize};
use tauri::ipc::InvokeBody;
use tauri::State;
use uuid::Uuid;

use crate::error::CommandError;

/// `generate_plan`'s `date` if given (`YYYY-MM-DD`), else tomorrow — the plan a user is about
/// to eat, not today's already-settled one. Mirrors `lifeagent-cli`'s own `resolve_date` (M6).
/// `pub(crate)` so the tray's 生成明日计划 item (`tray.rs`) can resolve the same default.
pub(crate) async fn resolve_plan_date(
    app: &CoreApp,
    date: Option<String>,
) -> Result<NaiveDate, CommandError> {
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
        .set_onboarding_complete(true, state.clock().now())
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
    Ok(state.save_weight_goal(&goal).await?)
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
    app: tauri::AppHandle,
    state: State<'_, CoreApp>,
    settings: SettingsDto,
) -> Result<(), CommandError> {
    let now = state.clock().now();
    let plan_time = parse_hhmm(&settings.plan_time)?;
    let day_boundary = parse_hhmm(&settings.day_boundary)?;
    let s = state.settings();
    s.set_plan_time_and_day_boundary(plan_time, day_boundary, now)
        .await?;
    s.set_notifications_enabled(settings.notifications_enabled, now)
        .await?;
    s.set_autostart(settings.autostart, now).await?;
    sync_autostart(&app, settings.autostart);
    Ok(())
}

/// Registers/unregisters the OS-level autostart entry to match the just-saved setting. Best
/// effort: a failure here (e.g. no permission to write the registry run key) shouldn't fail the
/// save of the user's actual preference, which already succeeded above — logged instead, same
/// as `notify.rs`'s own treatment of non-critical OS integration failures. Also called from
/// `setup()` on every launch. Only touches the registration when it differs from `enabled`:
/// disabling an entry that was never registered fails on Windows ("file not found").
pub(crate) fn sync_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let result = autolaunch.is_enabled().and_then(|registered| {
        if registered == enabled {
            Ok(())
        } else if enabled {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        }
    });
    if let Err(e) = result {
        tracing::error!(error = %e, enabled, "failed to sync OS autostart registration");
    }
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
    state
        .settings()
        .set_models(&models, state.clock().now())
        .await?;
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

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md` §8.2) --------------------

#[tauri::command]
pub async fn list_food_sources(state: State<'_, CoreApp>) -> Result<Vec<SourceInfo>, CommandError> {
    Ok(sources::list_sources(&state.plugin_context()).await?)
}

#[tauri::command]
pub async fn list_foods(
    state: State<'_, CoreApp>,
    source_id: Option<String>,
    query: Option<String>,
    limit: u32,
) -> Result<Vec<Food>, CommandError> {
    Ok(sources::list_foods(
        &state.plugin_context(),
        source_id.as_deref(),
        query.as_deref(),
        limit,
    )
    .await?)
}

#[tauri::command]
pub async fn reorder_food_sources(
    state: State<'_, CoreApp>,
    ids: Vec<String>,
) -> Result<(), CommandError> {
    Ok(sources::reorder_datasets(&state.plugin_context(), &ids).await?)
}

#[tauri::command]
pub async fn set_food_source_enabled(
    state: State<'_, CoreApp>,
    id: String,
    enabled: bool,
) -> Result<(), CommandError> {
    Ok(sources::set_enabled(&state.plugin_context(), &id, enabled).await?)
}

#[tauri::command]
pub async fn rename_food_source(
    state: State<'_, CoreApp>,
    id: String,
    name: String,
) -> Result<(), CommandError> {
    Ok(sources::rename_source(&state.plugin_context(), &id, &name).await?)
}

#[tauri::command]
pub async fn delete_food_source(state: State<'_, CoreApp>, id: String) -> Result<(), CommandError> {
    Ok(sources::delete_source(&state.plugin_context(), &id).await?)
}

/// A parsed-but-not-yet-committed import, kept in managed state between `preview_food_import`
/// and `commit_food_import`/`cancel_food_import` (§8.2 "Preview → commit token"). There is a
/// single slot: a new preview replaces the old one, and an entry expires after
/// [`PENDING_IMPORT_TTL_SECS`] via the injected `Clock`.
struct PendingImportRecord {
    token: Uuid,
    parsed: ParsedImport,
    created_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct PendingImportState(Mutex<Option<PendingImportRecord>>);

const PENDING_IMPORT_TTL_SECS: i64 = 15 * 60;

/// Minimal percent-decoding for the `x-file-name` header (`encodeURIComponent` on the frontend).
/// Not a general `Vec<u8>`-first decoder: invalid `%XX` sequences are left as literal text, which
/// only ever means a slightly odd display name, never a parsing failure.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or_default(),
                16,
            ) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Serialize)]
pub struct ImportPreviewWithToken {
    #[serde(flatten)]
    pub preview: ImportPreview,
    pub token: String,
}

/// Raw IPC body (§8.2 "Getting the file to Rust"): the frontend sends the file's bytes directly
/// (`new Uint8Array(await file.arrayBuffer())`) with the file name url-encoded in the
/// `x-file-name` header, so no JSON-encoded byte array and no new Tauri plugin are needed.
#[tauri::command]
pub async fn preview_food_import(
    request: tauri::ipc::Request<'_>,
    state: State<'_, CoreApp>,
    pending: State<'_, PendingImportState>,
) -> Result<ImportPreviewWithToken, CommandError> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(_) => {
            return Err(FoodDataError::Invalid("请求缺少文件内容。".into()).into())
        }
    };
    let file_name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_else(|| "import".to_string());

    let parsed = import::parse(&file_name, &bytes)?;
    let preview = import::preview(&state.plugin_context(), &parsed).await?;

    let token = Uuid::now_v7();
    *pending.0.lock().expect("pending import mutex poisoned") = Some(PendingImportRecord {
        token,
        parsed,
        created_at: state.clock().now(),
    });

    Ok(ImportPreviewWithToken {
        preview,
        token: token.to_string(),
    })
}

/// Consumes the pending import if `token` matches and hasn't expired; otherwise an expired/
/// unknown token returns `not_found` with the "re-select the file" message (§8.2).
fn take_pending_import(
    core: &CoreApp,
    pending: &PendingImportState,
    token: &str,
) -> Result<ParsedImport, CommandError> {
    const EXPIRED: &str = "导入已过期，请重新选择文件";
    let mut guard = pending.0.lock().expect("pending import mutex poisoned");
    let expired = guard
        .as_ref()
        .map(|p| (core.clock().now() - p.created_at).num_seconds() > PENDING_IMPORT_TTL_SECS)
        .unwrap_or(false);
    if expired {
        *guard = None;
    }
    match guard.take() {
        Some(record) if record.token.to_string() == token => Ok(record.parsed),
        Some(record) => {
            // A stale/mismatched invoke (e.g. a second browser tab) must not drop the import
            // that's still pending for whoever holds the current token.
            *guard = Some(record);
            Err(FoodDataError::NotFound(EXPIRED.into()).into())
        }
        None => Err(FoodDataError::NotFound(EXPIRED.into()).into()),
    }
}

#[tauri::command]
pub async fn commit_food_import(
    state: State<'_, CoreApp>,
    pending: State<'_, PendingImportState>,
    token: String,
    name: Option<String>,
    replace_source_id: Option<String>,
) -> Result<ImportReport, CommandError> {
    let parsed = take_pending_import(state.inner(), pending.inner(), &token)?;
    let target = match replace_source_id {
        Some(source_id) => ImportTarget::Replace { source_id },
        None => ImportTarget::New {
            name: name.unwrap_or_else(|| parsed.suggested_name.clone()),
        },
    };
    Ok(import::commit(&state.plugin_context(), parsed, target).await?)
}

#[tauri::command]
pub async fn cancel_food_import(
    pending: State<'_, PendingImportState>,
    token: String,
) -> Result<(), CommandError> {
    let mut guard = pending.0.lock().expect("pending import mutex poisoned");
    if guard.as_ref().is_some_and(|p| p.token.to_string() == token) {
        *guard = None;
    }
    Ok(())
}
