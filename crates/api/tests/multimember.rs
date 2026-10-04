//! The multimember plugin with the rest of the app: managed from chat (MM2), and shared meals
//! planned by the diet plugin from its computed section (MM4).

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::FixedClock;
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatResponse, ToolCall, Usage};
use lifeagent_plugin_diet::DietPlugin;
use lifeagent_plugin_multimember::{store, MultimemberPlugin};
use lifeagent_plugin_stock::StockPlugin;
use lifeagent_plugin_training::TrainingPlugin;
use serde_json::json;

fn local(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
    let naive = NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, 0, 0)
        .unwrap();
    Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap()
        .with_timezone(&Utc)
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        content: Some(text.into()),
        tool_calls: vec![],
        usage: Usage::default(),
        finish_reason: "stop".into(),
    }
}

fn tool_response(calls: Vec<(&str, serde_json::Value)>) -> ChatResponse {
    ChatResponse {
        content: None,
        tool_calls: calls
            .into_iter()
            .enumerate()
            .map(|(i, (name, arguments))| ToolCall {
                id: format!("call_{i}"),
                name: name.into(),
                arguments,
                provider_extra: None,
            })
            .collect(),
        usage: Usage::default(),
        finish_reason: "tool_calls".into(),
    }
}

struct Env {
    app: CoreApp,
    mock: Arc<MockChatModel>,
    _db: lifeagent_core::db::TestDb,
}

async fn app_at(now: DateTime<Utc>) -> Env {
    let db = lifeagent_core::db::TestDb::new().await;
    let mock = Arc::new(MockChatModel::new());
    let app = CoreAppBuilder::new(db.config.clone())
        .clock(Arc::new(FixedClock::new(now)))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
        .plugin(Arc::new(StockPlugin))
        .plugin(Arc::new(TrainingPlugin))
        .plugin(Arc::new(MultimemberPlugin))
        .chat_model_override(mock.clone())
        .build()
        .await
        .unwrap();
    Env { app, mock, _db: db }
}

#[tokio::test]
async fn members_are_managed_from_chat_once_the_plugin_is_on() {
    let env = app_at(local(2026, 10, 4, 10)).await;
    let (app, mock) = (&env.app, &env.mock);
    let tomorrow = app.logical_today().await.unwrap().succ_opt().unwrap();

    // Off by default: its tools aren't offered.
    mock.push_response(text_response("ok"));
    app.send_message("hi").await.unwrap();
    let offered = |i: usize| -> Vec<String> {
        mock.received_requests()[i]
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect()
    };
    assert!(!offered(0).iter().any(|n| n.starts_with("multimember_")));

    app.set_plugin_enabled("multimember", true).await.unwrap();
    mock.push_response(tool_response(vec![(
        "multimember_add_member",
        json!({
            "display_name": "Aki", "sex": "female", "birth_date": "1992-05-10",
            "height_cm": 160, "weight_kg": 52, "activity_level": "light",
            "dietary_pattern": "omnivore", "allergies": ["えび"]
        }),
    )]));
    mock.push_response(tool_response(vec![
        (
            "multimember_set_attendance",
            json!({"member": "Aki", "meal_type": "dinner", "present": false,
                   "date": tomorrow.to_string(), "note": "飲み会"}),
        ),
        (
            "multimember_add_dislike",
            json!({"member": "Aki", "text": "セロリ"}),
        ),
    ]));
    mock.push_response(text_response("登録しました"));
    let turn = app
        .send_message("妻のAkiも一緒に食べます。明日の夜は飲み会。セロリが苦手。")
        .await
        .unwrap();
    assert_eq!(turn.reply, "登録しました");
    assert!(offered(1).iter().any(|n| n == "multimember_add_member"));

    let ctx = app.plugin_context();
    let members = store::list_members(&ctx).await.unwrap();
    assert_eq!(members.len(), 1);
    let aki = &members[0];
    assert!(store::attendance_on(&ctx, tomorrow)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store::dislikes_on(&ctx, &aki.id, tomorrow).await.unwrap(),
        vec!["セロリ"]
    );

    // The next turn's system prompt carries the household, without body measurements.
    mock.push_response(text_response("ok"));
    app.send_message("明日の予定は？").await.unwrap();
    let system = format!("{:?}", mock.received_requests().last().unwrap().messages[0]);
    assert!(system.contains("Aki ("), "{system}");
    assert!(system.contains("Tomorrow"), "{system}");
}
