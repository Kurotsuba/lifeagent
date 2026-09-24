//! Shared test scaffolding for constructing a [`PluginContext`] without duplicating setup in
//! every module's test suite. Test-only (`#[cfg(test)]`): never compiled into a real build.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::router::{LlmRouter, ModelsConfig, ProviderConfig, ProviderKind, RoleBinding};
use lifeagent_llm::types::ChatModel;

use crate::clock::{Clock, FixedClock};
use crate::events::EventStore;
use crate::goals::GoalsService;
use crate::memory::MemoryService;
use crate::plugin::PluginContext;
use crate::profile::ProfileService;
use crate::settings::Settings;
use crate::usage::DbUsageSink;

/// A [`PluginContext`] wired to an isolated temp-dir database and a [`FixedClock`], plus the
/// [`MockChatModel`] handle behind `ctx.llm` for tests that do script LLM calls. Keep the
/// returned [`tempfile::TempDir`] alive for as long as the context is used.
pub(crate) async fn context() -> (PluginContext, Arc<MockChatModel>, tempfile::TempDir) {
    let (db, dir) = crate::db::test_pool().await;
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(Utc::now()));
    let settings = Settings::new(db.clone());
    let events = EventStore::new(db.clone(), clock.clone(), settings.clone());
    let profile = ProfileService::new(db.clone(), clock.clone());
    let goals = GoalsService::new(db.clone(), clock.clone());
    let memory = MemoryService::new(db.clone(), clock.clone());

    let mock = Arc::new(MockChatModel::new());
    let binding = RoleBinding {
        provider_id: "mock".into(),
        model: "mock".into(),
        temperature: None,
        effort: None,
    };
    let config = ModelsConfig {
        providers: vec![ProviderConfig {
            id: "mock".into(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: String::new(),
        }],
        parser: binding.clone(),
        planner: binding.clone(),
        conversation: binding,
    };
    let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
    providers.insert("mock".into(), mock.clone() as Arc<dyn ChatModel>);
    let router = Arc::new(LlmRouter::with_providers(
        config,
        providers,
        Arc::new(DbUsageSink::new(db.clone(), clock.clone())),
    ));

    let ctx = PluginContext {
        db,
        events,
        llm: router,
        clock,
        settings,
        profile,
        goals,
        memory,
    };
    (ctx, mock, dir)
}
