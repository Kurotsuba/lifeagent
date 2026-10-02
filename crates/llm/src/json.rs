//! `complete_json`: JSON-mode helper with fence-stripping and a one-shot repair retry.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.4. Load-bearing for the Anthropic
//! provider, which has no native JSON-mode equivalent to fall back on.

use serde::de::DeserializeOwned;

use crate::error::LlmError;
use crate::router::{LlmRouter, RoleRequest};
use crate::types::{ChatResponse, Message, ResponseFormat, Role};

/// Strips a leading/trailing ```` ```json ```` or ```` ``` ```` fence, if present.
/// Extracts the body of a ```` ``` ```` fence, with any info string (`json`, `JSON`, …) and any
/// prose around the fence. Text that already starts as JSON is returned as-is, so a string value
/// containing backticks is never mistaken for a fence.
fn strip_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.starts_with(['{', '[']) {
        return trimmed;
    }
    let Some(open) = trimmed.find("```") else {
        return trimmed;
    };
    let body = trimmed[open + 3..].trim_start_matches(|c: char| c.is_ascii_alphanumeric());
    match body.rfind("```") {
        Some(close) => body[..close].trim(),
        None => body.trim(),
    }
}

fn parse_content<T: DeserializeOwned>(resp: &ChatResponse) -> Result<T, String> {
    let content = resp.content.as_deref().unwrap_or("");
    serde_json::from_str(strip_fences(content)).map_err(|e| e.to_string())
}

/// Calls the given `role` with `ResponseFormat::JsonObject`, strips accidental ```` ```json ````
/// fences, and deserializes as `T`. On a first failure, makes one repair attempt: appends the
/// model's output as an assistant message and a user message asking for corrected JSON. A
/// second failure returns `LlmError::InvalidResponse`.
pub async fn complete_json<T: DeserializeOwned>(
    router: &LlmRouter,
    role: Role,
    messages: Vec<Message>,
) -> Result<T, LlmError> {
    let resp = router
        .complete(
            role,
            RoleRequest {
                messages: messages.clone(),
                tools: vec![],
                max_tokens: None,
                response_format: ResponseFormat::JsonObject,
            },
        )
        .await?;

    match parse_content::<T>(&resp) {
        Ok(value) => Ok(value),
        Err(parse_err) => {
            let mut repair_messages = messages;
            repair_messages.push(Message::Assistant {
                content: resp.content.clone(),
                tool_calls: vec![],
            });
            repair_messages.push(Message::User {
                content: format!(
                    "Your previous output was not valid for the required schema: {parse_err}. \
                     Return only corrected JSON."
                ),
            });

            let repair_resp = router
                .complete(
                    role,
                    RoleRequest {
                        messages: repair_messages,
                        tools: vec![],
                        max_tokens: None,
                        response_format: ResponseFormat::JsonObject,
                    },
                )
                .await?;

            parse_content::<T>(&repair_resp).map_err(|e| {
                LlmError::InvalidResponse(format!(
                    "repair attempt still failed to deserialize: {e}"
                ))
            })
        }
    }
}

/// Like `complete_json`, but with no repair attempt: output that doesn't deserialize as `T` is
/// `LlmError::InvalidResponse` straight away. For callers whose own fallback is cheaper than a
/// second call.
pub async fn complete_json_once<T: DeserializeOwned>(
    router: &LlmRouter,
    role: Role,
    messages: Vec<Message>,
) -> Result<T, LlmError> {
    let resp = router
        .complete(
            role,
            RoleRequest {
                messages,
                tools: vec![],
                max_tokens: None,
                response_format: ResponseFormat::JsonObject,
            },
        )
        .await?;
    parse_content::<T>(&resp).map_err(LlmError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockChatModel;
    use crate::router::{
        ModelsConfig, ProviderConfig, ProviderKind, RoleBinding, UsageEvent, UsageSink,
    };
    use crate::types::{ChatModel, ChatResponse, Usage};
    use async_trait::async_trait;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Sample {
        a: u32,
        b: String,
    }

    struct NoopUsageSink;

    #[async_trait]
    impl UsageSink for NoopUsageSink {
        async fn record(&self, _event: UsageEvent) {}
    }

    fn ok_response(content: &str) -> ChatResponse {
        ChatResponse {
            content: Some(content.into()),
            tool_calls: vec![],
            usage: Usage::default(),
            finish_reason: "stop".into(),
        }
    }

    fn test_router(mock: Arc<MockChatModel>) -> LlmRouter {
        let binding = RoleBinding {
            provider_id: "p1".into(),
            model: "model-x".into(),
            temperature: None,
            effort: None,
        };
        let config = ModelsConfig {
            providers: vec![ProviderConfig {
                id: "p1".into(),
                kind: ProviderKind::OpenAiCompatible,
                base_url: "http://example.invalid".into(),
                auth: Default::default(),
            }],
            parser: binding.clone(),
            planner: binding.clone(),
            conversation: binding,
        };
        let mut providers: HashMap<String, Arc<dyn ChatModel>> = HashMap::new();
        providers.insert("p1".into(), mock);
        LlmRouter::with_providers(config, providers, Arc::new(NoopUsageSink))
    }

    #[tokio::test]
    async fn deserializes_plain_json() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response(r#"{"a": 1, "b": "x"}"#));
        let router = test_router(mock);

        let value: Sample = complete_json(&router, Role::Parser, vec![]).await.unwrap();
        assert_eq!(
            value,
            Sample {
                a: 1,
                b: "x".into()
            }
        );
    }

    #[tokio::test]
    async fn strips_json_fence() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response("```json\n{\"a\": 2, \"b\": \"y\"}\n```"));
        let router = test_router(mock);

        let value: Sample = complete_json(&router, Role::Parser, vec![]).await.unwrap();
        assert_eq!(
            value,
            Sample {
                a: 2,
                b: "y".into()
            }
        );
    }

    #[tokio::test]
    async fn strips_bare_fence() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response("```\n{\"a\": 3, \"b\": \"z\"}\n```"));
        let router = test_router(mock);

        let value: Sample = complete_json(&router, Role::Parser, vec![]).await.unwrap();
        assert_eq!(
            value,
            Sample {
                a: 3,
                b: "z".into()
            }
        );
    }

    #[test]
    fn strip_fences_handles_uppercase_info_string_and_leading_prose() {
        assert_eq!(strip_fences("```JSON\n{\"a\": 1}\n```"), r#"{"a": 1}"#);
        assert_eq!(
            strip_fences("Here you go:\n```json\n{\"a\": 1}\n```\nDone."),
            r#"{"a": 1}"#
        );
        assert_eq!(strip_fences("```json {\"a\": 1}```"), r#"{"a": 1}"#);
        assert_eq!(strip_fences(r#"{"a": "```"}"#), r#"{"a": "```"}"#);
    }

    #[tokio::test]
    async fn once_variant_makes_no_repair_attempt() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response("not json at all"));
        let router = test_router(mock.clone());

        let err = complete_json_once::<Sample>(&router, Role::Parser, vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::InvalidResponse(_)));
        assert_eq!(mock.received_requests().len(), 1);
    }

    #[tokio::test]
    async fn repairs_after_one_bad_response() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response("not json at all"));
        mock.push_response(ok_response(r#"{"a": 4, "b": "fixed"}"#));
        let router = test_router(mock.clone());

        let value: Sample = complete_json(&router, Role::Parser, vec![]).await.unwrap();
        assert_eq!(
            value,
            Sample {
                a: 4,
                b: "fixed".into()
            }
        );

        let received = mock.received_requests();
        assert_eq!(received.len(), 2);
        // The repair call carries the bad output plus a correction request.
        assert_eq!(received[1].messages.len(), 2);
    }

    #[tokio::test]
    async fn gives_up_as_invalid_response_after_second_failure() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_response(ok_response("still not json"));
        mock.push_response(ok_response("still not json either"));
        let router = test_router(mock);

        let err = complete_json::<Sample>(&router, Role::Parser, vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn propagates_provider_error_without_repair_attempt() {
        let mock = Arc::new(MockChatModel::new());
        mock.push_error(LlmError::Auth);
        let router = test_router(mock.clone());

        let err = complete_json::<Sample>(&router, Role::Parser, vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Auth));
        assert_eq!(mock.received_requests().len(), 1);
    }
}
