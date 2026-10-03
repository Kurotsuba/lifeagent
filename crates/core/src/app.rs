//! `CoreAppBuilder` / `CoreApp`: composition root, orchestrator loop, event dispatcher and
//! today view, per `docs/PHASE1_SPEC.md` §7.1/§7.5/§7.11/§7.14.
//!
//! The rest of `CoreApp`'s eventual surface (plan methods, `start_scheduler`, ...) is M5 work
//! and is added incrementally rather than stubbed out here.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, RwLock};

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use lifeagent_llm::router::{
    LlmRouter, ModelsConfig, ProviderConfig, ProviderKind, RoleBinding, RoleRequest,
};
use lifeagent_llm::types::{ChatModel, Message, ResponseFormat, Role, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, Mutex};

use crate::clock::{Clock, SystemClock};
use crate::context;
use crate::conversation::{ConversationStore, MessageRole, NewMessage, StoredMessage};
use crate::db;
use crate::error::{CoreError, ValidationRule};
use crate::events::{AppEvent, EventChange, EventStore};
use crate::goals::{self, GoalsService, WeightGoal};
use crate::plan::{PlanService, PlanStore, PlanTrigger, StoredPlan};
use crate::plugin::{Plugin, PluginContext, PluginError};
use crate::preferences::PreferenceService;
use crate::profile::ProfileService;
use crate::scheduler::{Scheduler, TICK_INTERVAL};
use crate::secrets::{KeyringSecretStore, SecretKeyProvider, SecretStore};
use crate::settings::Settings;
use crate::time::{logical_date, logical_date_from_local};
use crate::tools::ToolRegistry;
use crate::usage::DbUsageSink;

/// Loop cap for `send_message` (§7.11): stop after this many tool-call rounds rather than
/// let a confused model spin forever.
const MAX_ITERATIONS: u32 = 6;

const EMPTY_REPLY: &str = "（模型没有返回内容，请再试一次。）";
const TOO_MANY_STEPS_REPLY: &str = "（处理步骤过多，已停止。请换个说法再试一次。）";

/// The result of one `send_message` turn.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct ChatTurn {
    pub reply: String,
    pub activities: Vec<String>,
}

/// One message in `get_history`'s result (§10.2): user/assistant only, tool call plumbing
/// hidden. Past turns' `activities` aren't reconstructed here — see `docs/DECISIONS.md`'s
/// M7.1-step-1 entry; only a *live* `send_message` response carries them (on `ChatTurn`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryMessage {
    pub role: MessageRole,
    pub content: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A logged weight, from the latest `body.weight_logged` event. §7.14 names this type but
/// never defines it; see M4 in `docs/DECISIONS.md`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeightPoint {
    pub kg: f64,
    pub logical_date: NaiveDate,
    pub occurred_at: DateTime<Utc>,
}

/// Everything the Today page shows (§7.14).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TodayView {
    pub logical_date: NaiveDate,
    /// Plugin id -> that plugin's `today_card`.
    pub cards: BTreeMap<String, Value>,
    pub today_plan: Option<StoredPlan>,
    pub tomorrow_plan: Option<StoredPlan>,
    pub latest_weight: Option<WeightPoint>,
}

pub struct CoreAppBuilder {
    db: db::DbConfig,
    clock: Arc<dyn Clock>,
    secrets: Arc<dyn SecretStore>,
    plugins: Vec<Arc<dyn Plugin>>,
    chat_model_override: Option<Arc<dyn ChatModel>>,
    fixed_models: Option<ModelsConfig>,
}

impl CoreAppBuilder {
    pub fn new(db: db::DbConfig) -> Self {
        Self {
            db,
            clock: Arc::new(SystemClock),
            secrets: Arc::new(KeyringSecretStore),
            plugins: Vec::new(),
            chat_model_override: None,
            fixed_models: None,
        }
    }

    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Overrides the default `KeyringSecretStore` — tests use `MemorySecretStore`.
    pub fn secrets(mut self, secrets: Arc<dyn SecretStore>) -> Self {
        self.secrets = secrets;
        self
    }

    pub fn plugin(mut self, p: Arc<dyn Plugin>) -> Self {
        self.plugins.push(p);
        self
    }

    /// Uses `config` instead of the `models` setting, for this build and every
    /// `reload_models`: the web server's models are set by the operator for all users, who
    /// never configure their own (`docs/HACKATHON.md` §2). Keys still come from `secrets`.
    pub fn models(mut self, config: ModelsConfig) -> Self {
        self.fixed_models = Some(config);
        self
    }

    /// Bypasses real provider construction entirely: every role is bound to `m`. Used by
    /// tests and the CLI's `--mock` flag.
    pub fn chat_model_override(mut self, m: Arc<dyn ChatModel>) -> Self {
        self.chat_model_override = Some(m);
        self
    }

    /// Opens the database, runs core and plugin migrations, builds the tool registry and the
    /// LLM router, and starts the event dispatcher. Must be called inside a tokio runtime.
    pub async fn build(self) -> Result<CoreApp, CoreError> {
        let db = db::open(&self.db).await?;
        let settings = Settings::new(db.clone());
        let events = EventStore::new(db.clone(), self.clock.clone(), settings.clone());
        let profile = ProfileService::new(db.clone(), self.clock.clone());
        let goals = GoalsService::new(db.clone(), self.clock.clone());
        let preferences = PreferenceService::new(db.clone(), self.clock.clone());
        let conversation = ConversationStore::new(db.clone(), self.clock.clone());
        let plans = PlanStore::new(db.clone(), self.clock.clone());

        for plugin in &self.plugins {
            crate::plugin::apply_migrations(&db, self.clock.as_ref(), &plugin.manifest()).await?;
        }

        let planner = Arc::new(PlanService::new(self.plugins.clone()));
        let registry = ToolRegistry::build(&self.plugins, planner.clone())?;
        let (router, models_configured) = self.build_router(&db, &settings).await?;

        let app = CoreApp(Arc::new(CoreAppInner {
            db,
            clock: self.clock,
            settings,
            secrets: self.secrets,
            fixed_models: self.fixed_models,
            router_state: RwLock::new(Arc::new(RouterState {
                router,
                models_configured,
            })),
            events,
            profile,
            goals,
            preferences,
            conversation,
            plans,
            planner,
            plugins: self.plugins,
            registry,
            send_lock: Mutex::new(()),
            dispatcher: OnceLock::new(),
        }));
        app.spawn_dispatcher();
        Ok(app)
    }

    /// A mock override always wins (tests, `--mock`); otherwise builds real providers from
    /// `settings.models()`. With neither, returns a placeholder router whose every call fails
    /// with `LlmError::Config`, so a `PluginContext` can still be built for read-only paths
    /// like `today_view` before onboarding configures a model. The `bool` says whether models
    /// are really configured.
    async fn build_router(
        &self,
        db: &PgPool,
        settings: &Settings,
    ) -> Result<(Arc<LlmRouter>, bool), CoreError> {
        let usage_sink = Arc::new(DbUsageSink::new(db.clone(), self.clock.clone()));

        if let Some(model) = &self.chat_model_override {
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
            providers.insert("mock".into(), model.clone());
            let router = LlmRouter::with_providers(config, providers, usage_sink);
            return Ok((Arc::new(router), true));
        }

        build_router_state(
            settings,
            self.fixed_models.as_ref(),
            &self.secrets,
            usage_sink,
        )
        .await
    }
}

/// The non-override half of [`CoreAppBuilder::build_router`], factored out so
/// [`CoreApp::reload_models`] can rebuild the router from the same logic after
/// `save_models`/`set_api_key` change what's stored, without needing a `CoreAppBuilder` (which
/// only ever exists at startup) or its test/CLI-only `chat_model_override`.
async fn build_router_state(
    settings: &Settings,
    fixed_models: Option<&ModelsConfig>,
    secrets: &Arc<dyn SecretStore>,
    usage_sink: Arc<DbUsageSink>,
) -> Result<(Arc<LlmRouter>, bool), CoreError> {
    let config = match fixed_models {
        Some(config) => Some(config.clone()),
        None => settings.models().await?,
    };
    match config {
        Some(config) => {
            let key_provider = SecretKeyProvider(secrets.clone());
            match LlmRouter::new(config, &key_provider, usage_sink.clone()) {
                Ok(router) => Ok((Arc::new(router), true)),
                // A stored config that can't build (e.g. the key isn't in the secret store
                // right now) must not fail the caller outright — a shell needs a working
                // `CoreApp` to reach Settings and fix it. Falls back to the same no-providers
                // placeholder as "never configured."
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "stored models config could not be used; treating as not configured"
                    );
                    Ok((Arc::new(unconfigured_router(usage_sink)), false))
                }
            }
        }
        None => Ok((Arc::new(unconfigured_router(usage_sink)), false)),
    }
}

/// A router with no providers: every call fails with `LlmError::Config` naming the
/// "models-not-configured" provider id.
fn unconfigured_router(usage_sink: Arc<DbUsageSink>) -> LlmRouter {
    let binding = RoleBinding {
        provider_id: "models-not-configured".into(),
        model: String::new(),
        temperature: None,
        effort: None,
    };
    let config = ModelsConfig {
        providers: Vec::new(),
        parser: binding.clone(),
        planner: binding.clone(),
        conversation: binding,
    };
    LlmRouter::with_providers(config, HashMap::new(), usage_sink)
}

/// The router and whether it's backed by a real, working config — swapped as one unit by
/// [`CoreApp::reload_models`] so a reader never sees a router paired with the wrong flag.
struct RouterState {
    router: Arc<LlmRouter>,
    models_configured: bool,
}

struct CoreAppInner {
    db: PgPool,
    clock: Arc<dyn Clock>,
    settings: Settings,
    secrets: Arc<dyn SecretStore>,
    /// `CoreAppBuilder::models`, which wins over the `models` setting.
    fixed_models: Option<ModelsConfig>,
    /// `std::sync::RwLock`, not `tokio`'s: every hold is a plain `Arc` clone with no `.await`
    /// inside it, so a blocking lock is simpler and correct — see `router_state()`.
    router_state: RwLock<Arc<RouterState>>,
    events: EventStore,
    profile: ProfileService,
    goals: GoalsService,
    preferences: PreferenceService,
    conversation: ConversationStore,
    plans: PlanStore,
    /// Shared with the tool registry, so chat, the scheduler and the shell all go through
    /// one generation lock.
    planner: Arc<PlanService>,
    plugins: Vec<Arc<dyn Plugin>>,
    registry: ToolRegistry,
    /// Serializes `send_message` calls (§7.11: "one turn at a time").
    send_lock: Mutex<()>,
    /// Set once by `spawn_dispatcher`. The task holds its own `PluginContext` (and so the event
    /// bus's sender), so it never ends by itself; it is aborted when the last `CoreApp` handle
    /// drops, which lets a server evict an idle user's app without leaking its task and pool.
    dispatcher: OnceLock<tokio::task::AbortHandle>,
}

impl Drop for CoreAppInner {
    fn drop(&mut self) {
        if let Some(dispatcher) = self.dispatcher.get() {
            dispatcher.abort();
        }
    }
}

/// Cheaply `Clone`-able handle (inner `Arc`) so shells can share one instance across threads.
#[derive(Clone)]
pub struct CoreApp(Arc<CoreAppInner>);

/// The wire shape a `ToolCall` is stored as in `messages.tool_calls_json`. Plain data, so it
/// derives `Serialize`/`Deserialize`, unlike `lifeagent_llm::types::ToolCall` which has no
/// reason to depend on serde for a type that crate never persists itself.
#[derive(Serialize, Deserialize)]
struct StoredToolCall {
    id: String,
    name: String,
    arguments: Value,
    /// Must be replayed to the provider with the call (Gemini thought signatures). Absent in
    /// rows written before it existed and for providers that don't use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_extra: Option<Value>,
}

impl CoreApp {
    pub fn events(&self) -> &EventStore {
        &self.0.events
    }
    pub fn profile(&self) -> &ProfileService {
        &self.0.profile
    }
    pub fn goals(&self) -> &GoalsService {
        &self.0.goals
    }
    pub fn settings(&self) -> &Settings {
        &self.0.settings
    }
    pub fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.0.secrets
    }
    pub fn preferences(&self) -> &PreferenceService {
        &self.0.preferences
    }
    pub fn conversation(&self) -> &ConversationStore {
        &self.0.conversation
    }
    pub fn plans(&self) -> &PlanStore {
        &self.0.plans
    }

    /// The injected `Clock` — composition roots use it for "now" (e.g. settings' `updated_at`)
    /// instead of `Utc::now()`, so tests can control time.
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.0.clock
    }
    /// The current `RouterState`. A plain `Arc` clone under a brief, non-`.await`-holding lock
    /// — see the field's own doc comment.
    fn router_state(&self) -> Arc<RouterState> {
        self.0
            .router_state
            .read()
            .expect("router_state lock poisoned")
            .clone()
    }

    /// Whether the LLM router is backed by a real, working config right now. Exposed (M7) for
    /// `get_app_state`; `scheduler.rs` also reads this internally to skip plan generation.
    pub fn models_configured(&self) -> bool {
        self.router_state().models_configured
    }

    fn require_models(&self) -> Result<(), CoreError> {
        if self.models_configured() {
            Ok(())
        } else {
            Err(CoreError::NotConfigured)
        }
    }

    /// Rebuilds the LLM router from the fixed models config if one was given, else from
    /// whatever `settings.models()` holds right now and swaps it
    /// in, so a `save_models`/`set_api_key` command takes effect for this already-running
    /// `CoreApp` immediately — previously an open item (see `docs/DECISIONS.md`: M4's
    /// "today_view works before models are configured" entry flagged this as "Open for M7").
    /// Falls back to "not configured" exactly like `build()` does, rather than erroring, for the
    /// same reason: a shell needs `CoreApp` to keep working so the user can fix Settings.
    pub async fn reload_models(&self) -> Result<(), CoreError> {
        let usage_sink = Arc::new(DbUsageSink::new(self.0.db.clone(), self.0.clock.clone()));
        let (router, models_configured) = build_router_state(
            &self.0.settings,
            self.0.fixed_models.as_ref(),
            &self.0.secrets,
            usage_sink,
        )
        .await?;
        *self
            .0
            .router_state
            .write()
            .expect("router_state lock poisoned") = Arc::new(RouterState {
            router,
            models_configured,
        });
        Ok(())
    }

    /// The current version of the plan for `date`, if any.
    pub async fn get_plan(&self, date: NaiveDate) -> Result<Option<StoredPlan>, CoreError> {
        self.0.plans.current(date).await
    }

    /// Generates a new plan version for `date` (§7.12), with the user's optional `request`
    /// for this plan only.
    pub async fn generate_plan(
        &self,
        date: NaiveDate,
        trigger: PlanTrigger,
        request: Option<&str>,
    ) -> Result<StoredPlan, CoreError> {
        self.require_models()?;
        self.0
            .planner
            .generate(&self.plugin_context(), date, trigger, request)
            .await
    }

    /// Revises the current plan for `date` following the user's `instruction`.
    pub async fn revise_plan(
        &self,
        date: NaiveDate,
        instruction: &str,
    ) -> Result<StoredPlan, CoreError> {
        self.require_models()?;
        self.0
            .planner
            .revise(&self.plugin_context(), date, instruction)
            .await
    }

    /// Marks the current plan for `date` accepted.
    pub async fn accept_plan(&self, date: NaiveDate) -> Result<(), CoreError> {
        self.0.plans.accept(date).await
    }

    /// The most recent `limit` user/assistant messages, oldest first, for the Chat page's
    /// history (§10.2's `get_history`).
    pub async fn get_history(&self, limit: u32) -> Result<Vec<HistoryMessage>, CoreError> {
        let messages = self.0.conversation.recent_visible(limit).await?;
        Ok(messages
            .into_iter()
            .map(|m| HistoryMessage {
                role: m.role,
                content: m.content,
                created_at: m.created_at,
            })
            .collect())
    }

    /// Soft-deletes a logged entry (§10.2's `delete_entry`) — any event, not just diet ones,
    /// since the command is generic over event id.
    pub async fn delete_entry(&self, id: &str) -> Result<(), CoreError> {
        self.0.events.soft_delete(id).await?;
        Ok(())
    }

    /// Saves the active weight goal (§10.2's `save_weight_goal`), filling in §7.4's default
    /// target from the latest logged weight when a Lose/Gain goal has none.
    pub async fn save_weight_goal(&self, goal: &WeightGoal) -> Result<(), CoreError> {
        let latest_kg = self.latest_weight().await?.map(|w| w.kg);
        self.0
            .goals
            .save_weight_goal(&goals::with_default_target(goal, latest_kg))
            .await
    }

    async fn latest_weight(&self) -> Result<Option<WeightPoint>, CoreError> {
        Ok(self
            .0
            .events
            .latest("body.weight_logged")
            .await?
            .and_then(|event| {
                let kg = event.payload.get("kg").and_then(Value::as_f64)?;
                Some(WeightPoint {
                    kg,
                    logical_date: event.logical_date,
                    occurred_at: event.occurred_at,
                })
            }))
    }

    /// Records a body weight reading from the UI (§10.2's `log_weight`), same validation as the
    /// `core_log_weight` chat tool (§7.8), always at "now" (the UI has no equivalent of that
    /// tool's optional `measured_at`).
    pub async fn log_weight(&self, kg: f64) -> Result<(), CoreError> {
        if !(30.0..=300.0).contains(&kg) {
            return Err(CoreError::InvalidInput(ValidationRule::WeightOutOfRange {
                got: kg,
            }));
        }
        self.0
            .events
            .append(
                "body.weight_logged",
                self.0.clock.now(),
                serde_json::json!({ "kg": kg }),
                crate::events::EventSource::Ui,
            )
            .await?;
        Ok(())
    }

    /// Starts the scheduler (§7.13): one tick immediately, then every 60 s, for the life of
    /// the runtime. Shells call this once.
    pub fn start_scheduler(&self) -> tokio::task::JoinHandle<()> {
        let mut scheduler = Scheduler::new(self.clone());
        tokio::spawn(async move {
            loop {
                scheduler.tick().await;
                tokio::time::sleep(TICK_INTERVAL).await;
            }
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.0.events.subscribe()
    }

    /// Assembles a fresh [`PluginContext`]. Exposed (M6) so a composition root can reach
    /// plugin-specific reads `CoreApp` itself has no business knowing about (e.g. the diet
    /// plugin's own food table) without `core` depending on that plugin — see the dependency
    /// rule in `CLAUDE.md`.
    pub fn plugin_context(&self) -> PluginContext {
        PluginContext {
            db: self.0.db.clone(),
            events: self.0.events.clone(),
            llm: self.router_state().router.clone(),
            clock: self.0.clock.clone(),
            settings: self.0.settings.clone(),
            profile: self.0.profile.clone(),
            goals: self.0.goals.clone(),
            preferences: self.0.preferences.clone(),
            plans: self.0.plans.clone(),
        }
    }

    /// The dispatcher (§7.5): delivers each appended event to every plugin's `on_event` and
    /// each deleted one to `on_event_deleted` (errors are logged, not propagated), then emits
    /// `AppEvent::DataChanged` for both. Runs until the last `CoreApp` handle drops (see `CoreAppInner::dispatcher`).
    fn spawn_dispatcher(&self) {
        let mut changes = self.0.events.subscribe_changes();
        let ctx = self.plugin_context();
        let plugins = self.0.plugins.clone();
        let task = tokio::spawn(async move {
            loop {
                let change = match changes.recv().await {
                    Ok(change) => change,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "event dispatcher lagged; some changes skipped");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };
                for plugin in &plugins {
                    let result = match &change {
                        EventChange::Appended(event) => plugin.on_event(&ctx, event).await,
                        EventChange::Deleted(event) => plugin.on_event_deleted(&ctx, event).await,
                    };
                    if let Err(e) = result {
                        tracing::warn!(plugin = plugin.manifest().id, error = %e, "event hook failed");
                    }
                }
                let event = change.event();
                ctx.events.publish(AppEvent::DataChanged {
                    event_type: event.event_type.clone(),
                    logical_date: event.logical_date,
                });
            }
        });
        let _ = self.0.dispatcher.set(task.abort_handle());
    }

    /// The current logical date (§4), using the `day_boundary` setting.
    pub async fn logical_today(&self) -> Result<NaiveDate, CoreError> {
        let boundary = self.0.settings.day_boundary().await?;
        Ok(logical_date(self.0.clock.now(), boundary))
    }

    /// Token usage per LLM role for the current calendar month (§10.3's Settings usage
    /// display). "Current month" is the UTC calendar month of `Clock::now()`, same as every
    /// `created_at` timestamp it's computed from — a simplification worth revisiting only if a
    /// non-UTC user actually notices the boundary landing a few hours off from their own
    /// midnight.
    pub async fn usage_summary(&self) -> Result<Vec<crate::usage::RoleUsage>, CoreError> {
        let now = self.0.clock.now();
        let month_start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
            .expect("the first of the month is always a valid date")
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always a valid time")
            .and_utc();
        crate::usage::summary(&self.0.db, month_start).await
    }

    /// Everything the Today page shows (§7.14). Works before models are configured.
    pub async fn today_view(&self) -> Result<TodayView, CoreError> {
        let today = self.logical_today().await?;
        let ctx = self.plugin_context();

        let mut cards = BTreeMap::new();
        for plugin in &self.0.plugins {
            let id = plugin.manifest().id;
            let card = plugin
                .today_card(&ctx, today)
                .await
                .map_err(|e| CoreError::Plugin(format!("{id}: {e}")))?;
            if let Some(card) = card {
                cards.insert(id.to_string(), card);
            }
        }

        let latest_weight = self.latest_weight().await?;

        Ok(TodayView {
            logical_date: today,
            cards,
            today_plan: self.0.plans.current(today).await?,
            tomorrow_plan: match today.succ_opt() {
                Some(tomorrow) => self.0.plans.current(tomorrow).await?,
                None => None,
            },
            latest_weight,
        })
    }

    /// Runs one conversation turn (§7.11): stores the user message, then alternates model
    /// calls and tool dispatch — tool calls within a round run sequentially, in call order —
    /// until the model stops calling tools or the iteration cap is hit.
    ///
    /// Retry-safe: if the previous turn failed before a final reply and `text` is that turn's
    /// own user message, the turn is resumed rather than the message being stored twice (the
    /// user message is persisted before the first model call, so a failed turn leaves it
    /// behind).
    pub async fn send_message(&self, text: &str) -> Result<ChatTurn, CoreError> {
        self.require_models()?;
        let _guard = self.0.send_lock.lock().await;

        let is_retry = self
            .0
            .conversation
            .unfinished_user_message()
            .await?
            .is_some_and(|m| m.content.as_deref() == Some(text));
        if !is_retry {
            self.0
                .conversation
                .append(NewMessage {
                    role: MessageRole::User,
                    content: Some(text.to_string()),
                    ..Default::default()
                })
                .await?;
        }

        let plugin_ctx = self.plugin_context();
        let router = plugin_ctx.llm.clone();
        let tool_specs = self.0.registry.specs();
        let mut activities = Vec::new();
        // Built once per turn, not per iteration: rebuilt after a tool call, "Today's status"
        // already lists the meal that call just logged, and the model then tells the user it
        // was "already recorded". Tool results carry the updated totals instead.
        let system_prompt = context::build_system_prompt(&plugin_ctx, &self.0.plugins).await?;
        let boundary = self.0.settings.day_boundary().await?;

        for _ in 0..MAX_ITERATIONS {
            let history = self.0.conversation.history_window().await?;
            let messages = Self::to_llm_messages(system_prompt.clone(), &history, boundary)?;

            let resp = router
                .complete(
                    Role::Conversation,
                    RoleRequest {
                        messages,
                        tools: tool_specs.clone(),
                        max_tokens: None,
                        response_format: ResponseFormat::Text,
                    },
                )
                .await?;

            let tool_calls_json = Self::serialize_tool_calls(&resp.tool_calls)?;
            self.0
                .conversation
                .append(NewMessage {
                    role: MessageRole::Assistant,
                    content: resp.content.clone(),
                    tool_calls_json,
                    ..Default::default()
                })
                .await?;

            if resp.tool_calls.is_empty() {
                let reply = resp.content.unwrap_or_else(|| EMPTY_REPLY.to_string());
                return Ok(ChatTurn { reply, activities });
            }

            for call in resp.tool_calls {
                let content = match self
                    .0
                    .registry
                    .call(&plugin_ctx, &call.name, call.arguments)
                    .await
                {
                    Ok(out) => {
                        activities.push(out.summary);
                        out.content
                    }
                    Err(PluginError::InvalidArgs(msg)) | Err(PluginError::NotFound(msg)) => {
                        serde_json::json!({ "error": msg })
                    }
                    Err(other) => {
                        tracing::error!(error = %other, tool = %call.name, "tool call failed");
                        serde_json::json!({ "error": "internal error" })
                    }
                };

                self.0
                    .conversation
                    .append(NewMessage {
                        role: MessageRole::Tool,
                        content: Some(content.to_string()),
                        tool_call_id: Some(call.id),
                        tool_name: Some(call.name),
                        ..Default::default()
                    })
                    .await?;
            }
        }

        Ok(ChatTurn {
            reply: TOO_MANY_STEPS_REPLY.to_string(),
            activities,
        })
    }

    fn serialize_tool_calls(tool_calls: &[ToolCall]) -> Result<Option<String>, CoreError> {
        if tool_calls.is_empty() {
            return Ok(None);
        }
        let stored: Vec<StoredToolCall> = tool_calls
            .iter()
            .map(|t| StoredToolCall {
                id: t.id.clone(),
                name: t.name.clone(),
                arguments: t.arguments.clone(),
                provider_extra: t.provider_extra.clone(),
            })
            .collect();
        Ok(Some(serde_json::to_string(&stored)?))
    }

    /// Each user message is prefixed with when it was sent (see [`user_time_tag`]): the
    /// history window spans day boundaries, and without it yesterday's "今天" reads as today's.
    fn to_llm_messages(
        system_prompt: String,
        history: &[StoredMessage],
        boundary: NaiveTime,
    ) -> Result<Vec<Message>, CoreError> {
        let mut messages = vec![Message::System {
            content: system_prompt,
        }];
        for m in history {
            match m.role {
                MessageRole::User => {
                    let local = m.created_at.with_timezone(&Local).naive_local();
                    messages.push(Message::User {
                        content: format!(
                            "{} {}",
                            user_time_tag(local, boundary),
                            m.content.as_deref().unwrap_or_default()
                        ),
                    })
                }
                MessageRole::Assistant => {
                    let tool_calls = match &m.tool_calls_json {
                        Some(json) => {
                            let stored: Vec<StoredToolCall> = serde_json::from_str(json)?;
                            stored
                                .into_iter()
                                .map(|t| ToolCall {
                                    id: t.id,
                                    name: t.name,
                                    arguments: t.arguments,
                                    provider_extra: t.provider_extra,
                                })
                                .collect()
                        }
                        None => Vec::new(),
                    };
                    messages.push(Message::Assistant {
                        content: m.content.clone(),
                        tool_calls,
                    });
                }
                MessageRole::Tool => messages.push(Message::Tool {
                    tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                    content: m.content.clone().unwrap_or_default(),
                }),
            }
        }
        Ok(messages)
    }
}

/// `[sent YYYY-MM-DD HH:MM, logical date YYYY-MM-DD]`, the prefix the conversation model
/// sees on each user message. Pure over the local time so tests don't depend on the TZ.
fn user_time_tag(local: NaiveDateTime, boundary: NaiveTime) -> String {
    format!(
        "[sent {}, logical date {}]",
        local.format("%Y-%m-%d %H:%M"),
        logical_date_from_local(local, boundary)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::secrets::MemorySecretStore;
    use chrono::Utc;
    use lifeagent_llm::mock::MockChatModel;
    use lifeagent_llm::types::{ChatResponse, Usage};

    fn response(content: Option<&str>, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            content: content.map(|s| s.to_string()),
            tool_calls,
            usage: Usage::default(),
            finish_reason: "stop".into(),
        }
    }

    async fn builder() -> (CoreAppBuilder, crate::db::TestDb) {
        let dir = crate::db::TestDb::new().await;
        let builder = CoreAppBuilder::new(dir.config.clone())
            .clock(Arc::new(FixedClock::new(Utc::now())))
            .secrets(Arc::new(MemorySecretStore::new()));
        (builder, dir)
    }

    /// Records every event its `on_event` and `on_event_deleted` receive, and returns a fixed
    /// today card.
    #[derive(Default)]
    struct RecordingPlugin {
        seen: std::sync::Mutex<Vec<String>>,
        deleted: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Plugin for RecordingPlugin {
        fn manifest(&self) -> crate::plugin::PluginManifest {
            crate::plugin::PluginManifest {
                id: "rec",
                name: "Recording",
                version: "0.1.0",
                event_types: vec![],
                migrations: vec![],
            }
        }
        fn tools(&self) -> Vec<lifeagent_llm::types::ToolSpec> {
            vec![]
        }
        async fn call_tool(
            &self,
            _ctx: &PluginContext,
            _name: &str,
            _args: Value,
        ) -> Result<crate::plugin::ToolOutput, PluginError> {
            unreachable!()
        }
        async fn on_event(
            &self,
            _ctx: &PluginContext,
            e: &crate::events::Event,
        ) -> Result<(), PluginError> {
            self.seen.lock().unwrap().push(e.id.clone());
            Ok(())
        }
        async fn on_event_deleted(
            &self,
            _ctx: &PluginContext,
            e: &crate::events::Event,
        ) -> Result<(), PluginError> {
            self.deleted.lock().unwrap().push(e.id.clone());
            Ok(())
        }
        async fn today_card(
            &self,
            _ctx: &PluginContext,
            date: NaiveDate,
        ) -> Result<Option<Value>, PluginError> {
            Ok(Some(serde_json::json!({ "date": date.to_string() })))
        }
    }

    async fn next_data_changed(rx: &mut broadcast::Receiver<AppEvent>) -> String {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for DataChanged")
            .unwrap();
        match event {
            AppEvent::DataChanged { event_type, .. } => event_type,
            other => panic!("unexpected AppEvent: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatcher_delivers_appends_to_plugins_then_emits_data_changed() {
        let (builder, _dir) = builder().await;
        let plugin = Arc::new(RecordingPlugin::default());
        let app = builder.plugin(plugin.clone()).build().await.unwrap();
        let mut rx = app.subscribe();

        let event = app
            .events()
            .append(
                "test.thing",
                Utc::now(),
                serde_json::json!({}),
                crate::events::EventSource::Chat,
            )
            .await
            .unwrap();

        assert_eq!(next_data_changed(&mut rx).await, "test.thing");
        // DataChanged is only emitted after on_event ran.
        assert_eq!(*plugin.seen.lock().unwrap(), vec![event.id]);
    }

    #[tokio::test]
    async fn dispatcher_delivers_deletes_to_on_event_deleted_then_emits_data_changed() {
        let (builder, _dir) = builder().await;
        let plugin = Arc::new(RecordingPlugin::default());
        let app = builder.plugin(plugin.clone()).build().await.unwrap();
        let mut rx = app.subscribe();

        let event = app
            .events()
            .append(
                "test.thing",
                Utc::now(),
                serde_json::json!({}),
                crate::events::EventSource::Chat,
            )
            .await
            .unwrap();
        next_data_changed(&mut rx).await;
        app.events().soft_delete(&event.id).await.unwrap();

        assert_eq!(next_data_changed(&mut rx).await, "test.thing");
        assert_eq!(plugin.seen.lock().unwrap().len(), 1);
        assert_eq!(*plugin.deleted.lock().unwrap(), vec![event.id]);
    }

    #[tokio::test]
    async fn today_view_works_without_models_configured() {
        let (builder, _dir) = builder().await;
        let app = builder
            .plugin(Arc::new(RecordingPlugin::default()))
            .build()
            .await
            .unwrap();
        let today = app.logical_today().await.unwrap();

        let view = app.today_view().await.unwrap();

        assert_eq!(view.logical_date, today);
        assert_eq!(
            view.cards.get("rec"),
            Some(&serde_json::json!({ "date": today.to_string() }))
        );
        assert_eq!(view.today_plan, None);
        assert_eq!(view.latest_weight, None);
    }

    #[tokio::test]
    async fn save_weight_goal_fills_default_target_from_latest_weight() {
        use crate::goals::GoalDirection;

        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();
        app.log_weight(70.0).await.unwrap();

        app.save_weight_goal(&WeightGoal {
            direction: GoalDirection::Lose,
            target_weight_kg: None,
            rate_kg_per_week: 0.5,
        })
        .await
        .unwrap();

        let goal = app.goals().get_active_weight_goal().await.unwrap().unwrap();
        assert_eq!(goal.target_weight_kg, Some(69.0));
    }

    #[tokio::test]
    async fn today_view_includes_latest_weight_and_plans() {
        use crate::plan::{PlanContent, PlanTrigger};

        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();
        let today = app.logical_today().await.unwrap();
        let tomorrow = today.succ_opt().unwrap();
        app.events()
            .append(
                "body.weight_logged",
                Utc::now(),
                serde_json::json!({"kg": 61.2}),
                crate::events::EventSource::Ui,
            )
            .await
            .unwrap();
        let content = PlanContent {
            summary: "明天多吃蔬菜".into(),
            sections: BTreeMap::new(),
            disclaimer: None,
        };
        let plan = app
            .plans()
            .insert(tomorrow, PlanTrigger::Manual, None, &content, &[])
            .await
            .unwrap();

        let view = app.today_view().await.unwrap();

        assert_eq!(view.latest_weight.map(|w| w.kg), Some(61.2));
        assert_eq!(view.today_plan, None);
        assert_eq!(view.tomorrow_plan, Some(plan));
    }

    #[tokio::test]
    async fn send_message_without_models_configured_errors() {
        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();

        assert!(matches!(
            app.send_message("你好").await,
            Err(CoreError::NotConfigured)
        ));
    }

    /// An `anthropic` `ModelsConfig` binding every role to the same model — enough to exercise
    /// real router construction (provider lookup, key fetch) without a live network call.
    fn anthropic_config() -> ModelsConfig {
        let binding = RoleBinding {
            provider_id: "anthropic".into(),
            model: "claude-sonnet-5".into(),
            temperature: None,
            effort: None,
        };
        ModelsConfig {
            providers: vec![ProviderConfig {
                id: "anthropic".into(),
                kind: ProviderKind::Anthropic,
                base_url: "https://api.anthropic.com".into(),
                auth: Default::default(),
            }],
            parser: binding.clone(),
            planner: binding.clone(),
            conversation: binding,
        }
    }

    /// A composition root (M7's desktop shell, in particular) must be able to start even when a
    /// previously-saved `models` config can't build right now — e.g. the OS keyring is
    /// unreachable at this exact moment, though the key was set successfully before. `build()`
    /// itself must still succeed; `models_configured` just reports `false`, same as "never
    /// configured," so the shell can route to Settings instead of failing to launch at all.
    #[tokio::test]
    async fn stored_models_config_with_an_unreachable_key_falls_back_to_unconfigured() {
        let dir = crate::db::TestDb::new().await;
        let clock = Arc::new(FixedClock::new(Utc::now()));
        let secrets = Arc::new(MemorySecretStore::new());

        let app = CoreAppBuilder::new(dir.config.clone())
            .clock(clock.clone())
            .secrets(secrets.clone())
            .build()
            .await
            .unwrap();
        assert!(!app.models_configured());

        app.settings()
            .set_models(&anthropic_config(), Utc::now())
            .await
            .unwrap();
        // Deliberately no `secrets.set("anthropic", ...)`: the key is missing, as it would be
        // if the OS keyring were unreachable at startup despite a config already on disk.

        let app2 = CoreAppBuilder::new(dir.config.clone())
            .clock(clock)
            .secrets(secrets)
            .build()
            .await
            .unwrap();
        assert!(!app2.models_configured());
    }

    /// The M7.1 fix for the M4 entry's "Open for M7" note: `save_models`/`set_api_key` must take
    /// effect on the already-running `CoreApp`, not just on the next app restart.
    #[tokio::test]
    async fn reload_models_picks_up_a_newly_saved_config_without_rebuilding_the_app() {
        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();
        assert!(!app.models_configured());

        app.settings()
            .set_models(&anthropic_config(), Utc::now())
            .await
            .unwrap();
        app.secrets().set("anthropic", "sk-test-key").unwrap();

        app.reload_models().await.unwrap();

        assert!(app.models_configured());
    }

    #[tokio::test]
    async fn reload_models_falls_back_to_unconfigured_when_the_new_config_cannot_build() {
        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();

        app.settings()
            .set_models(&anthropic_config(), Utc::now())
            .await
            .unwrap();
        // No `secrets.set("anthropic", ...)`: the key is missing.

        app.reload_models().await.unwrap();

        assert!(!app.models_configured());
    }

    #[tokio::test]
    async fn fixed_models_win_over_the_setting_and_survive_reload() {
        let (builder, _dir) = builder().await;
        let app = builder.models(anthropic_config()).build().await.unwrap();
        // Configured from the fixed config alone: nothing in settings, key in the secret store.
        app.secrets().set("anthropic", "sk-test-key").unwrap();
        app.reload_models().await.unwrap();
        assert!(app.models_configured());
        assert_eq!(app.settings().models().await.unwrap(), None);

        // A reload keeps using the fixed config rather than the (still empty) setting.
        app.reload_models().await.unwrap();
        assert!(app.models_configured());
    }

    #[tokio::test]
    async fn dropping_the_last_handle_stops_the_dispatcher() {
        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();
        let dispatcher = app.0.dispatcher.get().unwrap().clone();
        let second = app.clone();

        drop(app);
        tokio::task::yield_now().await;
        assert!(!dispatcher.is_finished(), "a clone still holds the app");

        drop(second);
        for _ in 0..100 {
            if dispatcher.is_finished() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("dispatcher still running after the last CoreApp handle dropped");
    }

    #[tokio::test]
    async fn send_message_returns_reply_with_no_tool_calls() {
        let (builder, _dir) = builder().await;
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(response(Some("你好呀"), vec![]));
        let app = builder.chat_model_override(mock).build().await.unwrap();

        let turn = app.send_message("你好").await.unwrap();
        assert_eq!(turn.reply, "你好呀");
        assert!(turn.activities.is_empty());
    }

    #[tokio::test]
    async fn send_message_substitutes_empty_reply_when_content_is_none() {
        let (builder, _dir) = builder().await;
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(response(None, vec![]));
        let app = builder.chat_model_override(mock).build().await.unwrap();

        let turn = app.send_message("你好").await.unwrap();
        assert_eq!(turn.reply, EMPTY_REPLY);
    }

    #[test]
    fn user_time_tag_uses_the_logical_date() {
        let boundary = NaiveTime::from_hms_opt(4, 0, 0).unwrap();
        let at = |h, m| {
            NaiveDate::from_ymd_opt(2026, 9, 29)
                .unwrap()
                .and_hms_opt(h, m, 0)
                .unwrap()
        };
        assert_eq!(
            user_time_tag(at(10, 43), boundary),
            "[sent 2026-09-29 10:43, logical date 2026-09-29]"
        );
        // Before the day boundary: still the previous day.
        assert_eq!(
            user_time_tag(at(3, 30), boundary),
            "[sent 2026-09-29 03:30, logical date 2026-09-28]"
        );
    }

    #[tokio::test]
    async fn send_message_tags_user_messages_with_their_own_send_time() {
        let dir = crate::db::TestDb::new().await;
        let clock = Arc::new(FixedClock::new(Utc::now()));
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(response(Some("好的"), vec![]));
        mock.push_response(response(Some("好的"), vec![]));
        let app = CoreAppBuilder::new(dir.config.clone())
            .clock(clock.clone())
            .secrets(Arc::new(MemorySecretStore::new()))
            .chat_model_override(mock.clone())
            .build()
            .await
            .unwrap();
        let boundary = app.0.settings.day_boundary().await.unwrap();
        let tag = |t: DateTime<Utc>| user_time_tag(t.with_timezone(&Local).naive_local(), boundary);

        let day1 = clock.now();
        app.send_message("早饭吃了牛肉饭").await.unwrap();
        clock.advance(chrono::Duration::days(1));
        let day2 = clock.now();
        app.send_message("我今天早饭吃了牛肉饭").await.unwrap();

        let requests = mock.received_requests();
        let users: Vec<&str> = requests
            .last()
            .unwrap()
            .messages
            .iter()
            .filter_map(|m| match m {
                Message::User { content } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            users,
            vec![
                format!("{} 早饭吃了牛肉饭", tag(day1)),
                format!("{} 我今天早饭吃了牛肉饭", tag(day2)),
            ]
        );
        assert_ne!(tag(day1), tag(day2));
    }

    #[tokio::test]
    async fn send_message_runs_a_core_tool_then_answers() {
        let (builder, _dir) = builder().await;
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(response(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "core_add_preference".into(),
                arguments: serde_json::json!({"kind": "dislike", "text": "香菜", "permanent": true}),
                provider_extra: None,
            }],
        ));
        mock.push_response(response(Some("记住啦"), vec![]));
        let app = builder.chat_model_override(mock).build().await.unwrap();

        let turn = app.send_message("我不吃香菜，记住").await.unwrap();
        assert_eq!(turn.reply, "记住啦");
        assert_eq!(turn.activities, vec!["已记住不喜欢：香菜"]);
        assert_eq!(app.preferences().list(10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tool_call_provider_extra_is_persisted_and_replayed() {
        let (builder, _dir) = builder().await;
        let mock = Arc::new(MockChatModel::new());
        let extra = serde_json::json!({"google": {"thought_signature": "c2ln"}});
        mock.push_response(response(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "core_add_preference".into(),
                arguments: serde_json::json!({"kind": "dislike", "text": "香菜", "permanent": true}),
                provider_extra: Some(extra.clone()),
            }],
        ));
        mock.push_response(response(Some("记住啦"), vec![]));
        let app = builder
            .chat_model_override(mock.clone())
            .build()
            .await
            .unwrap();

        app.send_message("我不吃香菜，记住").await.unwrap();

        // The second request rebuilds history from the `messages` table.
        let requests = mock.received_requests();
        let replayed = requests[1]
            .messages
            .iter()
            .find_map(|m| match m {
                Message::Assistant { tool_calls, .. } if !tool_calls.is_empty() => {
                    Some(tool_calls[0].provider_extra.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(replayed, Some(extra));
    }

    #[test]
    fn stored_tool_calls_without_provider_extra_still_deserialize() {
        let stored: Vec<StoredToolCall> =
            serde_json::from_str(r#"[{"id":"c","name":"n","arguments":{}}]"#).unwrap();
        assert_eq!(stored[0].provider_extra, None);
        let json = serde_json::to_string(&stored).unwrap();
        assert!(!json.contains("provider_extra"));
    }
}
