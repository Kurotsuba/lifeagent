//! `revise_plan`: the Nutrition tab's Adjust with wishes typed revises the current plan in place.

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use lifeagent_api::error::ErrorDetail;
use lifeagent_api::ops;
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::FixedClock;
use lifeagent_core::events::EventSource;
use lifeagent_core::plan::{PlanTrigger, StoredPlan};
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_core::time::age_years;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatRequest, ChatResponse, Message, Usage};
use lifeagent_plugin_diet::targets;
use lifeagent_plugin_diet::DietPlugin;
use serde_json::{json, Value};

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

fn json_response(value: Value) -> ChatResponse {
    ChatResponse {
        content: Some(value.to_string()),
        tool_calls: vec![],
        usage: Usage::default(),
        finish_reason: "stop".into(),
    }
}

const WEIGHT_KG: f64 = 78.0;

fn profile() -> Profile {
    Profile {
        display_name: None,
        sex: Sex::Male,
        birth_date: NaiveDate::from_ymd_opt(1996, 1, 1).unwrap(),
        height_cm: 178.0,
        activity_level: ActivityLevel::Moderate,
        dietary_pattern: DietaryPattern::Omnivore,
        allergies: vec![],
        medical_notes: None,
        meals_per_day: 3,
    }
}

/// The diet plugin with a profile and a weight, so there is a plan to make.
async fn app(mock: Arc<MockChatModel>) -> (CoreApp, lifeagent_core::db::TestDb) {
    let db = lifeagent_core::db::TestDb::new().await;
    let now = local(2026, 9, 24, 10);
    let app = CoreAppBuilder::new(db.config.clone())
        .clock(Arc::new(FixedClock::new(now)))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
        .chat_model_override(mock)
        .build()
        .await
        .unwrap();
    app.profile().set(&profile()).await.unwrap();
    app.events()
        .append(
            "body.weight_logged",
            now,
            json!({"kg": WEIGHT_KG}),
            EventSource::Ui,
        )
        .await
        .unwrap();
    (app, db)
}

fn planner_reply(summary: &str) -> ChatResponse {
    let meal = |meal_type: &str| {
        json!({"meal_type": meal_type, "title": "均衡餐",
               "items": [{"name": "均衡餐", "grams": 300.0}]})
    };
    json_response(json!({
        "summary": summary,
        "sections": {"diet": {"meals": [meal("breakfast"), meal("lunch"), meal("dinner")]}}
    }))
}

/// 均衡餐 per 100 g is a ninth of the day's targets, so the plan's 900 g hits them all.
fn estimate_for(date: NaiveDate) -> impl Fn(&ChatRequest) -> Result<ChatResponse, LlmError> {
    let p = profile();
    let t = targets::compute(&p, WEIGHT_KG, age_years(p.birth_date, date), None);
    move |req| match req.messages.first() {
        Some(Message::User { content }) if content.ends_with("Food: 均衡餐") => {
            Ok(json_response(json!({
                "kcal": t.kcal / 9.0, "protein_g": t.protein_g / 9.0,
                "carbs_g": t.carbs_g / 9.0, "fat_g": t.fat_g / 9.0,
                "fiber_g": t.fiber_g / 9.0
            })))
        }
        _ => Err(LlmError::Config("unexpected request".into())),
    }
}

async fn tomorrow(app: &CoreApp) -> NaiveDate {
    app.logical_today().await.unwrap().succ_opt().unwrap()
}

fn detail(result: Result<StoredPlan, lifeagent_api::error::CommandError>) -> ErrorDetail {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.detail,
    }
}

#[tokio::test]
async fn adjust_revises_the_current_plan_with_the_typed_instruction() {
    let mock = Arc::new(MockChatModel::new());
    let (app, _db) = app(mock.clone()).await;
    let day = tomorrow(&app).await;
    let date = day.format("%Y-%m-%d").to_string();
    mock.push_response(planner_reply("v1"));
    mock.push_closure(estimate_for(day));
    ops::generate_plan(&app, Some(date.clone()), None)
        .await
        .unwrap();
    // The food's estimate is cached by now, so the revision only asks the planner.
    mock.push_response(planner_reply("v2"));

    let revised = ops::revise_plan(&app, &date, "  明天中午公司聚餐 ")
        .await
        .unwrap();

    assert_eq!(revised.version, 2);
    assert_eq!(revised.trigger, PlanTrigger::Revision);
    assert_eq!(revised.instruction.as_deref(), Some("明天中午公司聚餐"));
    assert_eq!(revised.content.summary, "v2");
}

#[tokio::test]
async fn a_blank_instruction_is_instruction_empty() {
    let (app, _db) = app(Arc::new(MockChatModel::new())).await;
    let date = tomorrow(&app).await.format("%Y-%m-%d").to_string();

    assert_eq!(
        detail(ops::revise_plan(&app, &date, "  ").await),
        ErrorDetail::InstructionEmpty
    );
}

#[tokio::test]
async fn adjusting_without_a_plan_is_not_found() {
    let (app, _db) = app(Arc::new(MockChatModel::new())).await;
    let date = tomorrow(&app).await.format("%Y-%m-%d").to_string();

    assert_eq!(
        detail(ops::revise_plan(&app, &date, "少一点").await),
        ErrorDetail::NotFound
    );
}
