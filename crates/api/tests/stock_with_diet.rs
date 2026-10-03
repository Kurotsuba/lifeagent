//! The stock plugin reacting to the diet plugin's real `diet.meal_logged` payload, through a
//! full `CoreApp` with both plugins registered and the live event dispatcher: a meal logged in
//! chat takes its ingredients out of stock, and deleting the meal puts them back.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use lifeagent_api::ops::{self, NewStockDto};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::FixedClock;
use lifeagent_core::events::AppEvent;
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatRequest, ChatResponse, ToolCall, Usage};
use lifeagent_plugin_diet::DietPlugin;
use lifeagent_plugin_stock::model::Unit;
use lifeagent_plugin_stock::StockPlugin;
use serde_json::{json, Value};
use tokio::sync::broadcast;

fn response(content: Option<String>, tool_calls: Vec<ToolCall>) -> ChatResponse {
    ChatResponse {
        content,
        tool_calls,
        usage: Usage::default(),
        finish_reason: "stop".into(),
    }
}

fn json_response(value: Value) -> ChatResponse {
    response(Some(value.to_string()), vec![])
}

/// Answers every model request by what it asks for, since the stock matcher runs on the event
/// dispatcher concurrently with the conversation's final turn.
fn route(req: &ChatRequest) -> Result<ChatResponse, LlmError> {
    let text = format!("{:?}", req.messages);
    if text.contains("Home stock") {
        // Stock order: 米 (2000 g), 鸡蛋 (6 piece) — both undated, by name.
        assert!(text.contains("熟米饭 200 g"), "{text}");
        return Ok(json_response(
            json!({"uses": [{"stock": 0, "amount": 90}, {"stock": 1, "amount": 2}]}),
        ));
    }
    if text.contains("Food: 熟米饭") {
        return Ok(json_response(
            json!({"kcal": 116, "protein_g": 2.6, "carbs_g": 25.9, "fat_g": 0.3, "fiber_g": 0.3}),
        ));
    }
    if text.contains("Food: 鸡蛋") {
        return Ok(json_response(
            json!({"kcal": 144, "protein_g": 13.3, "carbs_g": 2.8, "fat_g": 8.8, "fiber_g": 0}),
        ));
    }
    if !req.tools.is_empty() {
        return Ok(if text.contains("Tool {") {
            response(Some("已记录。".into()), vec![])
        } else {
            response(
                None,
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "diet_log_meal".into(),
                    arguments: json!({"description": "午饭在家做了米饭和两个煎蛋"}),
                    provider_extra: None,
                }],
            )
        });
    }
    // The meal parser.
    Ok(json_response(json!({
        "meal_type": "lunch",
        "dishes": [
            {"name": "熟米饭", "components": [{"name": "熟米饭", "grams": 200}]},
            {"name": "煎蛋", "components": [{"name": "鸡蛋", "grams": 100}]}
        ]
    })))
}

async fn app(mock: Arc<MockChatModel>) -> (CoreApp, lifeagent_core::db::TestDb) {
    let dir = lifeagent_core::db::TestDb::new().await;
    let app = CoreAppBuilder::new(dir.config.clone())
        .clock(Arc::new(FixedClock::new(Utc::now())))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
        .plugin(Arc::new(StockPlugin))
        .chat_model_override(mock)
        .build()
        .await
        .unwrap();
    (app, dir)
}

async fn next_meal_change(rx: &mut broadcast::Receiver<AppEvent>) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for DataChanged")
            .unwrap();
        if matches!(event, AppEvent::DataChanged { ref event_type, .. } if event_type == "diet.meal_logged")
        {
            return;
        }
    }
}

async fn left(app: &CoreApp) -> Vec<(String, f64)> {
    ops::get_stock(app)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|i| (i.name, i.quantity))
        .collect()
}

#[tokio::test]
async fn a_meal_logged_in_chat_uses_stock_and_deleting_it_puts_it_back() {
    let mock = Arc::new(MockChatModel::new());
    for _ in 0..6 {
        mock.push_closure(route);
    }
    let (app, _dir) = app(mock.clone()).await;
    for (name, quantity, unit) in [("米", 2000.0, Unit::G), ("鸡蛋", 6.0, Unit::Piece)] {
        ops::add_stock_item(
            &app,
            &NewStockDto {
                name: name.into(),
                quantity,
                unit,
                expires_on: None,
            },
        )
        .await
        .unwrap();
    }
    let mut rx = app.subscribe();

    app.send_message("午饭在家做了米饭和两个煎蛋")
        .await
        .unwrap();
    // DataChanged follows the plugins' event hooks.
    next_meal_change(&mut rx).await;

    assert_eq!(
        left(&app).await,
        vec![("米".to_string(), 1910.0), ("鸡蛋".to_string(), 4.0)]
    );
    let view = ops::get_stock(&app).await.unwrap();
    assert_eq!(view.recent_uses.len(), 2);

    let entry_id = view.recent_uses[0].event_id.clone();
    ops::delete_entry(&app, &entry_id).await.unwrap();
    next_meal_change(&mut rx).await;

    assert_eq!(
        left(&app).await,
        vec![("米".to_string(), 2000.0), ("鸡蛋".to_string(), 6.0)]
    );
    assert!(ops::get_stock(&app).await.unwrap().recent_uses.is_empty());
}
