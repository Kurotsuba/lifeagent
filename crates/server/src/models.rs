//! Where the server's model keys come from, and the echo model for local development.

use async_trait::async_trait;
use lifeagent_core::error::CoreError;
use lifeagent_core::secrets::SecretStore;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::types::{ChatModel, ChatRequest, ChatResponse, Message, Usage};

/// API keys from the environment: provider `gemini` reads `LIFEAGENT_KEY_GEMINI` (uppercased,
/// anything but ASCII letters and digits becomes `_`). On Cloud Run these come from Secret
/// Manager via `--set-secrets`. Read-only: users can't set keys in the web build.
pub struct EnvSecretStore;

pub fn key_var(provider_id: &str) -> String {
    let suffix: String = provider_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("LIFEAGENT_KEY_{suffix}")
}

impl SecretStore for EnvSecretStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, CoreError> {
        Ok(std::env::var(key_var(provider_id))
            .ok()
            .filter(|k| !k.is_empty()))
    }

    fn set(&self, _provider_id: &str, _key: &str) -> Result<(), CoreError> {
        Err(CoreError::Secret(
            "the server's keys come from the environment".into(),
        ))
    }

    fn delete(&self, _provider_id: &str) -> Result<(), CoreError> {
        Err(CoreError::Secret(
            "the server's keys come from the environment".into(),
        ))
    }
}

/// `LIFEAGENT_MOCK_LLM=1`: answers every request by echoing the last user message, never with
/// tool calls, so chat plumbing can be tried without a model (like the CLI's `--mock`). Plan
/// generation fails, since the planner expects structured JSON.
pub struct EchoModel;

#[async_trait]
impl ChatModel for EchoModel {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let last_user_text = req
            .messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { content } => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_default();
        Ok(ChatResponse {
            content: Some(format!("(mock echo) {last_user_text}")),
            tool_calls: vec![],
            usage: Usage::default(),
            finish_reason: "stop".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_variables_are_derived_from_the_provider_id() {
        assert_eq!(key_var("gemini"), "LIFEAGENT_KEY_GEMINI");
        assert_eq!(key_var("vertex-ai.1"), "LIFEAGENT_KEY_VERTEX_AI_1");
    }
}
