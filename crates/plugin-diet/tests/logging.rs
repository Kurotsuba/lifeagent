//! M4 "done when" (`docs/PHASE1_SPEC.md` §14): an integration test logs a meal via the
//! orchestrator and `today_view` shows correct totals.

use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::FixedClock;
use lifeagent_core::events::EventSource;
use lifeagent_core::goals::{GoalDirection, WeightGoal};
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatRequest, ChatResponse, Message, ToolCall, Usage};
use lifeagent_plugin_diet::DietPlugin;
use serde_json::{json, Value};

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

/// Answers a food-estimate request by reading the food name out of the prompt, so the two
/// concurrent estimates can arrive in either order.
fn estimate(req: &ChatRequest) -> Result<ChatResponse, LlmError> {
    let prompt = match req.messages.first() {
        Some(Message::User { content }) => content.clone(),
        _ => String::new(),
    };
    if prompt.ends_with("Food: 熟米饭") {
        Ok(json_response(json!({
            "kcal": 116, "protein_g": 2.6, "carbs_g": 25.9, "fat_g": 0.3, "fiber_g": 0.3
        })))
    } else if prompt.ends_with("Food: 鸡蛋") {
        Ok(json_response(json!({
            "kcal": 144, "protein_g": 13.3, "carbs_g": 2.8, "fat_g": 8.8, "fiber_g": 0
        })))
    } else {
        Err(LlmError::Config(format!("unexpected prompt: {prompt}")))
    }
}

async fn app(mock: Arc<MockChatModel>) -> (CoreApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let app = CoreAppBuilder::new(dir.path().to_path_buf())
        .clock(Arc::new(FixedClock::new(Utc::now())))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
        .chat_model_override(mock)
        .build()
        .await
        .unwrap();

    app.profile()
        .set(&Profile {
            display_name: None,
            sex: Sex::Male,
            birth_date: NaiveDate::from_ymd_opt(1996, 1, 1).unwrap(),
            height_cm: 178.0,
            activity_level: ActivityLevel::Moderate,
            dietary_pattern: DietaryPattern::Omnivore,
            allergies: vec![],
            dislikes: vec![],
            medical_notes: None,
            meals_per_day: 3,
        })
        .await
        .unwrap();
    app.events()
        .append(
            "body.weight_logged",
            Utc::now(),
            json!({"kg": 78.0}),
            EventSource::Ui,
        )
        .await
        .unwrap();
    app.goals()
        .save_weight_goal(&WeightGoal {
            direction: GoalDirection::Lose,
            target_weight_kg: Some(72.0),
            rate_kg_per_week: 0.5,
        })
        .await
        .unwrap();
    (app, dir)
}

#[tokio::test]
async fn meal_logged_via_orchestrator_shows_up_in_today_view() {
    let mock = Arc::new(MockChatModel::new());
    // 1. Conversation model calls diet_log_meal with the user's words.
    mock.push_response(response(
        None,
        vec![ToolCall {
            id: "call_1".into(),
            name: "diet_log_meal".into(),
            arguments: json!({"description": "午饭吃了一碗米饭和两个鸡蛋"}),
            provider_extra: None,
        }],
    ));
    // 2. Meal parser.
    mock.push_response(json_response(json!({
        "meal_type": "lunch",
        "dishes": [
            {"name": "熟米饭", "components": [{"name": "熟米饭", "grams": 200}]},
            {"name": "鸡蛋", "components": [{"name": "鸡蛋", "grams": 100}]}
        ]
    })));
    // 3–4. One estimate per new food, concurrently.
    mock.push_closure(estimate);
    mock.push_closure(estimate);
    // 5. Conversation model answers, quoting the tool result.
    mock.push_response(response(Some("已记录午餐，共 376 kcal。".into()), vec![]));
    let (app, _dir) = app(mock.clone()).await;

    let turn = app
        .send_message("午饭吃了一碗米饭和两个鸡蛋")
        .await
        .unwrap();

    assert_eq!(turn.reply, "已记录午餐，共 376 kcal。");
    assert_eq!(turn.activities, vec!["已记录午餐 · 376 kcal"]);

    // The final conversation request saw the tool result with the totals.
    let requests = mock.received_requests();
    let tool_result = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .find_map(|m| match m {
            Message::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("tool result in the final request");
    let tool_result: Value = serde_json::from_str(&tool_result).unwrap();
    assert_eq!(tool_result["total"]["kcal"], 376.0);

    // today_view shows the same totals: 200 g rice (232 kcal, 5.2 g protein) + 100 g egg
    // (144 kcal, 13.3 g protein).
    let view = app.today_view().await.unwrap();
    let card = &view.cards["diet"];
    assert_eq!(card["total"]["kcal"], 376.0);
    assert_eq!(card["total"]["protein_g"], 18.5);
    assert_eq!(card["target"]["kcal"].as_f64().map(|k| k > 0.0), Some(true));
    let meals = card["meals"].as_array().unwrap();
    assert_eq!(meals.len(), 1);
    assert_eq!(meals[0]["meal_type"], "lunch");
    assert_eq!(meals[0]["items"][0]["name"], "熟米饭");
    assert_eq!(meals[0]["items"][0]["estimated"], true);
    assert_eq!(view.latest_weight.map(|w| w.kg), Some(78.0));
}

#[tokio::test]
async fn second_meal_with_known_foods_needs_no_estimates() {
    let mock = Arc::new(MockChatModel::new());
    let log_call = || {
        response(
            None,
            vec![ToolCall {
                id: "call".into(),
                name: "diet_log_meal".into(),
                arguments: json!({"description": "一碗米饭", "meal_type": "dinner"}),
                provider_extra: None,
            }],
        )
    };
    let parsed = || {
        json_response(json!({
            "meal_type": null,
            "dishes": [{"name": "熟米饭", "components": [{"name": "熟米饭", "grams": 200}]}]
        }))
    };
    // Turn 1: 熟米饭 is new, so it's estimated.
    mock.push_response(log_call());
    mock.push_response(parsed());
    mock.push_closure(estimate);
    mock.push_response(response(Some("好的".into()), vec![]));
    // Turn 2: 熟米饭 is in the food table now — no estimate.
    mock.push_response(log_call());
    mock.push_response(parsed());
    mock.push_response(response(Some("好的".into()), vec![]));
    let (app, _dir) = app(mock.clone()).await;

    app.send_message("晚饭一碗米饭").await.unwrap();
    app.send_message("又吃了一碗米饭").await.unwrap();

    // 2 turns × (2 conversation + 1 parser) + 1 estimate.
    assert_eq!(mock.received_requests().len(), 7);
    let card = &app.today_view().await.unwrap().cards["diet"];
    assert_eq!(card["total"]["kcal"], 464.0);
    assert_eq!(card["meals"].as_array().unwrap().len(), 2);
}
