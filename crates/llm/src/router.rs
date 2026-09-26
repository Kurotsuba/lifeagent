//! `LlmRouter`: maps a `Role` to a `(provider, model)` binding and dispatches to it.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::LlmError;
use crate::provider::anthropic::AnthropicProvider;
use crate::provider::openai_compat::OpenAiCompatibleProvider;
use crate::types::{
    ChatModel, ChatRequest, ChatResponse, Effort, Message, ResponseFormat, Role, ToolSpec,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiCompatible,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleBinding {
    pub provider_id: String,
    pub model: String,
    /// OpenAI-compatible only; `None` falls back to the per-role default (§6.3 table).
    pub temperature: Option<f32>,
    /// Anthropic only; `None` falls back to the per-role default (§6.3 table).
    pub effort: Option<Effort>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsConfig {
    pub providers: Vec<ProviderConfig>,
    pub parser: RoleBinding,
    pub planner: RoleBinding,
    pub conversation: RoleBinding,
}

/// What a call site builds: `ChatRequest` without `model`, `temperature` or `effort` — the
/// router fills those in from the resolved `RoleBinding` and `ProviderKind` (§6.3). The spec's
/// own description only names `model`/`temperature` as excluded; `effort` is excluded too for
/// the same reason (see `docs/DECISIONS.md`).
#[derive(Debug, Clone, Default)]
pub struct RoleRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
    pub response_format: ResponseFormat,
}

/// Supplies API keys by provider id. Implemented in `lifeagent-core` by the secrets module
/// (§7.3), kept in this crate as a trait so the router stays decoupled from how keys are
/// stored.
pub trait KeyProvider: Send + Sync {
    fn get_key(&self, provider_id: &str) -> Result<String, LlmError>;
}

/// One row of `llm_usage` (§5), minus the `id`/`created_at` the sink implementation assigns.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageEvent {
    pub role: Role,
    pub provider_id: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u64,
    pub ok: bool,
}

/// Records a `UsageEvent` after every router call, success or failure. Implemented in
/// `lifeagent-core`, writing to the `llm_usage` table.
#[async_trait]
pub trait UsageSink: Send + Sync {
    async fn record(&self, event: UsageEvent);
}

fn default_temperature(role: Role) -> f32 {
    match role {
        Role::Parser => 0.0,
        Role::Planner => 0.7,
        Role::Conversation => 0.5,
    }
}

fn default_effort(role: Role) -> Effort {
    match role {
        Role::Parser => Effort::Low,
        Role::Planner => Effort::High,
        Role::Conversation => Effort::Medium,
    }
}

pub struct LlmRouter {
    providers: HashMap<String, Arc<dyn ChatModel>>,
    provider_kinds: HashMap<String, ProviderKind>,
    config: ModelsConfig,
    usage_sink: Arc<dyn UsageSink>,
}

impl LlmRouter {
    /// Builds real HTTP providers from `config`, fetching each one's API key via
    /// `key_provider`. Only providers some role is bound to are built, so a leftover provider
    /// with no key doesn't take down the ones actually in use.
    pub fn new(
        config: ModelsConfig,
        key_provider: &dyn KeyProvider,
        usage_sink: Arc<dyn UsageSink>,
    ) -> Result<Self, LlmError> {
        let bound =
            [&config.parser, &config.planner, &config.conversation].map(|b| b.provider_id.as_str());
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        for pc in config
            .providers
            .iter()
            .filter(|pc| bound.contains(&pc.id.as_str()))
        {
            let key = key_provider.get_key(&pc.id)?;
            let model: Arc<dyn ChatModel> = match pc.kind {
                ProviderKind::OpenAiCompatible => {
                    Arc::new(OpenAiCompatibleProvider::new(pc.base_url.clone(), key))
                }
                ProviderKind::Anthropic => {
                    Arc::new(AnthropicProvider::new(pc.base_url.clone(), key))
                }
            };
            providers.insert(pc.id.clone(), model);
        }
        Ok(Self::with_providers(config, providers, usage_sink))
    }

    /// Builds a router over pre-constructed providers (e.g. `MockChatModel` in tests),
    /// bypassing key lookup and real HTTP clients.
    pub fn with_providers(
        config: ModelsConfig,
        providers: HashMap<String, Arc<dyn ChatModel>>,
        usage_sink: Arc<dyn UsageSink>,
    ) -> Self {
        let provider_kinds = config
            .providers
            .iter()
            .map(|pc| (pc.id.clone(), pc.kind))
            .collect();
        Self {
            providers,
            provider_kinds,
            config,
            usage_sink,
        }
    }

    fn binding_for(&self, role: Role) -> &RoleBinding {
        match role {
            Role::Parser => &self.config.parser,
            Role::Planner => &self.config.planner,
            Role::Conversation => &self.config.conversation,
        }
    }

    pub async fn complete(&self, role: Role, req: RoleRequest) -> Result<ChatResponse, LlmError> {
        let binding = self.binding_for(role);
        let provider = self.providers.get(&binding.provider_id).ok_or_else(|| {
            LlmError::Config(format!(
                "no provider configured for id {:?}",
                binding.provider_id
            ))
        })?;
        let kind = *self
            .provider_kinds
            .get(&binding.provider_id)
            .ok_or_else(|| {
                LlmError::Config(format!(
                    "no provider configured for id {:?}",
                    binding.provider_id
                ))
            })?;

        let (temperature, effort) = match kind {
            ProviderKind::OpenAiCompatible => (
                Some(
                    binding
                        .temperature
                        .unwrap_or_else(|| default_temperature(role)),
                ),
                None,
            ),
            ProviderKind::Anthropic => (
                None,
                Some(binding.effort.unwrap_or_else(|| default_effort(role))),
            ),
        };

        let provider_id = binding.provider_id.clone();
        let model = binding.model.clone();

        let full_req = ChatRequest {
            model: model.clone(),
            messages: req.messages,
            tools: req.tools,
            temperature,
            max_tokens: req.max_tokens,
            response_format: req.response_format,
            effort,
        };

        let start = Instant::now();
        let result = provider.complete(full_req).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        let (prompt_tokens, completion_tokens, ok) = match &result {
            Ok(resp) => (resp.usage.prompt_tokens, resp.usage.completion_tokens, true),
            Err(_) => (0, 0, false),
        };
        // §12: one INFO line per call — metadata only, never message content.
        tracing::info!(
            ?role,
            provider = %provider_id,
            model = %model,
            prompt_tokens,
            completion_tokens,
            latency_ms,
            ok,
            "llm call"
        );

        self.usage_sink
            .record(UsageEvent {
                role,
                provider_id,
                model,
                prompt_tokens,
                completion_tokens,
                latency_ms,
                ok,
            })
            .await;

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockChatModel;
    use crate::types::{ChatResponse, Usage};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingUsageSink {
        events: Mutex<Vec<UsageEvent>>,
    }

    #[async_trait]
    impl UsageSink for RecordingUsageSink {
        async fn record(&self, event: UsageEvent) {
            self.events.lock().expect("mutex poisoned").push(event);
        }
    }

    fn config(kind: ProviderKind) -> ModelsConfig {
        let binding = |effort: Option<Effort>, temperature: Option<f32>| RoleBinding {
            provider_id: "p1".into(),
            model: "model-x".into(),
            temperature,
            effort,
        };
        ModelsConfig {
            providers: vec![ProviderConfig {
                id: "p1".into(),
                kind,
                base_url: "http://example.invalid".into(),
            }],
            parser: binding(None, None),
            planner: binding(None, None),
            conversation: binding(None, None),
        }
    }

    fn ok_response() -> ChatResponse {
        ChatResponse {
            content: Some("hi".into()),
            tool_calls: vec![],
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
            },
            finish_reason: "stop".into(),
        }
    }

    #[tokio::test]
    async fn resolves_openai_compatible_temperature_defaults_per_role() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response());
        mock.push_response(ok_response());
        mock.push_response(ok_response());
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        providers.insert("p1".into(), mock.clone());
        let router = LlmRouter::with_providers(
            config(ProviderKind::OpenAiCompatible),
            providers,
            Arc::new(RecordingUsageSink::default()),
        );

        router
            .complete(Role::Parser, RoleRequest::default())
            .await
            .unwrap();
        router
            .complete(Role::Planner, RoleRequest::default())
            .await
            .unwrap();
        router
            .complete(Role::Conversation, RoleRequest::default())
            .await
            .unwrap();

        let received = mock.received_requests();
        assert_eq!(received[0].temperature, Some(0.0));
        assert_eq!(received[0].effort, None);
        assert_eq!(received[1].temperature, Some(0.7));
        assert_eq!(received[2].temperature, Some(0.5));
    }

    #[tokio::test]
    async fn resolves_anthropic_effort_defaults_per_role() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response());
        mock.push_response(ok_response());
        mock.push_response(ok_response());
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        providers.insert("p1".into(), mock.clone());
        let router = LlmRouter::with_providers(
            config(ProviderKind::Anthropic),
            providers,
            Arc::new(RecordingUsageSink::default()),
        );

        router
            .complete(Role::Parser, RoleRequest::default())
            .await
            .unwrap();
        router
            .complete(Role::Planner, RoleRequest::default())
            .await
            .unwrap();
        router
            .complete(Role::Conversation, RoleRequest::default())
            .await
            .unwrap();

        let received = mock.received_requests();
        assert_eq!(received[0].effort, Some(Effort::Low));
        assert_eq!(received[0].temperature, None);
        assert_eq!(received[1].effort, Some(Effort::High));
        assert_eq!(received[2].effort, Some(Effort::Medium));
    }

    #[tokio::test]
    async fn explicit_binding_overrides_the_per_role_default() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response());
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        providers.insert("p1".into(), mock.clone());
        let mut cfg = config(ProviderKind::OpenAiCompatible);
        cfg.parser.temperature = Some(0.42);
        let router =
            LlmRouter::with_providers(cfg, providers, Arc::new(RecordingUsageSink::default()));

        router
            .complete(Role::Parser, RoleRequest::default())
            .await
            .unwrap();

        assert_eq!(mock.received_requests()[0].temperature, Some(0.42));
    }

    #[tokio::test]
    async fn records_usage_on_success() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response());
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        providers.insert("p1".into(), mock);
        let sink = Arc::new(RecordingUsageSink::default());
        let router = LlmRouter::with_providers(
            config(ProviderKind::OpenAiCompatible),
            providers,
            sink.clone(),
        );

        router
            .complete(Role::Parser, RoleRequest::default())
            .await
            .unwrap();

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].role, Role::Parser);
        assert_eq!(events[0].provider_id, "p1");
        assert_eq!(events[0].model, "model-x");
        assert_eq!(events[0].prompt_tokens, 10);
        assert_eq!(events[0].completion_tokens, 5);
        assert!(events[0].ok);
    }

    #[tokio::test]
    async fn records_usage_with_zero_tokens_on_failure() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_error(LlmError::Auth);
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        providers.insert("p1".into(), mock);
        let sink = Arc::new(RecordingUsageSink::default());
        let router = LlmRouter::with_providers(
            config(ProviderKind::OpenAiCompatible),
            providers,
            sink.clone(),
        );

        let result = router.complete(Role::Parser, RoleRequest::default()).await;

        assert!(result.is_err());
        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].ok);
        assert_eq!(events[0].prompt_tokens, 0);
        assert_eq!(events[0].completion_tokens, 0);
    }

    struct KeysFor(&'static [&'static str]);

    impl KeyProvider for KeysFor {
        fn get_key(&self, provider_id: &str) -> Result<String, LlmError> {
            if self.0.contains(&provider_id) {
                Ok("key".into())
            } else {
                Err(LlmError::Config(format!("no key for {provider_id}")))
            }
        }
    }

    #[test]
    fn new_skips_unbound_providers_without_a_key() {
        let mut cfg = config(ProviderKind::OpenAiCompatible);
        cfg.providers.push(ProviderConfig {
            id: "unused".into(),
            kind: ProviderKind::Anthropic,
            base_url: "http://example.invalid".into(),
        });
        let sink = Arc::new(RecordingUsageSink::default());
        assert!(LlmRouter::new(cfg, &KeysFor(&["p1"]), sink).is_ok());
    }

    #[test]
    fn new_fails_when_a_bound_provider_has_no_key() {
        let sink = Arc::new(RecordingUsageSink::default());
        let result = LlmRouter::new(config(ProviderKind::OpenAiCompatible), &KeysFor(&[]), sink);
        assert!(matches!(result, Err(LlmError::Config(_))));
    }

    #[tokio::test]
    async fn unknown_provider_id_returns_config_error() {
        let providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        let mut cfg = config(ProviderKind::OpenAiCompatible);
        cfg.parser.provider_id = "missing".into();
        let router =
            LlmRouter::with_providers(cfg, providers, Arc::new(RecordingUsageSink::default()));

        let err = router
            .complete(Role::Parser, RoleRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Config(_)));
    }
}
