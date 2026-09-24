//! Anthropic Messages provider.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.2.2. Not reachable by pointing the
//! OpenAI-compatible provider at a different `base_url` — the wire format differs in message
//! shape, tool schema nesting, tool-result placement and parameter surface.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::send_with_retries;
use crate::error::LlmError;
use crate::types::{ChatModel, ChatRequest, ChatResponse, Effort, Message, ToolCall, Usage};

/// Default `base_url` when the user configures an Anthropic provider (§6.2.2). Unlike the
/// OpenAI-compatible `base_url`, this does not include `/v1` — the provider appends it.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

const DEFAULT_MAX_TOKENS: u32 = 16000;
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ChatModel for AnthropicProvider {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let body = build_request_body(&req);
        let url = format!("{}/v1/messages", self.base_url);
        let api_key = &self.api_key;
        let client = &self.client;
        let response = send_with_retries(|| {
            client
                .post(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(&body)
        })
        .await?;
        parse_response_body(&response)
    }
}

fn effort_str(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}

/// Hoists every `Message::System` into a single joined `system` string and maps the rest,
/// collapsing runs of consecutive `Message::Tool` into one `user` message with `tool_result`
/// blocks (§6.2.2 "Tool-result merging").
fn build_system_and_messages(messages: &[Message]) -> (Option<String>, Vec<Value>) {
    let system_parts: Vec<&str> = messages
        .iter()
        .filter_map(|m| match m {
            Message::System { content } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    let non_system: Vec<&Message> = messages
        .iter()
        .filter(|m| !matches!(m, Message::System { .. }))
        .collect();

    let mut out: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < non_system.len() {
        match non_system[i] {
            Message::System { .. } => unreachable!("filtered out above"),
            Message::User { content } => {
                out.push(json!({"role": "user", "content": content}));
                i += 1;
            }
            Message::Assistant {
                content,
                tool_calls,
            } => {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = content {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for tc in tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                out.push(json!({"role": "assistant", "content": blocks}));
                i += 1;
            }
            Message::Tool { .. } => {
                let mut blocks: Vec<Value> = Vec::new();
                while let Some(Message::Tool {
                    tool_call_id,
                    content,
                }) = non_system.get(i)
                {
                    blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content,
                    }));
                    i += 1;
                }
                out.push(json!({"role": "user", "content": blocks}));
            }
        }
    }

    (system, out)
}

/// Pure mapping from `ChatRequest` to the Anthropic Messages request body (§6.2.2). Separated
/// from `complete` so it can be tested against fixtures without a live endpoint.
pub(crate) fn build_request_body(req: &ChatRequest) -> Value {
    let (system, messages) = build_system_and_messages(&req.messages);

    let mut body = serde_json::Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert(
        "max_tokens".into(),
        json!(req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    body.insert("messages".into(), json!(messages));
    if let Some(system) = system {
        body.insert("system".into(), json!(system));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        body.insert("tools".into(), json!(tools));
    }
    if let Some(effort) = req.effort {
        body.insert(
            "output_config".into(),
            json!({"effort": effort_str(effort)}),
        );
    }
    // No temperature/top_p/top_k: current models reject them (§6.2.2), and no JSON-mode
    // field either — `complete_json` (§6.4) carries `ResponseFormat::JsonObject` via prompt.

    Value::Object(body)
}

/// Pure mapping from an Anthropic Messages response body to `ChatResponse` (§6.2.2).
pub(crate) fn parse_response_body(body: &Value) -> Result<ChatResponse, LlmError> {
    let content_blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| LlmError::InvalidResponse("missing content".into()))?;

    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in content_blocks {
        match block.get("type").and_then(|v| v.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    text_parts.push(t.to_string());
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LlmError::InvalidResponse("tool_use missing id".into()))?
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LlmError::InvalidResponse("tool_use missing name".into()))?
                    .to_string();
                let arguments = block.get("input").cloned().unwrap_or_else(|| json!({}));
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {}
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let finish_reason = body
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let usage = body
        .get("usage")
        .map(|u| Usage {
            prompt_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            completion_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        })
        .unwrap_or_default();

    Ok(ChatResponse {
        content,
        tool_calls,
        usage,
        finish_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatRequest, ResponseFormat, ToolSpec};

    fn base_request(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "claude-sonnet-5".into(),
            messages,
            tools: vec![],
            temperature: None,
            max_tokens: None,
            response_format: ResponseFormat::Text,
            effort: None,
        }
    }

    #[test]
    fn system_messages_are_hoisted_and_joined() {
        let req = base_request(vec![
            Message::System {
                content: "one".into(),
            },
            Message::System {
                content: "two".into(),
            },
            Message::User {
                content: "hi".into(),
            },
        ]);
        let body = build_request_body(&req);
        assert_eq!(body["system"], json!("one\n\ntwo"));
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            body["messages"][0],
            json!({"role": "user", "content": "hi"})
        );
    }

    #[test]
    fn no_system_field_when_no_system_messages() {
        let req = base_request(vec![Message::User {
            content: "hi".into(),
        }]);
        let body = build_request_body(&req);
        assert!(!body.as_object().unwrap().contains_key("system"));
    }

    #[test]
    fn max_tokens_defaults_to_16000_when_unset() {
        let req = base_request(vec![Message::User {
            content: "hi".into(),
        }]);
        let body = build_request_body(&req);
        assert_eq!(body["max_tokens"], json!(16000));
    }

    #[test]
    fn max_tokens_is_passed_through_when_set() {
        let mut req = base_request(vec![Message::User {
            content: "hi".into(),
        }]);
        req.max_tokens = Some(2048);
        let body = build_request_body(&req);
        assert_eq!(body["max_tokens"], json!(2048));
    }

    #[test]
    fn assistant_message_maps_text_then_tool_use_blocks() {
        let req = base_request(vec![Message::Assistant {
            content: Some("let me check".into()),
            tool_calls: vec![ToolCall {
                id: "toolu_1".into(),
                name: "resolve_food".into(),
                arguments: json!({"name": "鸡蛋"}),
            }],
        }]);
        let body = build_request_body(&req);
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[0], json!({"type": "text", "text": "let me check"}));
        assert_eq!(
            blocks[1],
            json!({"type": "tool_use", "id": "toolu_1", "name": "resolve_food", "input": {"name": "鸡蛋"}})
        );
    }

    #[test]
    fn consecutive_tool_messages_collapse_into_one_user_message() {
        let req = base_request(vec![
            Message::Assistant {
                content: None,
                tool_calls: vec![
                    ToolCall {
                        id: "toolu_1".into(),
                        name: "a".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "toolu_2".into(),
                        name: "b".into(),
                        arguments: json!({}),
                    },
                ],
            },
            Message::Tool {
                tool_call_id: "toolu_1".into(),
                content: "result a".into(),
            },
            Message::Tool {
                tool_call_id: "toolu_2".into(),
                content: "result b".into(),
            },
            Message::User {
                content: "thanks".into(),
            },
        ]);
        let body = build_request_body(&req);
        let messages = body["messages"].as_array().unwrap();
        // assistant, one merged user (tool results), one plain user — never split.
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], json!("user"));
        let blocks = messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0],
            json!({"type": "tool_result", "tool_use_id": "toolu_1", "content": "result a"})
        );
        assert_eq!(
            blocks[1],
            json!({"type": "tool_result", "tool_use_id": "toolu_2", "content": "result b"})
        );
        assert_eq!(messages[2], json!({"role": "user", "content": "thanks"}));
    }

    #[test]
    fn tool_spec_maps_flat_with_input_schema() {
        let mut req = base_request(vec![Message::User {
            content: "hi".into(),
        }]);
        req.tools = vec![ToolSpec {
            name: "resolve_food".into(),
            description: "resolves a food".into(),
            parameters: json!({"type": "object", "properties": {}}),
        }];
        let body = build_request_body(&req);
        assert_eq!(
            body["tools"][0],
            json!({
                "name": "resolve_food",
                "description": "resolves a food",
                "input_schema": {"type": "object", "properties": {}}
            })
        );
    }

    #[test]
    fn no_temperature_or_response_format_field_is_ever_sent() {
        let mut req = base_request(vec![Message::User {
            content: "hi".into(),
        }]);
        req.temperature = Some(0.7);
        req.response_format = ResponseFormat::JsonObject;
        let body = build_request_body(&req);
        let obj = body.as_object().unwrap();
        assert!(!obj.contains_key("temperature"));
        assert!(!obj.contains_key("top_p"));
        assert!(!obj.contains_key("top_k"));
        assert!(!obj.contains_key("response_format"));
    }

    #[test]
    fn effort_maps_to_output_config() {
        let mut req = base_request(vec![Message::User {
            content: "hi".into(),
        }]);
        req.effort = Some(Effort::High);
        let body = build_request_body(&req);
        assert_eq!(body["output_config"], json!({"effort": "high"}));
    }

    /// Fixture: a typical Messages response with one text block.
    const TEXT_RESPONSE_FIXTURE: &str = r#"{
        "content": [{"type": "text", "text": "总共摄入 500 千卡"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 120, "output_tokens": 30}
    }"#;

    #[test]
    fn parse_response_body_maps_text_response_fixture() {
        let body: Value = serde_json::from_str(TEXT_RESPONSE_FIXTURE).unwrap();
        let resp = parse_response_body(&body).unwrap();
        assert_eq!(resp.content.as_deref(), Some("总共摄入 500 千卡"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "end_turn");
        assert_eq!(
            resp.usage,
            Usage {
                prompt_tokens: 120,
                completion_tokens: 30
            }
        );
    }

    /// Fixture: a response with a text block followed by a tool_use block, `input` a JSON
    /// object (not a string) — Anthropic needs no argument-parsing step.
    const TOOL_USE_RESPONSE_FIXTURE: &str = r#"{
        "content": [
            {"type": "text", "text": "let me look that up"},
            {"type": "tool_use", "id": "toolu_1", "name": "resolve_food", "input": {"name": "鸡蛋", "grams": 50}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 300, "output_tokens": 45}
    }"#;

    #[test]
    fn parse_response_body_maps_tool_use_response_fixture() {
        let body: Value = serde_json::from_str(TOOL_USE_RESPONSE_FIXTURE).unwrap();
        let resp = parse_response_body(&body).unwrap();
        assert_eq!(resp.content.as_deref(), Some("let me look that up"));
        assert_eq!(resp.finish_reason, "tool_use");
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "toolu_1");
        assert_eq!(resp.tool_calls[0].name, "resolve_food");
        assert_eq!(
            resp.tool_calls[0].arguments,
            json!({"name": "鸡蛋", "grams": 50})
        );
    }

    #[test]
    fn parse_response_body_handles_refusal_as_empty_content_and_no_tool_calls() {
        let body: Value = serde_json::from_str(
            r#"{"content": [], "stop_reason": "refusal", "usage": {"input_tokens": 10, "output_tokens": 0}}"#,
        )
        .unwrap();
        let resp = parse_response_body(&body).unwrap();
        assert_eq!(resp.content, None);
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "refusal");
    }

    #[test]
    fn parse_response_body_returns_invalid_response_when_content_is_missing() {
        let body: Value = serde_json::from_str(r#"{"stop_reason": "end_turn"}"#).unwrap();
        let err = parse_response_body(&body).unwrap_err();
        assert!(matches!(err, LlmError::InvalidResponse(_)));
    }

    #[test]
    fn parse_response_body_defaults_usage_when_absent() {
        let body: Value =
            serde_json::from_str(r#"{"content": [{"type": "text", "text": "hi"}]}"#).unwrap();
        let resp = parse_response_body(&body).unwrap();
        assert_eq!(resp.usage, Usage::default());
    }
}
