//! `--mock`'s scripted response: a `MockChatModel` (§6.5) that echoes the user's last message
//! back instead of calling a real provider, per `docs/PHASE1_SPEC.md` §11 ("for smoke-testing
//! plumbing only").
//!
//! One closure is pushed onto the queue right before each `chat`/`say` turn (`run_chat_turn`
//! below): the model always answers without tool calls, so the orchestrator's loop
//! (`CoreApp::send_message`) makes exactly one call per turn, matching exactly one queued
//! response. Plan generation is not wired to echo anything — its planner role expects
//! structured JSON a plain echo could never satisfy, and `--mock` promises smoke-testing the
//! chat plumbing only, not the planner.

use lifeagent_llm::error::LlmError;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatRequest, ChatResponse, Message, Usage};

fn echo_response(req: &ChatRequest) -> Result<ChatResponse, LlmError> {
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

/// Queues one echo response. Call once immediately before each `CoreApp::send_message`.
pub fn queue_echo(mock: &MockChatModel) {
    mock.push_closure(echo_response);
}
