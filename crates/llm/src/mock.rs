//! `MockChatModel`: scripted responses for offline, deterministic tests.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.5. Used by every core and plugin test
//! per `CLAUDE.md`: never call a real LLM in tests.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::types::{ChatModel, ChatRequest, ChatResponse};

type ClosureResponder = Box<dyn Fn(&ChatRequest) -> Result<ChatResponse, LlmError> + Send + Sync>;

enum Scripted {
    Fixed(Result<ChatResponse, LlmError>),
    Closure(ClosureResponder),
}

/// Holds a queue of scripted responses (or closures receiving the request) and records every
/// request it receives, for assertions.
#[derive(Default)]
pub struct MockChatModel {
    queue: Mutex<VecDeque<Scripted>>,
    received: Mutex<Vec<ChatRequest>>,
}

impl MockChatModel {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            received: Mutex::new(Vec::new()),
        }
    }

    /// Queues a response to return on the next `complete` call.
    pub fn push_response(&self, response: ChatResponse) {
        self.queue
            .lock()
            .expect("MockChatModel mutex poisoned")
            .push_back(Scripted::Fixed(Ok(response)));
    }

    /// Queues an error to return on the next `complete` call.
    pub fn push_error(&self, error: LlmError) {
        self.queue
            .lock()
            .expect("MockChatModel mutex poisoned")
            .push_back(Scripted::Fixed(Err(error)));
    }

    /// Queues a closure that computes the response from the request it receives.
    pub fn push_closure(
        &self,
        f: impl Fn(&ChatRequest) -> Result<ChatResponse, LlmError> + Send + Sync + 'static,
    ) {
        self.queue
            .lock()
            .expect("MockChatModel mutex poisoned")
            .push_back(Scripted::Closure(Box::new(f)));
    }

    /// Every request received so far, in call order.
    pub fn received_requests(&self) -> Vec<ChatRequest> {
        self.received
            .lock()
            .expect("MockChatModel mutex poisoned")
            .clone()
    }
}

#[async_trait]
impl ChatModel for MockChatModel {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        self.received
            .lock()
            .expect("MockChatModel mutex poisoned")
            .push(req.clone());

        let scripted = self
            .queue
            .lock()
            .expect("MockChatModel mutex poisoned")
            .pop_front();

        match scripted {
            Some(Scripted::Fixed(result)) => result,
            Some(Scripted::Closure(f)) => f(&req),
            None => Err(LlmError::Config(
                "MockChatModel: no scripted response queued".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Usage;

    fn response(content: &str) -> ChatResponse {
        ChatResponse {
            content: Some(content.into()),
            tool_calls: vec![],
            usage: Usage::default(),
            finish_reason: "stop".into(),
        }
    }

    fn request(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            messages: vec![],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            response_format: Default::default(),
            effort: None,
        }
    }

    #[tokio::test]
    async fn returns_queued_responses_in_order() {
        let mock = MockChatModel::new();
        mock.push_response(response("first"));
        mock.push_response(response("second"));

        let first = mock.complete(request("m")).await.unwrap();
        let second = mock.complete(request("m")).await.unwrap();

        assert_eq!(first.content.as_deref(), Some("first"));
        assert_eq!(second.content.as_deref(), Some("second"));
    }

    #[tokio::test]
    async fn returns_queued_errors() {
        let mock = MockChatModel::new();
        mock.push_error(LlmError::RateLimited);

        let err = mock.complete(request("m")).await.unwrap_err();
        assert!(matches!(err, LlmError::RateLimited));
    }

    #[tokio::test]
    async fn closure_sees_the_request() {
        let mock = MockChatModel::new();
        mock.push_closure(|req| Ok(response(&req.model)));

        let resp = mock.complete(request("echo-me")).await.unwrap();
        assert_eq!(resp.content.as_deref(), Some("echo-me"));
    }

    #[tokio::test]
    async fn records_received_requests() {
        let mock = MockChatModel::new();
        mock.push_response(response("ok"));
        mock.push_response(response("ok"));

        mock.complete(request("m1")).await.unwrap();
        mock.complete(request("m2")).await.unwrap();

        let received = mock.received_requests();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].model, "m1");
        assert_eq!(received[1].model, "m2");
    }

    #[tokio::test]
    async fn empty_queue_returns_config_error() {
        let mock = MockChatModel::new();
        let err = mock.complete(request("m")).await.unwrap_err();
        assert!(matches!(err, LlmError::Config(_)));
    }
}
