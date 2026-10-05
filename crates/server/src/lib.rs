//! `lifeagent-server`: the HTTP backend of the web build, a composition root next to the CLI
//! (`docs/HACKATHON.md` §1).
//!
//! - `auth`: accounts and cookie sessions in the `auth` schema.
//! - `tenants`: one `CoreApp` per logged-in user, in that user's own schema and role.
//! - `routes`: the command API (`lifeagent_api::ops`), the event
//!   stream and the scheduler tick.
//!
//! Optionally serves the built UI from `LIFEAGENT_STATIC_DIR`, so one container serves both.

pub mod auth;
pub mod config;
pub mod error;
pub mod models;
pub mod routes;
pub mod tenants;

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use lifeagent_core::clock::Clock;
use lifeagent_core::db::validate_identifier;
use lifeagent_core::plugin::Plugin;
use lifeagent_llm::types::ChatModel;
use lifeagent_plugin_diet::DietPlugin;
use lifeagent_plugin_multimember::MultimemberPlugin;
use lifeagent_plugin_stock::StockPlugin;
use lifeagent_plugin_training::TrainingPlugin;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tower_http::services::{ServeDir, ServeFile};

use crate::auth::RateLimiter;
use crate::config::ServerConfig;
use crate::models::{EchoModel, EnvSecretStore};
use crate::tenants::{AppFactory, Tenants};

pub struct AppState {
    pub config: ServerConfig,
    pub clock: Arc<dyn Clock>,
    /// Pool on the `auth` schema, as the server's login role.
    pub auth_db: PgPool,
    pub tenants: Tenants,
    /// Login and registration attempts per client IP.
    pub limiter: RateLimiter<std::net::IpAddr>,
    /// Model-backed commands per user id (`routes::LLM_COMMANDS`).
    pub llm_limiter: RateLimiter<String>,
    dummy_hash: String,
}

/// Uploaded import files (`/api/preview_food_import`). Everything else stays at axum's 2 MB.
const IMPORT_BODY_LIMIT: usize = 10 * 1024 * 1024;
/// How long an inactive user's `CoreApp` stays cached.
const TENANT_MAX_IDLE: Duration = Duration::from_secs(15 * 60);

/// Creates the auth schema if needed and applies its migrations.
async fn open_auth_db(url: &str, schema: &str) -> Result<PgPool> {
    validate_identifier("schema", schema)?;
    let base = PgConnectOptions::from_str(url)?;
    let setup = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(base.clone())
        .await
        .context("connecting to the database")?;
    sqlx::query(AssertSqlSafe(format!(
        "CREATE SCHEMA IF NOT EXISTS \"{schema}\""
    )))
    .execute(&setup)
    .await?;
    setup.close().await;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(base.options([("search_path", format!("\"{schema}\""))]))
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

impl AppState {
    pub async fn new(config: ServerConfig, clock: Arc<dyn Clock>) -> Result<Arc<Self>> {
        let auth_db = open_auth_db(&config.database_url, &config.auth_schema).await?;
        let chat_model_override: Option<Arc<dyn ChatModel>> = if config.mock_llm {
            Some(Arc::new(EchoModel))
        } else {
            None
        };
        if chat_model_override.is_none() && config.models.is_none() {
            tracing::warn!(
                "no LIFEAGENT_MODELS and no LIFEAGENT_MOCK_LLM: chat will report not configured"
            );
        }
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(DietPlugin),
            Arc::new(StockPlugin),
            Arc::new(TrainingPlugin),
            Arc::new(MultimemberPlugin),
        ];
        let tenants = Tenants::new(AppFactory {
            database_url: config.database_url.clone(),
            clock: clock.clone(),
            secrets: Arc::new(EnvSecretStore),
            plugins,
            models: config.models.clone(),
            chat_model_override,
            daily_token_budget: config.daily_token_budget,
        });
        let dummy_hash = auth::dummy_hash()
            .await
            .map_err(|e| anyhow::anyhow!("hashing the dummy password: {:?}", e.0))?;
        Ok(Arc::new(Self {
            config,
            clock,
            auth_db,
            tenants,
            limiter: RateLimiter::new(auth::ATTEMPTS_PER_WINDOW, auth::ATTEMPT_WINDOW),
            llm_limiter: RateLimiter::new(
                routes::LLM_REQUESTS_PER_WINDOW,
                routes::LLM_REQUEST_WINDOW,
            ),
            dummy_hash,
        }))
    }

    /// Idle-tenant eviction, and the in-process scheduler loop if configured. Runs for the
    /// life of the process.
    pub fn spawn_background(self: &Arc<Self>) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let evicted = state.tenants.evict_idle(TENANT_MAX_IDLE);
                if evicted > 0 {
                    tracing::info!(
                        evicted,
                        cached = state.tenants.len(),
                        "evicted idle tenants"
                    );
                }
            }
        });
        if self.config.inprocess_scheduler {
            let state = self.clone();
            tokio::spawn(async move {
                loop {
                    if let Err(e) = routes::tick_all(&state).await {
                        tracing::warn!(error = ?e.0, "in-process scheduler tick failed");
                    }
                    tokio::time::sleep(lifeagent_core::scheduler::TICK_INTERVAL).await;
                }
            });
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/delete_account", post(auth::delete_account))
        .route("/events", get(routes::events))
        .route("/internal/tick", post(routes::internal_tick))
        .route(
            "/preview_food_import",
            post(routes::preview_food_import).layer(DefaultBodyLimit::max(IMPORT_BODY_LIMIT)),
        )
        .route("/{command}", post(routes::command))
        // Unknown `/api/...` paths get a JSON 404, never the UI's index.html.
        .fallback(routes::unknown_api);

    let app = Router::new()
        .nest("/api", api)
        // Not `/healthz`: Cloud Run's front end reserves some paths ending in `z` and answers
        // them itself with a 404.
        .route("/health", get(routes::health));
    let app = match &state.config.static_dir {
        // Unknown paths fall back to index.html so the UI's own routes survive a reload.
        Some(dir) => app
            .fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => app,
    };
    app.with_state(state)
}
