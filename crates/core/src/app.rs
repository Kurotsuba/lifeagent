//! `CoreAppBuilder` / `CoreApp`: composition root and orchestrator loop, per
//! `docs/PHASE1_SPEC.md` §7.1/§7.11.
//!
//! Only the builder and `send_message` land in M3, per the milestone note in §14 — the rest
//! of `CoreApp`'s eventual surface (plan methods, `today_view`, `start_scheduler`, ...) is
//! M4/M5 work and is added incrementally rather than stubbed out here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lifeagent_llm::router::{
    LlmRouter, ModelsConfig, ProviderConfig, ProviderKind, RoleBinding, RoleRequest,
};
use lifeagent_llm::types::{ChatModel, Message, ResponseFormat, Role, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{broadcast, Mutex};

use crate::clock::{Clock, SystemClock};
use crate::context;
use crate::conversation::{ConversationStore, MessageRole, NewMessage, StoredMessage};
use crate::db;
use crate::error::CoreError;
use crate::events::{AppEvent, EventStore};
use crate::goals::GoalsService;
use crate::memory::MemoryService;
use crate::plugin::{Plugin, PluginContext, PluginError};
use crate::profile::ProfileService;
use crate::secrets::{KeyringSecretStore, SecretKeyProvider, SecretStore};
use crate::settings::Settings;
use crate::tools::ToolRegistry;
use crate::usage::DbUsageSink;

/// Loop cap for `send_message` (§7.11): stop after this many tool-call rounds rather than
/// let a confused model spin forever.
const MAX_ITERATIONS: u32 = 6;

const EMPTY_REPLY: &str = "（模型没有返回内容，请再试一次。）";
const TOO_MANY_STEPS_REPLY: &str = "（处理步骤过多，已停止。请换个说法再试一次。）";

/// The result of one `send_message` turn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatTurn {
    pub reply: String,
    pub activities: Vec<String>,
}

pub struct CoreAppBuilder {
    data_dir: PathBuf,
    clock: Arc<dyn Clock>,
    secrets: Arc<dyn SecretStore>,
    plugins: Vec<Arc<dyn Plugin>>,
    chat_model_override: Option<Arc<dyn ChatModel>>,
}

impl CoreAppBuilder {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            clock: Arc::new(SystemClock),
            secrets: Arc::new(KeyringSecretStore),
            plugins: Vec::new(),
            chat_model_override: None,
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

    /// Bypasses real provider construction entirely: every role is bound to `m`. Used by
    /// tests and the CLI's `--mock` flag.
    pub fn chat_model_override(mut self, m: Arc<dyn ChatModel>) -> Self {
        self.chat_model_override = Some(m);
        self
    }

    /// Opens the database, runs core and plugin migrations, and builds the tool registry and
    /// (if configured) the LLM router.
    pub async fn build(self) -> Result<CoreApp, CoreError> {
        let db = db::open(&self.data_dir).await?;
        let settings = Settings::new(db.clone());
        let events = EventStore::new(db.clone(), self.clock.clone(), settings.clone());
        let profile = ProfileService::new(db.clone(), self.clock.clone());
        let goals = GoalsService::new(db.clone(), self.clock.clone());
        let memory = MemoryService::new(db.clone(), self.clock.clone());
        let conversation = ConversationStore::new(db.clone(), self.clock.clone());

        for plugin in &self.plugins {
            crate::plugin::apply_migrations(&db, self.clock.as_ref(), &plugin.manifest()).await?;
        }

        let registry = ToolRegistry::build(&self.plugins)?;
        let router = self.build_router(&db, &settings).await?;

        Ok(CoreApp(Arc::new(CoreAppInner {
            db,
            clock: self.clock,
            settings,
            secrets: self.secrets,
            router,
            events,
            profile,
            goals,
            memory,
            conversation,
            plugins: self.plugins,
            registry,
            send_lock: Mutex::new(()),
        })))
    }

    /// A mock override always wins (tests, `--mock`); otherwise builds real providers from
    /// `settings.models()`, or `None` if onboarding hasn't configured any yet.
    async fn build_router(
        &self,
        db: &SqlitePool,
        settings: &Settings,
    ) -> Result<Option<Arc<LlmRouter>>, CoreError> {
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
                }],
                parser: binding.clone(),
                planner: binding.clone(),
                conversation: binding,
            };
            let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
            providers.insert("mock".into(), model.clone());
            return Ok(Some(Arc::new(LlmRouter::with_providers(
                config, providers, usage_sink,
            ))));
        }

        match settings.models().await? {
            Some(config) => {
                let key_provider = SecretKeyProvider(self.secrets.clone());
                let router = LlmRouter::new(config, &key_provider, usage_sink)?;
                Ok(Some(Arc::new(router)))
            }
            None => Ok(None),
        }
    }
}

struct CoreAppInner {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
    settings: Settings,
    secrets: Arc<dyn SecretStore>,
    router: Option<Arc<LlmRouter>>,
    events: EventStore,
    profile: ProfileService,
    goals: GoalsService,
    memory: MemoryService,
    conversation: ConversationStore,
    plugins: Vec<Arc<dyn Plugin>>,
    registry: ToolRegistry,
    /// Serializes `send_message` calls (§7.11: "one turn at a time").
    send_lock: Mutex<()>,
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
    pub fn memory(&self) -> &MemoryService {
        &self.0.memory
    }
    pub fn conversation(&self) -> &ConversationStore {
        &self.0.conversation
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.0.events.subscribe()
    }

    fn plugin_context(&self) -> Result<PluginContext, CoreError> {
        let llm = self
            .0
            .router
            .clone()
            .ok_or_else(|| CoreError::Validation("models are not configured yet".into()))?;
        Ok(PluginContext {
            db: self.0.db.clone(),
            events: self.0.events.clone(),
            llm,
            clock: self.0.clock.clone(),
            settings: self.0.settings.clone(),
            profile: self.0.profile.clone(),
            goals: self.0.goals.clone(),
            memory: self.0.memory.clone(),
        })
    }

    /// Runs one conversation turn (§7.11): stores the user message, then alternates model
    /// calls and tool dispatch — tool calls within a round run sequentially, in call order —
    /// until the model stops calling tools or the iteration cap is hit.
    pub async fn send_message(&self, text: &str) -> Result<ChatTurn, CoreError> {
        let _guard = self.0.send_lock.lock().await;

        self.0
            .conversation
            .append(NewMessage {
                role: MessageRole::User,
                content: Some(text.to_string()),
                ..Default::default()
            })
            .await?;

        let plugin_ctx = self.plugin_context()?;
        let router = plugin_ctx.llm.clone();
        let tool_specs = self.0.registry.specs();
        let mut activities = Vec::new();

        for _ in 0..MAX_ITERATIONS {
            let system_prompt = context::build_system_prompt(&plugin_ctx, &self.0.plugins).await?;
            let history = self.0.conversation.history_window().await?;
            let messages = Self::to_llm_messages(system_prompt, &history)?;

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
            })
            .collect();
        Ok(Some(serde_json::to_string(&stored)?))
    }

    fn to_llm_messages(
        system_prompt: String,
        history: &[StoredMessage],
    ) -> Result<Vec<Message>, CoreError> {
        let mut messages = vec![Message::System {
            content: system_prompt,
        }];
        for m in history {
            match m.role {
                MessageRole::User => messages.push(Message::User {
                    content: m.content.clone().unwrap_or_default(),
                }),
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

    async fn builder() -> (CoreAppBuilder, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let builder = CoreAppBuilder::new(dir.path().to_path_buf())
            .clock(Arc::new(FixedClock::new(Utc::now())))
            .secrets(Arc::new(MemorySecretStore::new()));
        (builder, dir)
    }

    #[tokio::test]
    async fn send_message_without_models_configured_errors() {
        let (builder, _dir) = builder().await;
        let app = builder.build().await.unwrap();

        assert!(matches!(
            app.send_message("你好").await,
            Err(CoreError::Validation(_))
        ));
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

    #[tokio::test]
    async fn send_message_runs_a_core_tool_then_answers() {
        let (builder, _dir) = builder().await;
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(response(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "core_remember".into(),
                arguments: serde_json::json!({"note": "不吃香菜"}),
            }],
        ));
        mock.push_response(response(Some("记住啦"), vec![]));
        let app = builder.chat_model_override(mock).build().await.unwrap();

        let turn = app.send_message("我不吃香菜，记住").await.unwrap();
        assert_eq!(turn.reply, "记住啦");
        assert_eq!(turn.activities, vec!["已记住：不吃香菜"]);
        assert_eq!(app.memory().list(10).await.unwrap().len(), 1);
    }
}
