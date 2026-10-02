//! Shared test scaffolding for building a [`PluginContext`] without repeating the setup in
//! every test suite. Compiled only for this crate's tests or with the `test-support` feature
//! (which `lifeagent-plugin-diet` enables for its dev-dependency), never in a real build.

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
use crate::plan::PlanStore;
use crate::plugin::PluginContext;
use crate::profile::ProfileService;
use crate::settings::Settings;
use crate::usage::DbUsageSink;

/// A [`PluginContext`] on an isolated test schema, with every LLM role bound to `mock` and
/// "now" controlled by `clock`. Keep the value alive while the context is in use: dropping
/// `db` drops the schema.
pub struct TestEnv {
    pub ctx: PluginContext,
    pub mock: Arc<MockChatModel>,
    pub clock: Arc<FixedClock>,
    pub db: crate::db::TestDb,
}

pub async fn env() -> TestEnv {
    let (db, test_db) = crate::db::test_pool().await;
    let clock = Arc::new(FixedClock::new(Utc::now()));
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    let settings = Settings::new(db.clone());
    let events = EventStore::new(db.clone(), dyn_clock.clone(), settings.clone());

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
            auth: Default::default(),
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
        Arc::new(DbUsageSink::new(db.clone(), dyn_clock.clone())),
    ));

    let ctx = PluginContext {
        db: db.clone(),
        events,
        llm: router,
        clock: dyn_clock.clone(),
        settings,
        profile: ProfileService::new(db.clone(), dyn_clock.clone()),
        goals: GoalsService::new(db.clone(), dyn_clock.clone()),
        memory: MemoryService::new(db.clone(), dyn_clock.clone()),
        plans: PlanStore::new(db, dyn_clock),
    };
    TestEnv {
        ctx,
        mock,
        clock,
        db: test_db,
    }
}

/// Shorthand for tests that don't need to move the clock.
pub async fn context() -> (PluginContext, Arc<MockChatModel>, crate::db::TestDb) {
    let env = env().await;
    (env.ctx, env.mock, env.db)
}
