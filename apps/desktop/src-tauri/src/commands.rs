//! Tauri commands (`get_app_state`, `send_message`, `get_today`, ...), per
//! `docs/PHASE1_SPEC.md` §10.2.
//!
//! Each command extracts `CoreApp` from managed state and calls the matching function in
//! `lifeagent_api::ops`, which the web server shares. Only what is specific to Tauri stays
//! here: argument extraction, the raw IPC body of `preview_food_import`, and the OS autostart
//! sync after `save_settings` (which is why that command alone also takes the `AppHandle`).

use lifeagent_api::ops::{
    self, AppState, FoodOrDishList, FoodsKind, ImportPreviewWithToken, ListFoods,
    PendingImportState, SettingsDto, TestModelResult,
};
use lifeagent_core::app::{ChatTurn, CoreApp, HistoryMessage, TodayView};
use lifeagent_core::goals::WeightGoal;
use lifeagent_core::memory::MemoryNote;
use lifeagent_core::plan::StoredPlan;
use lifeagent_core::profile::Profile;
use lifeagent_core::usage::RoleUsage;
use lifeagent_llm::router::ModelsConfig;
use lifeagent_llm::types::Role;
use lifeagent_plugin_diet::import::{FoodDataError, ImportMessage, ImportReport};
use lifeagent_plugin_diet::sources::SourceInfo;
use lifeagent_plugin_diet::targets::NutritionTargets;
use tauri::ipc::InvokeBody;
use tauri::{AppHandle, State};

use crate::error::{CommandError, ErrorDetail};

#[tauri::command]
pub async fn get_app_state(state: State<'_, CoreApp>) -> Result<AppState, CommandError> {
    ops::get_app_state(&state).await
}

#[tauri::command]
pub async fn complete_onboarding(state: State<'_, CoreApp>) -> Result<(), CommandError> {
    ops::complete_onboarding(&state).await
}

// ---- Chat --------------------------------------------------------------------------------

#[tauri::command]
pub async fn send_message(
    state: State<'_, CoreApp>,
    text: String,
) -> Result<ChatTurn, CommandError> {
    ops::send_message(&state, &text).await
}

#[tauri::command]
pub async fn get_history(
    state: State<'_, CoreApp>,
    limit: u32,
) -> Result<Vec<HistoryMessage>, CommandError> {
    ops::get_history(&state, limit).await
}

// ---- Today / plan --------------------------------------------------------------------------

#[tauri::command]
pub async fn get_today(state: State<'_, CoreApp>) -> Result<TodayView, CommandError> {
    ops::get_today(&state).await
}

#[tauri::command]
pub async fn get_plan(
    state: State<'_, CoreApp>,
    date: String,
) -> Result<Option<StoredPlan>, CommandError> {
    ops::get_plan(&state, &date).await
}

#[tauri::command]
pub async fn generate_plan(
    state: State<'_, CoreApp>,
    date: Option<String>,
) -> Result<StoredPlan, CommandError> {
    ops::generate_plan(&state, date).await
}

#[tauri::command]
pub async fn accept_plan(state: State<'_, CoreApp>, date: String) -> Result<(), CommandError> {
    ops::accept_plan(&state, &date).await
}

#[tauri::command]
pub async fn delete_entry(state: State<'_, CoreApp>, id: String) -> Result<(), CommandError> {
    ops::delete_entry(&state, &id).await
}

// ---- Profile / goals / weight ---------------------------------------------------------------

#[tauri::command]
pub async fn get_profile(state: State<'_, CoreApp>) -> Result<Option<Profile>, CommandError> {
    ops::get_profile(&state).await
}

#[tauri::command]
pub async fn save_profile(state: State<'_, CoreApp>, profile: Profile) -> Result<(), CommandError> {
    ops::save_profile(&state, &profile).await
}

#[tauri::command]
pub async fn get_weight_goal(
    state: State<'_, CoreApp>,
) -> Result<Option<WeightGoal>, CommandError> {
    ops::get_weight_goal(&state).await
}

#[tauri::command]
pub async fn save_weight_goal(
    state: State<'_, CoreApp>,
    goal: WeightGoal,
) -> Result<(), CommandError> {
    ops::save_weight_goal(&state, &goal).await
}

#[tauri::command]
pub async fn log_weight(state: State<'_, CoreApp>, kg: f64) -> Result<(), CommandError> {
    ops::log_weight(&state, kg).await
}

#[tauri::command]
pub async fn preview_targets(
    state: State<'_, CoreApp>,
    profile: Profile,
    weight_kg: f64,
    goal: Option<WeightGoal>,
) -> Result<NutritionTargets, CommandError> {
    ops::preview_targets(&state, &profile, weight_kg, goal.as_ref()).await
}

// ---- Settings -------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_settings(state: State<'_, CoreApp>) -> Result<SettingsDto, CommandError> {
    ops::get_settings(&state).await
}

#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, CoreApp>,
    settings: SettingsDto,
) -> Result<(), CommandError> {
    ops::save_settings(&state, &settings).await?;
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
    ops::get_models(&state).await
}

#[tauri::command]
pub async fn save_models(
    state: State<'_, CoreApp>,
    models: ModelsConfig,
) -> Result<(), CommandError> {
    ops::save_models(&state, &models).await
}

#[tauri::command]
pub async fn set_api_key(
    state: State<'_, CoreApp>,
    provider_id: String,
    key: String,
) -> Result<(), CommandError> {
    ops::set_api_key(&state, &provider_id, &key).await
}

#[tauri::command]
pub async fn has_api_key(
    state: State<'_, CoreApp>,
    provider_id: String,
) -> Result<bool, CommandError> {
    ops::has_api_key(&state, &provider_id).await
}

#[tauri::command]
pub async fn test_model(
    state: State<'_, CoreApp>,
    role: Role,
) -> Result<TestModelResult, CommandError> {
    ops::test_model(&state, role).await
}

// ---- Memory ---------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_memory(
    state: State<'_, CoreApp>,
    limit: u32,
) -> Result<Vec<MemoryNote>, CommandError> {
    ops::get_memory(&state, limit).await
}

#[tauri::command]
pub async fn delete_memory(state: State<'_, CoreApp>, id: String) -> Result<(), CommandError> {
    ops::delete_memory(&state, &id).await
}

// ---- Usage ----------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_usage_summary(state: State<'_, CoreApp>) -> Result<Vec<RoleUsage>, CommandError> {
    ops::get_usage_summary(&state).await
}

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md` §8.2, extended by
// `docs/DISH_NUTRITION.md` §8.2 with dishes) --------------------------------------------------

#[tauri::command]
pub async fn list_food_sources(state: State<'_, CoreApp>) -> Result<Vec<SourceInfo>, CommandError> {
    ops::list_food_sources(&state).await
}

#[tauri::command]
pub async fn list_foods(
    state: State<'_, CoreApp>,
    source_id: Option<String>,
    query: Option<String>,
    limit: u32,
    kind: Option<FoodsKind>,
    brand: Option<String>,
) -> Result<FoodOrDishList, CommandError> {
    let args = ListFoods {
        source_id,
        query,
        limit,
        kind,
        brand,
    };
    ops::list_foods(&state, &args).await
}

#[tauri::command]
pub async fn reorder_food_sources(
    state: State<'_, CoreApp>,
    ids: Vec<String>,
) -> Result<(), CommandError> {
    ops::reorder_food_sources(&state, &ids).await
}

#[tauri::command]
pub async fn set_food_source_enabled(
    state: State<'_, CoreApp>,
    id: String,
    enabled: bool,
) -> Result<(), CommandError> {
    ops::set_food_source_enabled(&state, &id, enabled).await
}

#[tauri::command]
pub async fn rename_food_source(
    state: State<'_, CoreApp>,
    id: String,
    name: String,
) -> Result<(), CommandError> {
    ops::rename_food_source(&state, &id, &name).await
}

#[tauri::command]
pub async fn delete_food_source(state: State<'_, CoreApp>, id: String) -> Result<(), CommandError> {
    ops::delete_food_source(&state, &id).await
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
        InvokeBody::Raw(bytes) => bytes,
        InvokeBody::Json(_) => {
            return Err(FoodDataError::Invalid(ImportMessage::MissingFileContent).into())
        }
    };
    let file_name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(ops::percent_decode)
        .unwrap_or_else(|| "import".to_string());
    ops::preview_food_import(&state, &pending, &file_name, bytes).await
}

#[tauri::command]
pub async fn commit_food_import(
    state: State<'_, CoreApp>,
    pending: State<'_, PendingImportState>,
    token: String,
    name: Option<String>,
    replace_source_id: Option<String>,
) -> Result<ImportReport, CommandError> {
    ops::commit_food_import(&state, &pending, &token, name, replace_source_id).await
}

#[tauri::command]
pub async fn cancel_food_import(
    pending: State<'_, PendingImportState>,
    token: String,
) -> Result<(), CommandError> {
    ops::cancel_food_import(&pending, &token);
    Ok(())
}

// ---- UI language ----------------------------------------------------------------------------

/// The webview reports its UI language (`"ja"`, `"en"` or `"zh-CN"`) so the tray menu and
/// notifications match it (`locale.rs`).
#[tauri::command]
pub fn set_locale(app: AppHandle, locale: String) -> Result<(), CommandError> {
    let Some(locale) = crate::locale::Locale::from_tag(&locale) else {
        return Err(CommandError::new(
            "invalid_input",
            ErrorDetail::InvalidInput,
        ));
    };
    crate::locale::set(locale);
    crate::tray::apply_locale(&app);
    Ok(())
}
