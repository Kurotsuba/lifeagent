//! The multimember plugin with the rest of the app: managed from chat (MM2), and shared meals
//! planned by the diet plugin from its computed section (MM4).

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::clock::FixedClock;
use lifeagent_core::events::EventSource;
use lifeagent_core::plan::PlanTrigger;
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_core::time::age_years;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatResponse, Message, ToolCall, Usage};
use lifeagent_plugin_diet::model::Macros;
use lifeagent_plugin_diet::sources::upsert_manual_food;
use lifeagent_plugin_diet::targets;
use lifeagent_plugin_diet::DietPlugin;
use lifeagent_plugin_multimember::model::MemberProfile;
use lifeagent_plugin_multimember::{store, MultimemberPlugin};
use lifeagent_plugin_stock::StockPlugin;
use lifeagent_plugin_training::TrainingPlugin;
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
        .send_message("Akiも一緒に食べます。明日の夜は飲み会。セロリが苦手。")
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

    // The next turn's system prompt carries the members, without body measurements.
    mock.push_response(text_response("ok"));
    app.send_message("明日の予定は？").await.unwrap();
    let system = format!("{:?}", mock.received_requests().last().unwrap().messages[0]);
    assert!(system.contains("Aki ("), "{system}");
    assert!(system.contains("Tomorrow"), "{system}");
}

// ---- MM4: shared meals ----------------------------------------------------------------------

const OWNER_KG: f64 = 78.0;

fn owner() -> Profile {
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

fn aki() -> MemberProfile {
    MemberProfile {
        display_name: "Aki".into(),
        sex: Sex::Female,
        birth_date: NaiveDate::from_ymd_opt(1992, 5, 10).unwrap(),
        height_cm: 160.0,
        weight_kg: 52.0,
        activity_level: ActivityLevel::Light,
        dietary_pattern: DietaryPattern::Omnivore,
        allergies: vec!["えび".into()],
    }
}

fn owner_kcal(date: NaiveDate) -> f64 {
    let p = owner();
    targets::compute(&p, OWNER_KG, age_years(p.birth_date, date), None).kcal
}

fn aki_maintenance_kcal(date: NaiveDate) -> f64 {
    let m = aki();
    let p = Profile {
        sex: m.sex,
        height_cm: m.height_cm,
        activity_level: m.activity_level,
        ..owner()
    };
    targets::compute(&p, m.weight_kg, age_years(m.birth_date, date), None).kcal
}

fn json_response(value: Value) -> ChatResponse {
    text_response(&value.to_string())
}

fn planner_prompts(mock: &MockChatModel) -> Vec<String> {
    mock.received_requests()
        .iter()
        .filter(|r| {
            matches!(r.messages.first(), Some(Message::User { content })
                if content.starts_with("You create"))
        })
        .map(|r| match r.messages.last() {
            Some(Message::User { content }) => content.clone(),
            _ => String::new(),
        })
        .collect()
}

/// Owner profile and weight, and foods: 均衡餐 per 100 g is a tenth of the owner's day targets,
/// so 1,000 g hits them; えびフライ and セロリ are tiny so they don't move the totals.
async fn seed_owner(env: &Env, date: NaiveDate) {
    let app = &env.app;
    app.profile().set(&owner()).await.unwrap();
    app.events()
        .append(
            "body.weight_logged",
            app.clock().now(),
            json!({"kg": OWNER_KG}),
            EventSource::Ui,
        )
        .await
        .unwrap();
    let p = owner();
    let t = targets::compute(&p, OWNER_KG, age_years(p.birth_date, date), None);
    let ctx = app.plugin_context();
    let tenth = Macros {
        kcal: t.kcal / 10.0,
        protein_g: t.protein_g / 10.0,
        carbs_g: t.carbs_g / 10.0,
        fat_g: t.fat_g / 10.0,
        fiber_g: t.fiber_g / 10.0,
    };
    upsert_manual_food(&ctx, "均衡餐", tenth).await.unwrap();
    let tiny = Macros {
        kcal: 1.0,
        protein_g: 0.0,
        carbs_g: 0.1,
        fat_g: 0.0,
        fiber_g: 0.1,
    };
    upsert_manual_food(&ctx, "えびフライ", tiny).await.unwrap();
    upsert_manual_food(&ctx, "セロリ", tiny).await.unwrap();
}

fn planned(dinner_extra: &str) -> Value {
    let meal = |meal_type: &str, grams: f64, extra: Option<&str>| {
        let mut items = vec![json!({"name": "均衡餐", "grams": grams})];
        if let Some(name) = extra {
            items.push(json!({"name": name, "grams": 10}));
        }
        json!({"meal_type": meal_type, "title": "均衡餐", "items": items})
    };
    json!({
        "summary": "shared dinner",
        "sections": {
            "diet": {"meals": [
                meal("breakfast", 300.0, None),
                meal("lunch", 400.0, None),
                meal("dinner", 300.0, Some(dinner_extra)),
            ]},
            "stock": {"uses": [], "shopping_list": []}
        }
    })
}

#[tokio::test]
async fn a_shared_dinner_is_planned_with_guest_constraints_and_portions() {
    let env = app_at(local(2026, 10, 4, 10)).await;
    let (app, mock) = (&env.app, &env.mock);
    let tomorrow = app.logical_today().await.unwrap().succ_opt().unwrap();
    seed_owner(&env, tomorrow).await;
    app.set_plugin_enabled("multimember", true).await.unwrap();
    let ctx = app.plugin_context();
    let aki_m = store::add_member(&ctx, &aki(), None).await.unwrap();
    store::add_dislike(&ctx, &aki_m.id, "セロリ", None, None)
        .await
        .unwrap();
    // Ken is 70 today and 71 on the plan date: left out with a warning.
    let ken = MemberProfile {
        display_name: "Ken".into(),
        birth_date: NaiveDate::from_ymd_opt(1955, 10, 5).unwrap(),
        allergies: vec![],
        ..aki()
    };
    store::add_member(&ctx, &ken, None).await.unwrap();

    // First answer: Aki's allergen at dinner, a violation. Second: only a disliked item.
    mock.push_response(json_response(planned("えびフライ")));
    mock.push_response(json_response(planned("セロリ")));
    let plan = app
        .generate_plan(tomorrow, PlanTrigger::Manual, None)
        .await
        .unwrap();

    // Diet and stock in one planner call and its retry; no call for the computed section.
    assert_eq!(planner_prompts(mock).len(), 2);
    let prompt = match mock.received_requests()[0].messages.first() {
        Some(Message::User { content }) => content.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        prompt.contains("- Shared dinner with Aki: cook about"),
        "{prompt}"
    );
    assert!(prompt.contains("Never include (allergies at this table, hard constraint): えび"));
    assert!(prompt.contains("Avoid (guests' dislikes): セロリ"));
    assert!(prompt.contains("cover every portion"));
    assert!(!prompt.contains("Ken"));
    // Guests' body measurements never reach the model.
    assert!(!prompt.contains("multimember"), "{prompt}");
    assert!(!prompt.contains("weight_kg"), "{prompt}");
    let requests = mock.received_requests();
    let retry = match requests.last().unwrap().messages.last() {
        Some(Message::User { content }) => content.clone(),
        other => panic!("{other:?}"),
    };
    assert!(retry.contains("含Aki的过敏原“えび”"), "{retry}");

    let diet = &plan.content.sections["diet"];
    let dinner = &diet["meals"][2];
    let expected_scale =
        ((aki_maintenance_kcal(tomorrow) / owner_kcal(tomorrow)) * 100.0).round() / 100.0;
    let servings = dinner["servings"].as_array().unwrap();
    assert_eq!(servings.len(), 1);
    assert_eq!(servings[0]["member_id"], aki_m.id.as_str());
    assert_eq!(servings[0]["name"], "Aki");
    assert_eq!(servings[0]["scale"], expected_scale);
    let dinner_kcal = dinner["total"]["kcal"].as_f64().unwrap();
    let serving_kcal = servings[0]["total"]["kcal"].as_f64().unwrap();
    assert!((serving_kcal - dinner_kcal * expected_scale).abs() <= 1.0);
    assert_eq!(
        dinner["portions"],
        ((1.0 + expected_scale) * 10.0).round() / 10.0
    );
    // Ingredient amounts for the whole table; `grams` and kcal stay the owner's own.
    let factor = 1.0 + expected_scale;
    for item in dinner["items"].as_array().unwrap() {
        let grams = item["grams"].as_f64().unwrap();
        let table = item["table_grams"].as_f64().unwrap();
        assert!((table - grams * factor).abs() <= 0.1, "{item}");
    }
    assert_eq!(dinner["items"][0]["grams"], 300.0);
    assert!(diet["meals"][0].get("servings").is_none());
    assert!(diet["meals"][0]["items"][0].get("table_grams").is_none());
    assert_eq!(
        diet["guest_estimates"][aki_m.id.as_str()]["kcal"],
        aki_maintenance_kcal(tomorrow)
    );
    assert_eq!(
        diet["guest_estimates"].as_object().unwrap().len(),
        1,
        "only guests who share a meal"
    );
    // The owner's own targets are untouched by guests.
    assert_eq!(diet["targets"]["kcal"], owner_kcal(tomorrow));

    assert!(plan.content.sections.contains_key("multimember"));
    assert!(
        plan.warnings.iter().any(|w| w.contains("Ken")),
        "{:?}",
        plan.warnings
    );
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("Aki不喜欢的“セロリ”")),
        "{:?}",
        plan.warnings
    );
}

#[tokio::test]
async fn without_guests_the_diet_section_is_unchanged() {
    let env = app_at(local(2026, 10, 4, 10)).await;
    let (app, mock) = (&env.app, &env.mock);
    let tomorrow = app.logical_today().await.unwrap().succ_opt().unwrap();
    seed_owner(&env, tomorrow).await;
    app.set_plugin_enabled("multimember", true).await.unwrap();
    // A member who shares no meal.
    store::add_member(&app.plugin_context(), &aki(), Some(&[]))
        .await
        .unwrap();

    mock.push_response(json_response(planned("セロリ")));
    let plan = app
        .generate_plan(tomorrow, PlanTrigger::Manual, None)
        .await
        .unwrap();

    assert!(!plan.content.sections.contains_key("multimember"));
    let diet = &plan.content.sections["diet"];
    assert!(diet.get("guest_estimates").is_none());
    for meal in diet["meals"].as_array().unwrap() {
        assert!(meal.get("servings").is_none() && meal.get("portions").is_none());
    }
    let prompt = match mock.received_requests()[0].messages.first() {
        Some(Message::User { content }) => content.clone(),
        other => panic!("{other:?}"),
    };
    assert!(!prompt.contains("Shared"));
    assert!(!prompt.contains("cover every portion"));
}
