//! One `CoreApp` per user, opened on first use and evicted when idle (`docs/HACKATHON.md` §1,
//! "Connections").
//!
//! Each app runs in its user's tenant (`DbConfig::tenant`: own schema, own role, a pool of at
//! most 2 connections). Keeping one app per user per server instance matters beyond cost: the
//! app's event bus is in memory, so a live `/api/events` stream only sees changes made through
//! the same `CoreApp`. An entry is therefore only evicted when nothing but the cache holds it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lifeagent_api::ops::PendingImportState;
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::Clock;
use lifeagent_core::db::DbConfig;
use lifeagent_core::error::CoreError;
use lifeagent_core::plugin::Plugin;
use lifeagent_core::scheduler::Scheduler;
use lifeagent_core::secrets::SecretStore;
use lifeagent_llm::router::ModelsConfig;
use lifeagent_llm::types::ChatModel;
use tokio::sync::OnceCell;

/// Everything the server keeps for one user while they're active.
pub struct Tenant {
    pub app: CoreApp,
    /// The pending batch import between preview and commit (`ops::preview_food_import`).
    pub imports: PendingImportState,
    /// Per-user plan-generation retry state across ticks (in memory only, §7.13).
    pub scheduler: tokio::sync::Mutex<Scheduler>,
    last_used: Mutex<Instant>,
}

impl Tenant {
    fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }
}

/// How each user's `CoreApp` is built: shared by all users except the tenant name.
pub struct AppFactory {
    pub database_url: String,
    pub clock: Arc<dyn Clock>,
    pub secrets: Arc<dyn SecretStore>,
    pub plugins: Vec<Arc<dyn Plugin>>,
    pub models: Option<ModelsConfig>,
    pub chat_model_override: Option<Arc<dyn ChatModel>>,
    /// `ServerConfig::daily_token_budget`, the same for every user.
    pub daily_token_budget: Option<u64>,
}

impl AppFactory {
    pub fn tenant_config(&self, schema: &str) -> DbConfig {
        DbConfig::tenant(self.database_url.clone(), schema)
    }

    async fn build(&self, schema: &str) -> Result<CoreApp, CoreError> {
        let mut builder = CoreAppBuilder::new(self.tenant_config(schema))
            .clock(self.clock.clone())
            .secrets(self.secrets.clone())
            .daily_token_budget(self.daily_token_budget);
        for plugin in &self.plugins {
            builder = builder.plugin(plugin.clone());
        }
        if let Some(models) = &self.models {
            builder = builder.models(models.clone());
        }
        if let Some(model) = &self.chat_model_override {
            builder = builder.chat_model_override(model.clone());
        }
        builder.build().await
    }
}

type Cell = Arc<OnceCell<Arc<Tenant>>>;

pub struct Tenants {
    factory: AppFactory,
    /// Keyed by tenant schema. A cell, not the tenant itself, so the slow first open happens
    /// outside the map's lock and concurrent first requests for one user share a single open.
    cells: Mutex<HashMap<String, Cell>>,
}

impl Tenants {
    pub fn new(factory: AppFactory) -> Self {
        Self {
            factory,
            cells: Mutex::new(HashMap::new()),
        }
    }

    pub fn factory(&self) -> &AppFactory {
        &self.factory
    }

    fn cell(&self, schema: &str) -> Cell {
        self.cells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(schema.to_string())
            .or_default()
            .clone()
    }

    /// The user's tenant, opening it (and provisioning the schema and role) if needed. Counts
    /// as activity for idle eviction.
    pub async fn get(&self, schema: &str) -> Result<Arc<Tenant>, CoreError> {
        let tenant = self.get_quietly(schema).await?;
        tenant.touch();
        Ok(tenant)
    }

    /// Like `get`, but without counting as user activity: the scheduler tick visits every user
    /// and shouldn't keep them all cached.
    pub async fn get_quietly(&self, schema: &str) -> Result<Arc<Tenant>, CoreError> {
        let cell = self.cell(schema);
        let tenant = cell
            .get_or_try_init(|| async {
                let app = self.factory.build(schema).await?;
                Ok::<_, CoreError>(Arc::new(Tenant {
                    scheduler: tokio::sync::Mutex::new(Scheduler::new(app.clone())),
                    app,
                    imports: PendingImportState::default(),
                    last_used: Mutex::new(Instant::now()),
                }))
            })
            .await?;
        Ok(tenant.clone())
    }

    /// Forgets a tenant (account deletion). Requests still holding it finish normally.
    pub fn remove(&self, schema: &str) {
        self.cells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(schema);
    }

    /// Drops tenants idle for longer than `max_idle` that nothing else holds (no request in
    /// flight, no open event stream). Dropping the last `CoreApp` handle stops its dispatcher
    /// and closes its pool. Returns how many were evicted.
    pub fn evict_idle(&self, max_idle: Duration) -> usize {
        let mut cells = self.cells.lock().unwrap_or_else(PoisonError::into_inner);
        let before = cells.len();
        cells.retain(|_, cell| match cell.get() {
            // The map's cell holds one reference to the tenant; anything more is a user.
            Some(tenant) => Arc::strong_count(tenant) > 1 || tenant.idle_for() < max_idle,
            // Never opened, or the open failed: nothing to keep unless someone is opening it.
            None => Arc::strong_count(cell) > 1,
        });
        before - cells.len()
    }

    pub fn len(&self) -> usize {
        self.cells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
