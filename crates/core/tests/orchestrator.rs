//! Orchestrator integration tests with a tiny test plugin, per `docs/PHASE1_SPEC.md` §13/§14
//! (M3 "done when": these pass).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use lifeagent_core::app::CoreAppBuilder;
use lifeagent_core::clock::FixedClock;
use lifeagent_core::events::EventSource;
use lifeagent_core::plugin::{Plugin, PluginContext, PluginError, PluginManifest, ToolOutput};
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatResponse, ToolCall, Usage};
use serde_json::{json, Value};

/// A minimal plugin exercising exactly one tool, standing in for the diet plugin that lands
/// in M4 (the milestone note explicitly asks for "a tiny test plugin" here).
struct TestPlugin;

#[async_trait]
impl Plugin for TestPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "test",
            name: "Test Plugin",
            version: "0.1.0",
            event_types: vec!["test.thing_logged"],
            migrations: vec![],
        }
    }

    fn tools(&self) -> Vec<lifeagent_llm::types::ToolSpec> {
        vec![lifeagent_llm::types::ToolSpec {
            name: "test_log_thing".into(),
            description: "Logs a thing, for orchestrator tests.".into(),
            parameters: json!({
                "type": "object",
                "required": ["value"],
                "properties": { "value": {"type": "string"} }
            }),
        }]
    }

    async fn context_snippet(
        &self,
        ctx: &PluginContext,
        _date: chrono::NaiveDate,
    ) -> Result<Option<String>, PluginError> {
        let logged = ctx.events.latest("test.thing_logged").await?;
        Ok(Some(match logged {
            Some(event) => format!("logged: {}", event.payload["value"]),
            None => "nothing logged yet".into(),
        }))
    }

    async fn call_tool(
        &self,
        ctx: &PluginContext,
        name: &str,
        args: Value,
    ) -> Result<ToolOutput, PluginError> {
        match name {
            "test_log_thing" => {
                let value = args
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or_else(|| PluginError::InvalidArgs("value must be a string".into()))?;
                let event = ctx
                    .events
                    .append(
                        "test.thing_logged",
                        ctx.clock.now(),
                        json!({"value": value}),
                        EventSource::Chat,
                    )
                    .await?;
                Ok(ToolOutput {
                    content: json!({"entry_id": event.id, "value": value}),
                    summary: format!("已记录：{value}"),
                })
            }
            other => Err(PluginError::NotFound(format!("unknown tool {other}"))),
        }
    }
}

fn text_response(content: &str) -> ChatResponse {
    ChatResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Usage::default(),
        finish_reason: "stop".into(),
    }
}

fn tool_call_response(id: &str, name: &str, arguments: Value) -> ChatResponse {
    ChatResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
            provider_extra: None,
        }],
        usage: Usage::default(),
        finish_reason: "tool_use".into(),
    }
}

async fn builder() -> (CoreAppBuilder, lifeagent_core::db::TestDb) {
    let dir = lifeagent_core::db::TestDb::new().await;
    let builder = CoreAppBuilder::new(dir.config.clone())
        .clock(Arc::new(FixedClock::new(Utc::now())))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(TestPlugin));
    (builder, dir)
}

#[tokio::test]
async fn logs_via_tool_then_answers() {
    let (builder, _dir) = builder().await;
    let mock = Arc::new(MockChatModel::new());
    mock.push_response(tool_call_response(
        "call_1",
        "test_log_thing",
        json!({"value": "燕麦牛奶"}),
    ));
    mock.push_response(text_response("已经帮你记下啦"));
    let app = builder.chat_model_override(mock).build().await.unwrap();

    let turn = app.send_message("测试记录一下燕麦牛奶").await.unwrap();

    assert_eq!(turn.reply, "已经帮你记下啦");
    assert_eq!(turn.activities, vec!["已记录：燕麦牛奶"]);

    // The event was actually stored.
    let event = app
        .events()
        .latest("test.thing_logged")
        .await
        .unwrap()
        .expect("event should be stored");
    assert_eq!(event.payload, json!({"value": "燕麦牛奶"}));

    // The tool message made it into conversation history too.
    let recent = app.conversation().recent(10).await.unwrap();
    let tool_message = recent
        .iter()
        .find(|m| m.tool_name.as_deref() == Some("test_log_thing"))
        .expect("a tool message should have been stored");
    assert!(tool_message
        .content
        .as_deref()
        .unwrap()
        .contains("燕麦牛奶"));
}

#[tokio::test]
async fn system_prompt_is_not_rebuilt_after_a_tool_call_within_a_turn() {
    let (builder, _dir) = builder().await;
    let mock = Arc::new(MockChatModel::new());
    mock.push_response(tool_call_response(
        "call_1",
        "test_log_thing",
        json!({"value": "燕麦牛奶"}),
    ));
    mock.push_response(text_response("已经帮你记下啦"));
    let app = builder
        .chat_model_override(mock.clone())
        .build()
        .await
        .unwrap();

    app.send_message("测试记录一下燕麦牛奶").await.unwrap();

    // Otherwise the second call's status already shows the thing as logged, and the model
    // reads the user's message as a repeat (seen in the M7.6 Windows run).
    let system_of = |req: &lifeagent_llm::types::ChatRequest| match &req.messages[0] {
        lifeagent_llm::types::Message::System { content } => content.clone(),
        other => panic!("expected a system message first, got {other:?}"),
    };
    let received = mock.received_requests();
    assert_eq!(received.len(), 2);
    assert!(system_of(&received[0]).contains("nothing logged yet"));
    assert_eq!(system_of(&received[0]), system_of(&received[1]));
}

#[tokio::test]
async fn invalid_tool_args_return_an_error_the_model_can_retry_from() {
    let (builder, _dir) = builder().await;
    let mock = Arc::new(MockChatModel::new());
    // Missing the required "value" field.
    mock.push_response(tool_call_response("call_1", "test_log_thing", json!({})));
    mock.push_response(tool_call_response(
        "call_2",
        "test_log_thing",
        json!({"value": "修正后的值"}),
    ));
    mock.push_response(text_response("好的，已修正"));
    let app = builder
        .chat_model_override(mock.clone())
        .build()
        .await
        .unwrap();

    let turn = app.send_message("测试无效参数重试").await.unwrap();

    assert_eq!(turn.reply, "好的，已修正");
    assert_eq!(turn.activities, vec!["已记录：修正后的值"]);

    // The retry request (third call overall: the two tool rounds plus the final answer) must
    // have seen the error from the first, invalid call.
    let received = mock.received_requests();
    assert_eq!(received.len(), 3);
    let retry_request = &received[1];
    let saw_error = retry_request.messages.iter().any(|m| match m {
        lifeagent_llm::types::Message::Tool { content, .. } => content.contains("error"),
        _ => false,
    });
    assert!(saw_error, "the retry request should include the tool error");
}

#[tokio::test]
async fn stops_after_the_iteration_cap_with_no_empty_tool_calls() {
    let (builder, _dir) = builder().await;
    let mock = Arc::new(MockChatModel::new());
    for i in 0..6 {
        mock.push_response(tool_call_response(
            &format!("call_{i}"),
            "test_log_thing",
            json!({"value": format!("第{i}次")}),
        ));
    }
    let app = builder
        .chat_model_override(mock.clone())
        .build()
        .await
        .unwrap();

    let turn = app.send_message("一直调用工具").await.unwrap();

    assert_eq!(turn.reply, "（处理步骤过多，已停止。请换个说法再试一次。）");
    assert_eq!(turn.activities.len(), 6);
    // Exactly 6 rounds were sent to the model — the cap, not one more.
    assert_eq!(mock.received_requests().len(), 6);
}

#[tokio::test]
async fn retrying_a_failed_turn_does_not_store_the_user_message_twice() {
    let (builder, _dir) = builder().await;
    let mock = Arc::new(MockChatModel::new());
    mock.push_error(lifeagent_llm::error::LlmError::Auth);
    mock.push_response(text_response("好的"));
    let app = builder
        .chat_model_override(mock.clone())
        .build()
        .await
        .unwrap();

    assert!(app.send_message("重试的消息").await.is_err());
    let turn = app.send_message("重试的消息").await.unwrap();
    assert_eq!(turn.reply, "好的");

    let recent = app.conversation().recent(10).await.unwrap();
    let contents: Vec<_> = recent.iter().map(|m| m.content.as_deref()).collect();
    assert_eq!(contents, vec![Some("重试的消息"), Some("好的")]);

    // A new, different message after a completed turn is stored normally, and so is sending
    // the same text again once its earlier turn has finished.
    mock.push_response(text_response("再次收到"));
    app.send_message("重试的消息").await.unwrap();
    assert_eq!(app.conversation().recent(10).await.unwrap().len(), 4);
}
