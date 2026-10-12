//! Diet planned on top of training: with the training plugin enabled, tomorrow's plan is made
//! in two planner calls, and the diet section's targets and brief follow the planned session.

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use lifeagent_core::app::CoreAppBuilder;
use lifeagent_core::clock::FixedClock;
use lifeagent_core::events::EventSource;
use lifeagent_core::goals::{GoalDirection, WeightGoal};
use lifeagent_core::plan::PlanTrigger;
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_core::time::age_years;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatRequest, ChatResponse, Message, Usage};
use lifeagent_plugin_diet::targets::{self, NutritionTargets};
use lifeagent_plugin_diet::DietPlugin;
use lifeagent_plugin_training::{prefs, TrainingPlugin};
use serde_json::{json, Value};

const WEIGHT_KG: f64 = 78.0;
/// 40 min of moderate cardio at 78 kg: (7 − 1) × 78 × 40/60.
const SESSION_KCAL: f64 = 312.0;

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

fn goal() -> WeightGoal {
    WeightGoal {
        direction: GoalDirection::Lose,
        target_weight_kg: Some(72.0),
        rate_kg_per_week: 0.5,
    }
}

fn targets_on(date: NaiveDate) -> NutritionTargets {
    let p = profile();
    let base = targets::compute(&p, WEIGHT_KG, age_years(p.birth_date, date), Some(&goal()));
    targets::with_exercise(base, SESSION_KCAL)
}

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

/// 均衡餐 per 100 g is a tenth of the training-day targets, so 1,000 g hits them all.
fn estimate_for(date: NaiveDate) -> impl Fn(&ChatRequest) -> Result<ChatResponse, LlmError> {
    let t = targets_on(date);
    move |req| match req.messages.first() {
        Some(Message::User { content }) if content.ends_with("Food: 均衡餐") => {
            Ok(json_response(json!({
                "kcal": t.kcal / 10.0, "protein_g": t.protein_g / 10.0,
                "carbs_g": t.carbs_g / 10.0, "fat_g": t.fat_g / 10.0,
                "fiber_g": t.fiber_g / 10.0
            })))
        }
        _ => Err(LlmError::Config("unexpected request".into())),
    }
}

fn planner_prompts(mock: &MockChatModel) -> Vec<String> {
    mock.received_requests()
        .iter()
        .filter_map(|r| match r.messages.first() {
            Some(Message::User { content }) if content.starts_with("You create") => {
                Some(content.clone())
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn invalid_training_never_reaches_the_diet_stage() {
    let db = lifeagent_core::db::TestDb::new().await;
    let now = local(2026, 9, 24, 10);
    let mock = Arc::new(MockChatModel::new());
    let app = CoreAppBuilder::new(db.config.clone())
        .clock(Arc::new(FixedClock::new(now)))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
        .plugin(Arc::new(TrainingPlugin))
        .chat_model_override(mock.clone())
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
    app.set_plugin_enabled("training", true).await.unwrap();
    let tomorrow = app.logical_today().await.unwrap().succ_opt().unwrap();
    for _ in 0..2 {
        mock.push_response(json_response(json!({
            "summary": "invalid training",
            "sections": {"training": {
                "rest_day": false,
                "exercises": [
                    {"name": "run", "kind": "cardio", "duration_min": -20}
                ]
            }}
        })));
    }

    let result = app.generate_plan(tomorrow, PlanTrigger::Manual, None).await;

    assert!(result.is_err());
    let prompts = planner_prompts(&mock);
    assert_eq!(prompts.len(), 2, "only training and its retry may run");
    assert!(prompts
        .iter()
        .all(|prompt| !prompt.contains("## Section \"diet\"")));
    assert!(app.get_plan(tomorrow).await.unwrap().is_none());
}

#[tokio::test]
async fn diet_is_planned_after_training_with_raised_targets() {
    let db = lifeagent_core::db::TestDb::new().await;
    let now = local(2026, 9, 24, 10);
    let mock = Arc::new(MockChatModel::new());
    let app = CoreAppBuilder::new(db.config.clone())
        .clock(Arc::new(FixedClock::new(now)))
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
        .plugin(Arc::new(TrainingPlugin))
        .chat_model_override(mock.clone())
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
    app.goals().save_weight_goal(&goal()).await.unwrap();
    app.set_plugin_enabled("training", true).await.unwrap();
    let tomorrow = app.logical_today().await.unwrap().succ_opt().unwrap();
    prefs::set_day_preference(&app.plugin_context(), tomorrow, "晚上慢跑")
        .await
        .unwrap();

    mock.push_response(json_response(json!({
        "summary": "training only",
        "sections": {"training": {
            "rest_day": false, "title": "慢跑", "time_of_day": "evening",
            "exercises": [
                {"name": "慢跑", "kind": "cardio", "intensity": "moderate", "duration_min": 40}
            ]
        }}
    })));
    let meal = |meal_type: &str, grams: f64| {
        json!({"meal_type": meal_type, "title": "均衡餐",
               "items": [{"name": "均衡餐", "grams": grams}]})
    };
    mock.push_response(json_response(json!({
        "summary": "晚上慢跑，晚餐补充蛋白质。",
        "sections": {"diet": {"meals": [
            meal("breakfast", 300.0), meal("lunch", 400.0), meal("dinner", 300.0)
        ]}}
    })));
    mock.push_closure(estimate_for(tomorrow));

    let plan = app
        .generate_plan(tomorrow, PlanTrigger::Manual, None)
        .await
        .unwrap();

    assert_eq!(plan.content.summary, "晚上慢跑，晚餐补充蛋白质。");
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let training = &plan.content.sections["training"];
    assert_eq!(training["duration_min"], 40.0);
    assert_eq!(training["exercise_kcal"], SESSION_KCAL);
    let diet = &plan.content.sections["diet"];
    let expected = targets_on(tomorrow);
    assert_eq!(diet["exercise_kcal_added"], 160.0);
    assert_eq!(diet["targets"]["kcal"], expected.kcal);

    let prompts = planner_prompts(&mock);
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("## Section \"training\""));
    assert!(prompts[0].contains("User's request for this day: 晚上慢跑"));
    assert!(!prompts[0].contains("## Section \"diet\""));
    assert!(prompts[1].contains("## Already planned"));
    assert!(prompts[1].contains("## Section \"diet\""));
    assert!(prompts[1].contains(
        "Training that day (already planned): 慢跑, 40 min, in the evening. The targets above \
         already include 160 kcal extra (as carbs) for it."
    ));

    // Once stored, the day's diet target follows the plan's training.
    let card_target = lifeagent_plugin_diet::intake::targets_for(&app.plugin_context(), tomorrow)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card_target.kcal, expected.kcal);
}
