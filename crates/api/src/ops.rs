//! One function per command of `docs/PHASE1_SPEC.md` §10.2 (plus the nutrition-source and
//! import commands of `docs/NUTRITION_SOURCES.md` §8.2 / `docs/DISH_NUTRITION.md` §8.2).
//!
//! Every function is a thin wrapper: call an existing `CoreApp`/service method, map the error
//! through `CommandError`. No business logic lives here beyond the odd default (e.g.
//! `generate_plan`'s date) already established by the CLI in M6. Served over HTTP by
//! `lifeagent-server` (`docs/DECISIONS.md`, "Web server").

use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, NaiveDate, Utc};
use lifeagent_core::app::{ChatTurn, CoreApp, HistoryMessage, PluginInfo, TodayView};
use lifeagent_core::error::CoreError;
use lifeagent_core::goals::WeightGoal;
use lifeagent_core::i18n::Language;
use lifeagent_core::plan::{PlanTrigger, StoredPlan};
use lifeagent_core::preferences::{NewPreference, Preference};
use lifeagent_core::profile::Profile;
use lifeagent_core::time::{age_years, format_hhmm, parse_hhmm};
use lifeagent_core::usage::RoleUsage;
use lifeagent_plugin_diet::import::{
    self, FoodDataError, ImportMessage, ImportPreview, ImportReport, ImportTarget, ParsedImport,
};
use lifeagent_plugin_diet::model::Dish;
use lifeagent_plugin_diet::sources::{self, Food, SourceInfo};
use lifeagent_plugin_diet::targets::{self, NutritionTargets};
use lifeagent_plugin_multimember::model::{DefaultSlot, Dislike, MealType, Member, MemberProfile};
use lifeagent_plugin_multimember::store as members;
use lifeagent_plugin_multimember::view::{self as members_view, MembersView};
use lifeagent_plugin_stock::model::{StockItem, Unit};
use lifeagent_plugin_stock::settings::StockSettings;
use lifeagent_plugin_stock::shopping::{ShoppingLineRef, ShoppingStatus};
use lifeagent_plugin_stock::store::{self as stock_store, NewStock};
use lifeagent_plugin_stock::view::{self as stock_view, StockView};
use lifeagent_plugin_training::prefs::{self as training_prefs, TrainingPreferences};
use lifeagent_plugin_training::view::{self as training_view, TrainingView};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    pub onboarding_complete: bool,
    pub models_configured: bool,
    pub logical_date: NaiveDate,
    /// Ids of the plugins switched on, so the shell shows only their pages.
    pub enabled_plugins: Vec<String>,
}

pub async fn get_app_state(app: &CoreApp) -> Result<AppState, CommandError> {
    Ok(AppState {
        onboarding_complete: app.settings().onboarding_complete().await?,
        models_configured: app.models_configured(),
        logical_date: app.logical_today().await?,
        enabled_plugins: app
            .enabled_plugins()
            .await?
            .iter()
            .map(|p| p.manifest().id.to_string())
            .collect(),
    })
}

// ---- Plugins --------------------------------------------------------------------------------

pub async fn get_plugins(app: &CoreApp) -> Result<Vec<PluginInfo>, CommandError> {
    Ok(app.plugins().await?)
}

pub async fn set_plugin_enabled(
    app: &CoreApp,
    id: &str,
    enabled: bool,
) -> Result<(), CommandError> {
    Ok(app.set_plugin_enabled(id, enabled).await?)
}

// ---- Training (`lifeagent-plugin-training`) -------------------------------------------------

pub async fn get_training_view(app: &CoreApp) -> Result<TrainingView, CommandError> {
    Ok(training_view::training_view(&app.plugin_context()).await?)
}

pub async fn save_training_preferences(
    app: &CoreApp,
    preferences: &TrainingPreferences,
) -> Result<(), CommandError> {
    Ok(training_prefs::save(&app.plugin_context(), preferences).await?)
}

/// `date` defaults to tomorrow, like `generate_plan`'s. Blank `text` clears the request.
pub async fn save_training_day_preference(
    app: &CoreApp,
    date: Option<String>,
    text: &str,
) -> Result<(), CommandError> {
    let date = resolve_plan_date(app, date).await?;
    Ok(training_prefs::set_day_preference(&app.plugin_context(), date, text).await?)
}

// ---- Members (`lifeagent-plugin-multimember`) --------------------------------------------------

pub async fn get_members_view(app: &CoreApp) -> Result<MembersView, CommandError> {
    Ok(members_view::members_view(&app.plugin_context()).await?)
}

/// `attendance` gives the new member's weekly defaults; `None`: dinner every day.
pub async fn add_member(
    app: &CoreApp,
    profile: &MemberProfile,
    attendance: Option<&[DefaultSlot]>,
) -> Result<Member, CommandError> {
    Ok(members::add_member(&app.plugin_context(), profile, attendance).await?)
}

pub async fn update_member(
    app: &CoreApp,
    id: &str,
    profile: &MemberProfile,
) -> Result<Member, CommandError> {
    Ok(members::update_member(&app.plugin_context(), id, profile).await?)
}

pub async fn archive_member(app: &CoreApp, id: &str) -> Result<(), CommandError> {
    Ok(members::archive_member(&app.plugin_context(), id).await?)
}

pub async fn set_attendance_default(
    app: &CoreApp,
    member_id: &str,
    slot: &DefaultSlot,
) -> Result<(), CommandError> {
    Ok(members::set_default(&app.plugin_context(), member_id, slot).await?)
}

pub async fn set_attendance_override(
    app: &CoreApp,
    member_id: &str,
    date: &str,
    meal_type: MealType,
    present: bool,
    note: Option<&str>,
) -> Result<(), CommandError> {
    let date = parse_date(date)?;
    Ok(members::set_override(
        &app.plugin_context(),
        member_id,
        date,
        meal_type,
        present,
        note,
    )
    .await?)
}

pub async fn clear_attendance_override(
    app: &CoreApp,
    member_id: &str,
    date: &str,
    meal_type: MealType,
) -> Result<(), CommandError> {
    let date = parse_date(date)?;
    Ok(members::clear_override(&app.plugin_context(), member_id, date, meal_type).await?)
}

pub async fn add_member_dislike(
    app: &CoreApp,
    member_id: &str,
    text: &str,
    valid_from: Option<&str>,
    valid_until: Option<&str>,
) -> Result<Dislike, CommandError> {
    let valid_from = valid_from.map(parse_date).transpose()?;
    let valid_until = valid_until.map(parse_date).transpose()?;
    Ok(members::add_dislike(
        &app.plugin_context(),
        member_id,
        text,
        valid_from,
        valid_until,
    )
    .await?)
}

pub async fn remove_member_dislike(app: &CoreApp, id: &str) -> Result<(), CommandError> {
    Ok(members::remove_dislike(&app.plugin_context(), id).await?)
}

pub async fn complete_onboarding(app: &CoreApp) -> Result<(), CommandError> {
    app.settings()
        .set_onboarding_complete(true, app.clock().now())
        .await?;
    Ok(())
}

// ---- Chat --------------------------------------------------------------------------------

pub async fn send_message(app: &CoreApp, text: &str) -> Result<ChatTurn, CommandError> {
    Ok(app.send_message(text).await?)
}

pub async fn get_history(app: &CoreApp, limit: u32) -> Result<Vec<HistoryMessage>, CommandError> {
    Ok(app.get_history(limit).await?)
}

// ---- Today / plan --------------------------------------------------------------------------

pub async fn get_today(app: &CoreApp) -> Result<TodayView, CommandError> {
    Ok(app.today_view().await?)
}

pub async fn get_plan(app: &CoreApp, date: &str) -> Result<Option<StoredPlan>, CommandError> {
    let date = parse_date(date)?;
    Ok(app.get_plan(date).await?)
}

/// `request` is the user's optional wish for this plan only (the Nutrition tab's request box).
pub async fn generate_plan(
    app: &CoreApp,
    date: Option<String>,
    request: Option<String>,
) -> Result<StoredPlan, CommandError> {
    let date = resolve_plan_date(app, date).await?;
    Ok(app
        .generate_plan(date, PlanTrigger::Manual, request.as_deref())
        .await?)
}

/// Revises the current plan for `date` following `instruction` (the Nutrition tab's request
/// box with Adjust). Blank is `instruction_empty`; no plan for `date` is `not_found`.
pub async fn revise_plan(
    app: &CoreApp,
    date: &str,
    instruction: &str,
) -> Result<StoredPlan, CommandError> {
    let date = parse_date(date)?;
    Ok(app.revise_plan(date, instruction).await?)
}

/// Whether a plan generation is under way, for a page opened mid-way; `plan-generating`
/// events report the changes after that.
pub fn is_plan_generating(app: &CoreApp) -> bool {
    app.plan_generating()
}

pub async fn accept_plan(app: &CoreApp, date: &str) -> Result<(), CommandError> {
    let date = parse_date(date)?;
    Ok(app.accept_plan(date).await?)
}

pub async fn delete_entry(app: &CoreApp, id: &str) -> Result<(), CommandError> {
    Ok(app.delete_entry(id).await?)
}

// ---- Profile / goals / weight ---------------------------------------------------------------

pub async fn get_profile(app: &CoreApp) -> Result<Option<Profile>, CommandError> {
    Ok(app.profile().get().await?)
}

pub async fn save_profile(app: &CoreApp, profile: &Profile) -> Result<(), CommandError> {
    Ok(app.profile().set(profile).await?)
}

pub async fn get_weight_goal(app: &CoreApp) -> Result<Option<WeightGoal>, CommandError> {
    Ok(app.goals().get_active_weight_goal().await?)
}

pub async fn save_weight_goal(app: &CoreApp, goal: &WeightGoal) -> Result<(), CommandError> {
    Ok(app.save_weight_goal(goal).await?)
}

pub async fn log_weight(app: &CoreApp, kg: f64) -> Result<(), CommandError> {
    Ok(app.log_weight(kg).await?)
}

/// Pure computation (no DB write): calls `targets::compute` directly rather than going through
/// `CoreApp`, per the spec's own note for this command.
pub async fn preview_targets(
    app: &CoreApp,
    profile: &Profile,
    weight_kg: f64,
    goal: Option<&WeightGoal>,
) -> Result<NutritionTargets, CommandError> {
    let today = app.logical_today().await?;
    let age = age_years(profile.birth_date, today);
    Ok(targets::compute(profile, weight_kg, age, goal))
}

// ---- Settings -------------------------------------------------------------------------------

/// All of §7.2's settings keys except `models` (its own `get_models`/`save_models` pair) and
/// `onboarding_complete` (its own `complete_onboarding`, which only ever sets it to `true` — the
/// Settings page never exposes a way to un-complete onboarding, so it isn't part of this
/// general-purpose settings form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsDto {
    pub plan_time: String,
    pub day_boundary: String,
}

pub async fn get_settings(app: &CoreApp) -> Result<SettingsDto, CommandError> {
    let settings = app.settings();
    Ok(SettingsDto {
        plan_time: format_hhmm(settings.plan_time().await?),
        day_boundary: format_hhmm(settings.day_boundary().await?),
    })
}

pub async fn save_settings(app: &CoreApp, settings: &SettingsDto) -> Result<(), CommandError> {
    let now = app.clock().now();
    let plan_time = parse_hhmm(&settings.plan_time)?;
    let day_boundary = parse_hhmm(&settings.day_boundary)?;
    app.settings()
        .set_plan_time_and_day_boundary(plan_time, day_boundary, now)
        .await?;
    Ok(())
}

/// Stores the UI's locale as the language the assistant replies in. The web app sends it on
/// every load, so changing the UI language also changes the assistant's.
pub async fn set_language(app: &CoreApp, language: Language) -> Result<(), CommandError> {
    let now = app.clock().now();
    app.settings().set_language(language, now).await?;
    Ok(())
}

// ---- Preferences ----------------------------------------------------------------------------

pub async fn get_preferences(app: &CoreApp, limit: u32) -> Result<Vec<Preference>, CommandError> {
    Ok(app.preferences().list(limit).await?)
}

pub async fn add_preference(
    app: &CoreApp,
    preference: &NewPreference,
) -> Result<Preference, CommandError> {
    let today = app.plugin_context().today().await?;
    Ok(app.preferences().add(preference, today).await?)
}

pub async fn delete_preference(app: &CoreApp, id: &str) -> Result<(), CommandError> {
    Ok(app.preferences().remove(id).await?)
}

// ---- Usage ----------------------------------------------------------------------------------

pub async fn get_usage_summary(app: &CoreApp) -> Result<Vec<RoleUsage>, CommandError> {
    Ok(app.usage_summary().await?)
}

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md` §8.2, extended by
// `docs/DISH_NUTRITION.md` §8.2 with dishes) --------------------------------------------------

pub async fn list_food_sources(app: &CoreApp) -> Result<Vec<SourceInfo>, CommandError> {
    Ok(sources::list_sources(&app.plugin_context()).await?)
}

/// Which array `list_foods` returns; defaults to `Food` when omitted.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoodsKind {
    Food,
    Dish,
}

/// `#[serde(untagged)]`: serializes as a plain `Food[]` or `Dish[]` — the caller already knows
/// which it asked for via `kind`, so no extra discriminant field is needed on each item.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum FoodOrDishList {
    Foods(Vec<Food>),
    Dishes(Vec<Dish>),
}

/// `list_foods`'s arguments. `query` is a food-only substring filter; `brand` is a dish-only
/// exact filter (`docs/DISH_NUTRITION.md` §8.1/§8.2 — the same split the CLI's `foods list`
/// makes).
#[derive(Debug, Clone, Deserialize)]
pub struct ListFoods {
    pub source_id: Option<String>,
    pub query: Option<String>,
    pub limit: u32,
    pub kind: Option<FoodsKind>,
    pub brand: Option<String>,
}

pub async fn list_foods(app: &CoreApp, args: &ListFoods) -> Result<FoodOrDishList, CommandError> {
    let ctx = app.plugin_context();
    Ok(match args.kind {
        Some(FoodsKind::Dish) => FoodOrDishList::Dishes(
            sources::list_dishes(
                &ctx,
                args.source_id.as_deref(),
                args.brand.as_deref(),
                args.limit,
            )
            .await?,
        ),
        None | Some(FoodsKind::Food) => FoodOrDishList::Foods(
            sources::list_foods(
                &ctx,
                args.source_id.as_deref(),
                args.query.as_deref(),
                args.limit,
            )
            .await?,
        ),
    })
}

pub async fn reorder_food_sources(app: &CoreApp, ids: &[String]) -> Result<(), CommandError> {
    Ok(sources::reorder_datasets(&app.plugin_context(), ids).await?)
}

pub async fn set_food_source_enabled(
    app: &CoreApp,
    id: &str,
    enabled: bool,
) -> Result<(), CommandError> {
    Ok(sources::set_enabled(&app.plugin_context(), id, enabled).await?)
}

pub async fn rename_food_source(app: &CoreApp, id: &str, name: &str) -> Result<(), CommandError> {
    Ok(sources::rename_source(&app.plugin_context(), id, name).await?)
}

pub async fn delete_food_source(app: &CoreApp, id: &str) -> Result<(), CommandError> {
    Ok(sources::delete_source(&app.plugin_context(), id).await?)
}

// ---- Stock (`lifeagent-plugin-stock`, the Stock tab) ----------------------------------------

pub async fn get_stock(app: &CoreApp) -> Result<StockView, CommandError> {
    Ok(stock_view::stock_view(&app.plugin_context()).await?)
}

/// `add_stock_item`'s argument. `expires_on` is `YYYY-MM-DD`; empty or omitted means no date.
#[derive(Debug, Clone, Deserialize)]
pub struct NewStockDto {
    pub name: String,
    pub quantity: f64,
    pub unit: Unit,
    #[serde(default)]
    pub expires_on: Option<String>,
}

pub async fn add_stock_item(app: &CoreApp, item: &NewStockDto) -> Result<StockItem, CommandError> {
    let expires_on = match item.expires_on.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => Some(parse_date(s)?),
    };
    let new = NewStock {
        name: item.name.clone(),
        quantity: item.quantity,
        unit: item.unit,
        expires_on,
    };
    Ok(stock_store::add(&app.plugin_context(), &new).await?)
}

pub async fn set_stock_quantity(
    app: &CoreApp,
    id: &str,
    quantity: f64,
) -> Result<StockItem, CommandError> {
    Ok(stock_store::set_quantity(&app.plugin_context(), id, quantity).await?)
}

pub async fn delete_stock_item(app: &CoreApp, id: &str) -> Result<(), CommandError> {
    Ok(stock_store::delete(&app.plugin_context(), id).await?)
}

/// Puts back what one logged meal took out of stock, keeping the meal (the undo in the web
/// app's "taken from stock" notice). A no-op if it was already put back. The meal is never
/// matched against stock again, and deleting it later puts nothing back twice.
pub async fn undo_stock_use(app: &CoreApp, event_id: &str) -> Result<(), CommandError> {
    stock_store::restore(&app.plugin_context(), event_id).await?;
    Ok(())
}

pub async fn save_stock_settings(
    app: &CoreApp,
    settings: &StockSettings,
) -> Result<(), CommandError> {
    let ctx = app.plugin_context();
    Ok(lifeagent_plugin_stock::settings::set(&ctx, settings, app.clock().now()).await?)
}

/// Marks a shopping-list line done (bought: its planned amount goes into stock), cancelled, or
/// open again.
pub async fn set_shopping_status(
    app: &CoreApp,
    line: &ShoppingLineRef,
    status: ShoppingStatus,
) -> Result<(), CommandError> {
    Ok(lifeagent_plugin_stock::shopping::set_status(&app.plugin_context(), line, status).await?)
}

/// A parsed-but-not-yet-committed import, kept between `preview_food_import` and
/// `commit_food_import`/`cancel_food_import` (§8.2 "Preview → commit token"). There is a single
/// slot: a new preview replaces the old one, and an entry expires after
/// [`PENDING_IMPORT_TTL_SECS`] via the injected `Clock`. One per user: the web server keeps one per
/// logged-in user.
struct PendingImportRecord {
    token: Uuid,
    parsed: ParsedImport,
    created_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct PendingImportState(Mutex<Option<PendingImportRecord>>);

impl PendingImportState {
    /// A poisoned lock only means another thread panicked mid-update of a plain `Option`, which
    /// can't leave it half-written, so the value is still usable.
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<PendingImportRecord>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

const PENDING_IMPORT_TTL_SECS: i64 = 15 * 60;

/// Minimal percent-decoding for the `x-file-name` header (`encodeURIComponent` on the frontend).
/// Not a general `Vec<u8>`-first decoder: invalid `%XX` sequences are left as literal text, which
/// only ever means a slightly odd display name, never a parsing failure.
pub fn percent_decode(s: &str) -> String {
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

/// `bytes` is the uploaded file and `file_name` its (already decoded) name; the server reads
/// them from the HTTP request.
pub async fn preview_food_import(
    app: &CoreApp,
    pending: &PendingImportState,
    file_name: &str,
    bytes: &[u8],
) -> Result<ImportPreviewWithToken, CommandError> {
    let parsed = import::parse(file_name, bytes)?;
    let preview = import::preview(&app.plugin_context(), &parsed).await?;

    let token = Uuid::now_v7();
    *pending.slot() = Some(PendingImportRecord {
        token,
        parsed,
        created_at: app.clock().now(),
    });

    Ok(ImportPreviewWithToken {
        preview,
        token: token.to_string(),
    })
}

/// Consumes the pending import if `token` matches and hasn't expired; otherwise an expired/
/// unknown token returns `not_found` with the "re-select the file" message (§8.2).
fn take_pending_import(
    app: &CoreApp,
    pending: &PendingImportState,
    token: &str,
) -> Result<ParsedImport, CommandError> {
    let mut guard = pending.slot();
    let expired = guard
        .as_ref()
        .map(|p| (app.clock().now() - p.created_at).num_seconds() > PENDING_IMPORT_TTL_SECS)
        .unwrap_or(false);
    if expired {
        *guard = None;
    }
    match guard.take() {
        Some(record) if record.token.to_string() == token => Ok(record.parsed),
        Some(record) => {
            // A stale/mismatched request (e.g. a second browser tab) must not drop the import
            // that's still pending for whoever holds the current token.
            *guard = Some(record);
            Err(FoodDataError::NotFound(ImportMessage::ImportExpired).into())
        }
        None => Err(FoodDataError::NotFound(ImportMessage::ImportExpired).into()),
    }
}

pub async fn commit_food_import(
    app: &CoreApp,
    pending: &PendingImportState,
    token: &str,
    name: Option<String>,
    replace_source_id: Option<String>,
) -> Result<ImportReport, CommandError> {
    let parsed = take_pending_import(app, pending, token)?;
    let target = match replace_source_id {
        Some(source_id) => ImportTarget::Replace { source_id },
        None => ImportTarget::New {
            name: name.unwrap_or_else(|| parsed.suggested_name.clone()),
        },
    };
    Ok(import::commit(&app.plugin_context(), parsed, target).await?)
}

pub fn cancel_food_import(pending: &PendingImportState, token: &str) {
    let mut guard = pending.slot();
    if guard.as_ref().is_some_and(|p| p.token.to_string() == token) {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_utf8_and_leaves_bad_escapes() {
        assert_eq!(percent_decode("%E9%A3%9F%E7%89%A9.csv"), "食物.csv");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("100%zz"), "100%zz");
    }
}
