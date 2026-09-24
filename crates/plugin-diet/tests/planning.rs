//! M5 "done when" (`docs/PHASE1_SPEC.md` §14): planning and scheduler integration tests,
//! against the real diet plugin (§13).

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::{Clock, FixedClock};
use lifeagent_core::events::{AppEvent, EventSource};
use lifeagent_core::goals::{GoalDirection, WeightGoal};
use lifeagent_core::plan::PlanTrigger;
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use lifeagent_core::scheduler::Scheduler;
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_core::time::age_years;
use lifeagent_llm::error::LlmError;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatRequest, ChatResponse, Message, Usage};
use lifeagent_plugin_diet::targets::{self, NutritionTargets};
use lifeagent_plugin_diet::DietPlugin;
use serde_json::{json, Value};
use tokio::sync::broadcast;

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
        dislikes: vec![],
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
    targets::compute(&p, WEIGHT_KG, age_years(p.birth_date, date), Some(&goal()))
}

fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    let naive = NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, min, 0)
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

/// A planner reply whose diet section is breakfast/lunch/dinner of 均衡餐, `dinner_grams` for
/// dinner. With 300 g, the day is 1,000 g: exactly on target (see `estimate_for`).
fn planner_reply(summary: &str, dinner_grams: f64) -> ChatResponse {
    let meal = |meal_type: &str, grams: f64| {
        json!({
            "meal_type": meal_type,
            "title": format!("{meal_type} 均衡餐"),
            "items": [{"name": "均衡餐", "grams": grams}]
        })
    };
    json_response(json!({
        "summary": summary,
        "sections": {"diet": {"meals": [
            meal("breakfast", 300.0),
            meal("lunch", 400.0),
            meal("dinner", dinner_grams),
        ]}}
    }))
}

/// Answers the food estimate for 均衡餐 with a tenth of the day's targets per 100 g, so
/// 1,000 g of it hits every target.
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

struct Env {
    app: CoreApp,
    clock: Arc<FixedClock>,
    mock: Arc<MockChatModel>,
    dir: tempfile::TempDir,
}

async fn env_at(now: DateTime<Utc>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(now));
    let mock = Arc::new(MockChatModel::new());
    let app = CoreAppBuilder::new(dir.path().to_path_buf())
        .clock(clock.clone())
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin))
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
    app.settings()
        .set_onboarding_complete(true, now)
        .await
        .unwrap();
    Env {
        app,
        clock,
        mock,
        dir,
    }
}

fn planner_requests(mock: &MockChatModel) -> Vec<String> {
    mock.received_requests()
        .iter()
        .filter_map(|r| match r.messages.first() {
            Some(Message::User { content }) if content.starts_with("You create") => {
                match r.messages.last() {
                    Some(Message::User { content }) => Some(content.clone()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn plan_ready_events(rx: &mut broadcast::Receiver<AppEvent>) -> Vec<(NaiveDate, u32, PlanTrigger)> {
    let mut found = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::PlanReady {
            date,
            version,
            trigger,
        } = event
        {
            found.push((date, version, trigger));
        }
    }
    found
}

#[tokio::test]
async fn kcal_violation_is_retried_with_the_violation_then_stored_as_version_1() {
    let env = env_at(local(2026, 9, 24, 10, 0)).await;
    let tomorrow = env.app.logical_today().await.unwrap().succ_opt().unwrap();
    let mut rx = env.app.subscribe();
    // Dinner 100 g: the day is 800 g, 80% of the kcal target.
    env.mock.push_response(planner_reply("first", 100.0));
    env.mock.push_closure(estimate_for(tomorrow));
    env.mock
        .push_response(planner_reply("明天三餐均衡，继续保持！", 300.0));

    let plan = env
        .app
        .generate_plan(tomorrow, PlanTrigger::Manual)
        .await
        .unwrap();

    assert_eq!(plan.version, 1);
    assert_eq!(plan.trigger, PlanTrigger::Manual);
    assert_eq!(plan.content.summary, "明天三餐均衡，继续保持！");
    assert!(
        plan.warnings.iter().all(|w| !w.starts_with("未完全满足")),
        "{:?}",
        plan.warnings
    );
    let t = targets_on(tomorrow);
    assert_eq!(
        plan.content.sections["diet"]["total"]["kcal"],
        t.kcal.round()
    );

    // The retry (second planner request) names the violation.
    let planner = planner_requests(&env.mock);
    assert_eq!(planner.len(), 2);
    assert!(planner[1].contains("## Section \"diet\""));
    assert!(planner[1].contains("全天热量"));

    assert_eq!(env.app.get_plan(tomorrow).await.unwrap(), Some(plan));
    assert_eq!(
        plan_ready_events(&mut rx),
        vec![(tomorrow, 1, PlanTrigger::Manual)]
    );
}

#[tokio::test]
async fn revision_creates_version_2_with_trigger_revision_and_the_instruction() {
    let env = env_at(local(2026, 9, 24, 10, 0)).await;
    let tomorrow = env.app.logical_today().await.unwrap().succ_opt().unwrap();
    env.mock.push_response(planner_reply("v1", 300.0));
    env.mock.push_closure(estimate_for(tomorrow));
    env.mock.push_response(planner_reply("v2", 300.0));
    env.app
        .generate_plan(tomorrow, PlanTrigger::Manual)
        .await
        .unwrap();

    let revised = env
        .app
        .revise_plan(tomorrow, "明天中午公司聚餐，吃不了计划里的午餐")
        .await
        .unwrap();

    assert_eq!(revised.version, 2);
    assert_eq!(revised.trigger, PlanTrigger::Revision);
    assert_eq!(
        revised.instruction.as_deref(),
        Some("明天中午公司聚餐，吃不了计划里的午餐")
    );
    let planner = planner_requests(&env.mock);
    assert!(planner[1].contains("## Current plan (version 1)"));
    assert!(planner[1].contains("## User's change request\n明天中午公司聚餐"));

    env.app.accept_plan(tomorrow).await.unwrap();
    let current = env.app.get_plan(tomorrow).await.unwrap().unwrap();
    assert_eq!(current.version, 2);
    assert_eq!(current.status, lifeagent_core::plan::PlanStatus::Accepted);
}

#[tokio::test]
async fn scheduler_catches_up_after_midnight_once_and_takes_the_backup() {
    // 01:00 on the 24th is still logical day 23 (boundary 04:00), and plan_time (21:00) on
    // the 23rd has passed: catch-up must generate the plan for the 24th (B3).
    let env = env_at(local(2026, 9, 24, 1, 0)).await;
    let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
    let tomorrow = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
    assert_eq!(env.app.logical_today().await.unwrap(), today);
    env.mock.push_response(planner_reply("s", 300.0));
    env.mock.push_closure(estimate_for(tomorrow));
    let mut scheduler = Scheduler::new(env.app.clone());

    scheduler.tick().await;
    scheduler.tick().await;

    let plan = env.app.get_plan(tomorrow).await.unwrap().unwrap();
    assert_eq!(plan.trigger, PlanTrigger::Schedule);
    // Exactly one generation: the second tick found the plan and did nothing.
    assert_eq!(planner_requests(&env.mock).len(), 1);
    // Catch-up only targets tomorrow; today's missing plan is not backfilled.
    assert_eq!(env.app.get_plan(today).await.unwrap(), None);
    assert!(env
        .dir
        .path()
        .join("backups/lifeagent-2026-09-23.db")
        .exists());
}

#[tokio::test]
async fn scheduler_does_nothing_before_plan_time() {
    let env = env_at(local(2026, 9, 23, 20, 59)).await;
    let mut scheduler = Scheduler::new(env.app.clone());

    scheduler.tick().await;

    assert!(env.mock.received_requests().is_empty());
    // The backup doesn't wait for plan_time.
    assert!(env
        .dir
        .path()
        .join("backups/lifeagent-2026-09-23.db")
        .exists());
}

#[tokio::test]
async fn scheduler_skips_generation_until_onboarding_is_complete() {
    let env = env_at(local(2026, 9, 23, 22, 0)).await;
    env.app
        .settings()
        .set_onboarding_complete(false, env.clock.now())
        .await
        .unwrap();
    let mut scheduler = Scheduler::new(env.app.clone());

    scheduler.tick().await;

    assert!(env.mock.received_requests().is_empty());
}

#[tokio::test]
async fn failed_generation_emits_plan_failed_and_retries_every_15_minutes_at_most_3_times() {
    let env = env_at(local(2026, 9, 23, 22, 0)).await;
    let tomorrow = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
    for _ in 0..4 {
        env.mock.push_error(LlmError::RateLimited);
    }
    let mut rx = env.app.subscribe();
    let mut scheduler = Scheduler::new(env.app.clone());

    scheduler.tick().await; // attempt 1
    scheduler.tick().await; // too soon to retry
    assert_eq!(env.mock.received_requests().len(), 1);
    let failed: Vec<NaiveDate> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|e| match e {
            AppEvent::PlanFailed { date, .. } => Some(date),
            _ => None,
        })
        .collect();
    assert_eq!(failed, vec![tomorrow]);

    for _ in 0..3 {
        env.clock.advance(chrono::Duration::minutes(15));
        scheduler.tick().await;
    }
    // Attempts 2 and 3, then nothing: 3 attempts in total.
    assert_eq!(env.mock.received_requests().len(), 3);
    assert_eq!(env.app.get_plan(tomorrow).await.unwrap(), None);
}
