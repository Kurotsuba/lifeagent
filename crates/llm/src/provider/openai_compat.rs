//! OpenAI-compatible Chat Completions provider.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.2.1.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::send_with_retries;
use crate::error::LlmError;
use crate::types::{
    ChatModel, ChatRequest, ChatResponse, Message, ResponseFormat, ToolCall, Usage,
};

pub struct OpenAiCompatibleProvider {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            // `complete` appends `/chat/completions`; a pasted trailing slash would double it.
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ChatModel for OpenAiCompatibleProvider {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let body = build_request_body(&req);
        let url = format!("{}/chat/completions", self.base_url);
        let api_key = &self.api_key;
        let client = &self.client;
        let response =
            send_with_retries(|| client.post(&url).bearer_auth(api_key).json(&body)).await?;
        parse_response_body(&response)
    }
}

fn message_to_json(msg: &Message) -> Value {
    match msg {
        Message::System { content } => json!({"role": "system", "content": content}),
        Message::User { content } => json!({"role": "user", "content": content}),
        Message::Assistant {
            content,
            tool_calls,
        } => {
            let mut obj = serde_json::Map::new();
            obj.insert("role".into(), json!("assistant"));
            obj.insert(
                "content".into(),
                content.as_ref().map(|c| json!(c)).unwrap_or(Value::Null),
            );
            if !tool_calls.is_empty() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        let arguments = serde_json::to_string(&tc.arguments)
                            .unwrap_or_else(|_| "{}".to_string());
                        let mut call = json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": arguments,
                            }
                        });
                        if let (Some(extra), Some(obj)) = (&tc.provider_extra, call.as_object_mut())
                        {
                            obj.insert("extra_content".into(), extra.clone());
                        }
                        call
                    })
                    .collect();
                obj.insert("tool_calls".into(), json!(calls));
            }
            Value::Object(obj)
        }
        Message::Tool {
            tool_call_id,
            content,
        } => json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "content": content,
        }),
    }
}

/// Pure mapping from `ChatRequest` to the OpenAI Chat Completions request body (§6.2.1).
/// Separated from `complete` so it can be tested against fixtures without a live endpoint.
pub(crate) fn build_request_body(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req.messages.iter().map(message_to_json).collect();

    let mut body = serde_json::Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("messages".into(), json!(messages));

    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();
        body.insert("tools".into(), json!(tools));
    }

    if let Some(temperature) = req.temperature {
        if !ignores_temperature(&req.model) {
            body.insert("temperature".into(), json!(temperature));
        }
    }
    if let Some(max_tokens) = req.max_tokens {
        body.insert("max_tokens".into(), json!(max_tokens));
    }
    if req.response_format == ResponseFormat::JsonObject {
        body.insert("response_format".into(), json!({"type": "json_object"}));
    }

    Value::Object(body)
}

/// Gemini 3 models are tuned for their default temperature (1.0); Google warns that lower
/// values can cause looping or degraded reasoning, and the per-role defaults (§6.3) go down to
/// 0.0. Like the Anthropic provider's `effort` denylist, this is a model fact handled here
/// rather than in the router. Matches both `gemini-3…` and `models/gemini-3…`.
fn ignores_temperature(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model
        .strip_prefix("models/")
        .unwrap_or(&model)
        .starts_with("gemini-3")
}

/// Pure mapping from an OpenAI Chat Completions response body to `ChatResponse` (§6.2.1).
pub(crate) fn parse_response_body(body: &Value) -> Result<ChatResponse, LlmError> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| LlmError::InvalidResponse("missing choices[0]".into()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| LlmError::InvalidResponse("missing choices[0].message".into()))?;

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .map(str::to_string);

    let tool_calls = match message.get("tool_calls").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, tc)| parse_tool_call(i, tc))
            .collect::<Result<Vec<_>, LlmError>>()?,
        None => Vec::new(),
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let usage = body
        .get("usage")
        .map(|u| Usage {
            prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            completion_tokens: u
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
        })
        .unwrap_or_default();

    Ok(ChatResponse {
        content,
        tool_calls,
        usage,
        finish_reason,
    })
}

/// A missing or empty `id` gets a positional fallback (`call_{index}`): some compatible
/// endpoints omit it, and it only has to pair this call with its `tool` result message.
fn parse_tool_call(index: usize, tc: &Value) -> Result<ToolCall, LlmError> {
    let id = match tc.get("id").and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => format!("call_{index}"),
    };
    let function = tc
        .get("function")
        .ok_or_else(|| LlmError::InvalidResponse("tool call missing function".into()))?;
    let name = function
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmError::InvalidResponse("tool call missing function.name".into()))?
        .to_string();
    let arguments_str = function
        .get("arguments")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmError::InvalidResponse("tool call missing function.arguments".into()))?;
    let arguments: Value = serde_json::from_str(arguments_str)
        .map_err(|e| LlmError::InvalidResponse(format!("invalid tool call arguments JSON: {e}")))?;
    Ok(ToolCall {
        id,
        name,
        arguments,
        provider_extra: tc.get("extra_content").filter(|v| !v.is_null()).cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolSpec;

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        let p = OpenAiCompatibleProvider::new("https://api.deepseek.com/v1/".into(), "k".into());
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
    }

    #[test]
    fn build_request_body_maps_every_message_variant() {
        let req = ChatRequest {
            model: "gpt-test".into(),
            messages: vec![
                Message::System {
                    content: "sys".into(),
                },
                Message::User {
                    content: "hi".into(),
                },
                Message::Assistant {
                    content: Some("hello".into()),
                    tool_calls: vec![],
                },
                Message::Assistant {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "log_meal".into(),
                        arguments: json!({"food": "egg"}),
                        provider_extra: None,
                    }],
                },
                Message::Tool {
                    tool_call_id: "call_1".into(),
                    content: "ok".into(),
                },
            ],
            tools: vec![ToolSpec {
                name: "log_meal".into(),
                description: "logs a meal".into(),
                parameters: json!({"type": "object", "properties": {}}),
            }],
            temperature: Some(0.7),
            max_tokens: Some(1000),
            response_format: ResponseFormat::JsonObject,
            effort: None,
        };

        let body = build_request_body(&req);

        assert_eq!(body["model"], json!("gpt-test"));
        assert_eq!(
            body["messages"][0],
            json!({"role": "system", "content": "sys"})
        );
        assert_eq!(
            body["messages"][1],
            json!({"role": "user", "content": "hi"})
        );
        assert_eq!(
            body["messages"][2],
            json!({"role": "assistant", "content": "hello"})
        );
        assert_eq!(body["messages"][3]["role"], json!("assistant"));
        assert_eq!(body["messages"][3]["content"], Value::Null);
        assert_eq!(body["messages"][3]["tool_calls"][0]["id"], json!("call_1"));
        assert_eq!(
            body["messages"][3]["tool_calls"][0]["type"],
            json!("function")
        );
        assert_eq!(
            body["messages"][3]["tool_calls"][0]["function"]["name"],
            json!("log_meal")
        );
        let args_str = body["messages"][3]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(args_str).unwrap(),
            json!({"food": "egg"})
        );
        assert_eq!(
            body["messages"][4],
            json!({"role": "tool", "tool_call_id": "call_1", "content": "ok"})
        );

        assert_eq!(body["tools"][0]["type"], json!("function"));
        assert_eq!(body["tools"][0]["function"]["name"], json!("log_meal"));
        // f32 -> f64 widening (via serde_json::Number) doesn't round-trip exactly.
        assert_eq!(body["temperature"].as_f64().unwrap() as f32, 0.7f32);
        assert_eq!(body["max_tokens"], json!(1000));
        assert_eq!(body["response_format"], json!({"type": "json_object"}));
    }

    #[test]
    fn build_request_body_omits_optional_fields_when_absent() {
        let req = ChatRequest {
            model: "gpt-test".into(),
            messages: vec![Message::User {
                content: "hi".into(),
            }],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            response_format: ResponseFormat::Text,
            effort: None,
        };

        let body = build_request_body(&req);
        let obj = body.as_object().unwrap();
        assert!(!obj.contains_key("tools"));
        assert!(!obj.contains_key("temperature"));
        assert!(!obj.contains_key("max_tokens"));
        assert!(!obj.contains_key("response_format"));
    }

    /// Fixture: a typical Chat Completions response with plain text content and usage.
    const TEXT_RESPONSE_FIXTURE: &str = r#"{
        "choices": [
            {
                "message": {"role": "assistant", "content": "总共摄入 500 千卡"},
                "finish_reason": "stop"
            }
        ],
        "usage": {"prompt_tokens": 120, "completion_tokens": 30}
    }"#;

    #[test]
    fn parse_response_body_maps_text_response_fixture() {
        let body: Value = serde_json::from_str(TEXT_RESPONSE_FIXTURE).unwrap();
        let resp = parse_response_body(&body).unwrap();

        assert_eq!(resp.content.as_deref(), Some("总共摄入 500 千卡"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "stop");
        assert_eq!(
            resp.usage,
            Usage {
                prompt_tokens: 120,
                completion_tokens: 30
            }
        );
    }

    /// Fixture: a response carrying one tool call, arguments as a JSON-encoded string per
    /// the OpenAI Chat Completions format.
    const TOOL_CALL_RESPONSE_FIXTURE: &str = r#"{
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "log_meal",
                                "arguments": "{\"food\":\"egg\",\"grams\":100}"
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }
        ],
        "usage": {"prompt_tokens": 200, "completion_tokens": 15}
    }"#;

    #[test]
    fn parse_response_body_maps_tool_call_response_fixture() {
        let body: Value = serde_json::from_str(TOOL_CALL_RESPONSE_FIXTURE).unwrap();
        let resp = parse_response_body(&body).unwrap();

        assert_eq!(resp.content, None);
        assert_eq!(resp.finish_reason, "tool_calls");
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_abc");
        assert_eq!(resp.tool_calls[0].name, "log_meal");
        assert_eq!(
            resp.tool_calls[0].arguments,
            json!({"food": "egg", "grams": 100})
        );
    }

    #[test]
    fn parse_response_body_returns_invalid_response_when_tool_call_arguments_are_malformed() {
        let body: Value = serde_json::from_str(
            r#"{
                "choices": [{
                    "message": {
                        "tool_calls": [{
                            "id": "call_1",
                            "function": {"name": "f", "arguments": "not json"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }"#,
        )
        .unwrap();

        let err = parse_response_body(&body).unwrap_err();
        assert!(matches!(err, LlmError::InvalidResponse(_)));
    }

    #[test]
    fn parse_response_body_returns_invalid_response_when_choices_is_missing() {
        let body: Value = serde_json::from_str(r#"{"usage": {}}"#).unwrap();
        let err = parse_response_body(&body).unwrap_err();
        assert!(matches!(err, LlmError::InvalidResponse(_)));
    }

    #[test]
    fn parse_response_body_defaults_usage_when_absent() {
        let body: Value = serde_json::from_str(
            r#"{"choices": [{"message": {"content": "hi"}, "finish_reason": "stop"}]}"#,
        )
        .unwrap();
        let resp = parse_response_body(&body).unwrap();
        assert_eq!(resp.usage, Usage::default());
    }

    fn tool_call_request(model: &str, provider_extra: Option<Value>) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            messages: vec![Message::Assistant {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "log_meal".into(),
                    arguments: json!({}),
                    provider_extra,
                }],
            }],
            tools: vec![],
            temperature: Some(0.0),
            max_tokens: None,
            response_format: ResponseFormat::Text,
            effort: None,
        }
    }

    /// Fixture: Gemini's OpenAI-compatible endpoint attaching a thought signature to a call.
    const GEMINI_TOOL_CALL_FIXTURE: &str = r#"{
        "choices": [{
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "",
                    "type": "function",
                    "function": {"name": "log_meal", "arguments": "{}"},
                    "extra_content": {"google": {"thought_signature": "c2ln"}}
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }"#;

    #[test]
    fn gemini_thought_signature_round_trips_and_empty_id_gets_fallback() {
        let body: Value = serde_json::from_str(GEMINI_TOOL_CALL_FIXTURE).unwrap();
        let resp = parse_response_body(&body).unwrap();
        let call = &resp.tool_calls[0];
        assert_eq!(call.id, "call_0");
        let extra = json!({"google": {"thought_signature": "c2ln"}});
        assert_eq!(call.provider_extra, Some(extra.clone()));

        let req = tool_call_request("gemini-2.5-flash", call.provider_extra.clone());
        let sent = build_request_body(&req);
        assert_eq!(sent["messages"][0]["tool_calls"][0]["extra_content"], extra);
    }

    #[test]
    fn tool_calls_without_provider_extra_send_no_extra_content() {
        let body: Value = serde_json::from_str(TOOL_CALL_RESPONSE_FIXTURE).unwrap();
        assert_eq!(
            parse_response_body(&body).unwrap().tool_calls[0].provider_extra,
            None
        );

        let sent = build_request_body(&tool_call_request("gpt-test", None));
        let call = sent["messages"][0]["tool_calls"][0].as_object().unwrap();
        assert!(!call.contains_key("extra_content"));
    }

    #[test]
    fn temperature_is_dropped_for_gemini_3_only() {
        for model in [
            "gemini-3-pro-preview",
            "models/gemini-3-flash",
            " Gemini-3-pro ",
        ] {
            let body = build_request_body(&tool_call_request(model, None));
            assert!(body.get("temperature").is_none(), "{model}");
        }
        for model in ["gemini-2.5-flash", "gpt-test", "deepseek-chat"] {
            let body = build_request_body(&tool_call_request(model, None));
            assert_eq!(body["temperature"], json!(0.0), "{model}");
        }
    }
}
