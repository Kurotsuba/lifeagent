//! Core request/response types shared by every provider (`ChatModel`, `Message`, `ToolSpec`, ...).
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.1.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::LlmError;

/// Which of the app's three LLM-calling roles a request is for. Selects a `RoleBinding` in
/// `ModelsConfig` (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Parser,
    Planner,
    Conversation,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Must match `^[a-z][a-z0-9_]{0,63}$`.
    pub name: String,
    pub description: String,
    /// A JSON Schema object.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseFormat {
    #[default]
    Text,
    JsonObject,
}

/// Reasoning effort for providers that expose it (Anthropic only, §6.2.2/§6.3). Not part of
/// the spec's §6.1 listing, which only shows `Effort` defined under the router (§6.3); it is
/// defined here instead because `ChatRequest` needs it (see `docs/DECISIONS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    /// OpenAI-compatible only; ignored by the Anthropic provider.
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub response_format: ResponseFormat,
    /// Anthropic only; ignored by the OpenAI-compatible provider. See `docs/DECISIONS.md` for
    /// why this field exists even though §6.1's code block doesn't list it.
    pub effort: Option<Effort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: String,
}

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
}
