//! The command API: `POST /api/{command}` with the command's arguments as a JSON object
//! (snake_case keys), answered with the command's JSON result, or an `ApiError` body. Each
//! command calls one `lifeagent_api::ops` function.
//!
//! Left out on purpose: `get_models`, `save_models`, `set_api_key`, `has_api_key` and
//! `test_model`. In the web build the operator configures models for everyone
//! (`ServerConfig::models`); users can't see or change them.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream};
use lifeagent_api::events::payload_for;
use lifeagent_api::ops::{self, ListFoods, SettingsDto};
use lifeagent_api::ErrorDetail;
use lifeagent_core::goals::WeightGoal;
use lifeagent_core::profile::Profile;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::AppState;

fn args<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    let json: &[u8] = if body.is_empty() { b"{}" } else { body };
    serde_json::from_slice(json).map_err(|e| {
        tracing::debug!(error = %e, "invalid command arguments");
        ApiError::invalid(ErrorDetail::BadRequest)
    })
}

fn reply<T: Serialize>(
    result: Result<T, lifeagent_api::CommandError>,
) -> Result<Response, ApiError> {
    Ok(Json(result?).into_response())
}

#[derive(Deserialize)]
struct Text {
    text: String,
}
#[derive(Deserialize)]
struct Limit {
    limit: u32,
}
#[derive(Deserialize)]
struct Date {
    date: String,
}
#[derive(Deserialize)]
struct OptionalDate {
    #[serde(default)]
    date: Option<String>,
}
#[derive(Deserialize)]
struct Id {
    id: String,
}
#[derive(Deserialize)]
struct ProfileArg {
    profile: Profile,
}
#[derive(Deserialize)]
struct GoalArg {
    goal: WeightGoal,
}
#[derive(Deserialize)]
struct Kg {
    kg: f64,
}
#[derive(Deserialize)]
struct PreviewTargets {
    profile: Profile,
    weight_kg: f64,
    #[serde(default)]
    goal: Option<WeightGoal>,
}
#[derive(Deserialize)]
struct SettingsArg {
    settings: SettingsDto,
}
#[derive(Deserialize)]
struct Ids {
    ids: Vec<String>,
}
#[derive(Deserialize)]
struct Enabled {
    id: String,
    enabled: bool,
}
#[derive(Deserialize)]
struct Rename {
    id: String,
    name: String,
}
#[derive(Deserialize)]
struct Token {
    token: String,
}
#[derive(Deserialize)]
struct CommitImport {
    token: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    replace_source_id: Option<String>,
}

pub async fn command(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(command): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let tenant = state.tenants.get(&user.schema).await?;
    let app = &tenant.app;
    match command.as_str() {
        "get_app_state" => reply(ops::get_app_state(app).await),
        "complete_onboarding" => reply(ops::complete_onboarding(app).await),
        "send_message" => {
            let a: Text = args(&body)?;
            reply(ops::send_message(app, &a.text).await)
        }
        "get_history" => {
            let a: Limit = args(&body)?;
            reply(ops::get_history(app, a.limit).await)
        }
        "get_today" => reply(ops::get_today(app).await),
        "get_plan" => {
            let a: Date = args(&body)?;
            reply(ops::get_plan(app, &a.date).await)
        }
        "generate_plan" => {
            let a: OptionalDate = args(&body)?;
            reply(ops::generate_plan(app, a.date).await)
        }
        "accept_plan" => {
            let a: Date = args(&body)?;
            reply(ops::accept_plan(app, &a.date).await)
        }
        "delete_entry" => {
            let a: Id = args(&body)?;
            reply(ops::delete_entry(app, &a.id).await)
        }
        "get_profile" => reply(ops::get_profile(app).await),
        "save_profile" => {
            let a: ProfileArg = args(&body)?;
            reply(ops::save_profile(app, &a.profile).await)
        }
        "get_weight_goal" => reply(ops::get_weight_goal(app).await),
        "save_weight_goal" => {
            let a: GoalArg = args(&body)?;
            reply(ops::save_weight_goal(app, &a.goal).await)
        }
        "log_weight" => {
            let a: Kg = args(&body)?;
            reply(ops::log_weight(app, a.kg).await)
        }
        "preview_targets" => {
            let a: PreviewTargets = args(&body)?;
            reply(ops::preview_targets(app, &a.profile, a.weight_kg, a.goal.as_ref()).await)
        }
        "get_settings" => reply(ops::get_settings(app).await),
        "save_settings" => {
            let a: SettingsArg = args(&body)?;
            reply(ops::save_settings(app, &a.settings).await)
        }
        "get_memory" => {
            let a: Limit = args(&body)?;
            reply(ops::get_memory(app, a.limit).await)
        }
        "delete_memory" => {
            let a: Id = args(&body)?;
            reply(ops::delete_memory(app, &a.id).await)
        }
        "get_usage_summary" => reply(ops::get_usage_summary(app).await),
        "list_food_sources" => reply(ops::list_food_sources(app).await),
        "list_foods" => {
            let a: ListFoods = args(&body)?;
            reply(ops::list_foods(app, &a).await)
        }
        "reorder_food_sources" => {
            let a: Ids = args(&body)?;
            reply(ops::reorder_food_sources(app, &a.ids).await)
        }
        "set_food_source_enabled" => {
            let a: Enabled = args(&body)?;
            reply(ops::set_food_source_enabled(app, &a.id, a.enabled).await)
        }
        "rename_food_source" => {
            let a: Rename = args(&body)?;
            reply(ops::rename_food_source(app, &a.id, &a.name).await)
        }
        "delete_food_source" => {
            let a: Id = args(&body)?;
            reply(ops::delete_food_source(app, &a.id).await)
        }
        "commit_food_import" => {
            let a: CommitImport = args(&body)?;
            reply(
                ops::commit_food_import(
                    app,
                    &tenant.imports,
                    &a.token,
                    a.name,
                    a.replace_source_id,
                )
                .await,
            )
        }
        "cancel_food_import" => {
            let a: Token = args(&body)?;
            ops::cancel_food_import(&tenant.imports, &a.token);
            reply(Ok(()))
        }
        _ => Err(ApiError::new("not_found", ErrorDetail::UnknownEndpoint)),
    }
}

/// The file's raw bytes as the body, its name url-encoded in `x-file-name`: the same contract
/// as the desktop's raw IPC body (§8.2 "Getting the file to Rust").
pub async fn preview_food_import(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let tenant = state.tenants.get(&user.schema).await?;
    let file_name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(ops::percent_decode)
        .unwrap_or_else(|| "import".to_string());
    reply(ops::preview_food_import(&tenant.app, &tenant.imports, &file_name, &body).await)
}

/// The user's `AppEvent`s as Server-Sent Events, with the names and payloads of
/// `lifeagent_api::events`. Holding the tenant keeps it cached while
/// the stream is open, so changes made by this user's other requests reach it.
pub async fn events(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let tenant = state.tenants.get(&user.schema).await?;
    let rx = tenant.app.subscribe();
    let stream = stream::unfold((rx, tenant), |(mut rx, tenant)| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let (name, payload) = payload_for(&event);
                    let sse = Event::default().event(name).data(payload.to_string());
                    return Some((Ok(sse), (rx, tenant)));
                }
                // The UI refetches on whichever event does arrive, so skipped ones are only a
                // delay, as on the desktop.
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "event stream lagged; some AppEvents were skipped");
                }
                Err(RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20))))
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TickReport {
    pub users: usize,
    pub failed: usize,
}

/// Users ticked at once: each tick may call the planner model, and each open tenant holds up
/// to two DB connections.
const TICK_CONCURRENCY: usize = 4;

/// Runs one scheduler tick (§7.13) for every user: what the desktop does once a minute, for
/// all accounts. Called by Cloud Scheduler through `/api/internal/tick`, or by the in-process
/// loop in development.
pub async fn tick_all(state: &Arc<AppState>) -> Result<TickReport, ApiError> {
    let schemas: Vec<String> = sqlx::query_scalar("SELECT schema_name FROM users ORDER BY id")
        .fetch_all(&state.auth_db)
        .await?;
    let permits = Arc::new(Semaphore::new(TICK_CONCURRENCY));
    let mut tasks = JoinSet::new();
    for schema in &schemas {
        let state = state.clone();
        let permits = permits.clone();
        let schema = schema.clone();
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await;
            match state.tenants.get_quietly(&schema).await {
                Ok(tenant) => {
                    tenant.scheduler.lock().await.tick().await;
                    true
                }
                Err(e) => {
                    tracing::error!(schema, error = %e, "tick: could not open tenant");
                    false
                }
            }
        });
    }
    let mut report = TickReport {
        users: schemas.len(),
        failed: 0,
    };
    while let Some(result) = tasks.join_next().await {
        if !matches!(result, Ok(true)) {
            report.failed += 1;
        }
    }
    Ok(report)
}

/// Where the tick secret may be sent. Cloud Scheduler sets custom headers on HTTP targets;
/// `Authorization: Bearer` is accepted too, for curl.
pub const TICK_TOKEN_HEADER: &str = "x-lifeagent-tick-token";

/// `X-LifeAgent-Tick-Token: <LIFEAGENT_TICK_TOKEN>` (or `Authorization: Bearer ...`). The
/// service is public (the app does its own login), so this endpoint checks a shared secret
/// itself rather than relying on Cloud Run IAM. A shared secret instead of verifying Cloud
/// Scheduler's OIDC token keeps JWT validation out of the hackathon build (`docs/DECISIONS.md`).
pub async fn internal_tick(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<TickReport>, ApiError> {
    let Some(expected) = state.config.tick_token.as_deref() else {
        return Err(ApiError::new("not_found", ErrorDetail::UnknownEndpoint));
    };
    let given = headers
        .get(TICK_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .unwrap_or_default();
    if !constant_time_eq(given.as_bytes(), expected.as_bytes()) {
        return Err(ApiError::unauthorized());
    }
    Ok(Json(tick_all(&state).await?))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn unknown_api() -> ApiError {
    ApiError::new("not_found", ErrorDetail::UnknownEndpoint)
}

pub async fn health() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_compares_whole_values() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn empty_bodies_parse_as_empty_argument_objects() {
        let a: OptionalDate = args(&Bytes::new()).unwrap();
        assert_eq!(a.date, None);
        assert!(args::<Text>(&Bytes::from_static(b"{}")).is_err());
        assert!(args::<Text>(&Bytes::from_static(b"not json")).is_err());
    }
}
