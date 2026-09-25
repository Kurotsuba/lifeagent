//! Real Tauri IPC round-trips through the actual `#[tauri::command]` wrappers in
//! `commands.rs`, per `docs/PHASE1_SPEC.md` §10.2.
//!
//! Everything else in this workspace tests business logic through plain Rust calls; that proves
//! `CoreApp`'s own methods are correct, but not that a given command is wired up with the right
//! argument names/casing, or that its inputs/outputs actually (de)serialize the way the frontend
//! will send/expect them. `tauri::test`'s mock runtime lets these go through the real
//! `invoke_handler` and real JSON, without a live window.

use std::sync::Arc;

use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_plugin_diet::DietPlugin;
use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_builder, MockRuntime};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

async fn core_app() -> (CoreApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let app = CoreAppBuilder::new(dir.path().to_path_buf())
        .plugin(Arc::new(DietPlugin))
        .build()
        .await
        .unwrap();
    (app, dir)
}

/// Uses the project's *real* `tauri.conf.json` and `capabilities/` (via `generate_context!`),
/// not `tauri::test::mock_context`'s bare-bones one — command permissions are enforced by the
/// real ACL regardless of runtime, so a mock context with no capabilities would reject every
/// command with "Plugin not found" and prove nothing. Returns the one "main" webview a test
/// invokes every command through — a second `WebviewWindowBuilder` call with the same label
/// conflicts, so callers must reuse this rather than building another.
fn setup(core_app: CoreApp) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            lifeagent_desktop_lib::commands::get_app_state,
            lifeagent_desktop_lib::commands::complete_onboarding,
            lifeagent_desktop_lib::commands::send_message,
            lifeagent_desktop_lib::commands::get_today,
            lifeagent_desktop_lib::commands::get_profile,
            lifeagent_desktop_lib::commands::save_profile,
            lifeagent_desktop_lib::commands::get_weight_goal,
            lifeagent_desktop_lib::commands::save_weight_goal,
            lifeagent_desktop_lib::commands::log_weight,
            lifeagent_desktop_lib::commands::preview_targets,
            lifeagent_desktop_lib::commands::set_api_key,
            lifeagent_desktop_lib::commands::has_api_key,
            lifeagent_desktop_lib::commands::test_model,
        ])
        .manage(core_app)
        .build(tauri::generate_context!())
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    (app, webview)
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    tauri::test::get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|b| b.deserialize::<Value>().unwrap())
}

fn sample_profile() -> Value {
    json!({
        "display_name": null,
        "sex": "female",
        "birth_date": "1996-05-01",
        "height_cm": 165.0,
        "activity_level": "moderate",
        "dietary_pattern": "omnivore",
        "allergies": [],
        "dislikes": [],
        "medical_notes": null,
        "meals_per_day": 3,
    })
}

#[tokio::test]
async fn get_app_state_before_onboarding() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let state = invoke(&webview, "get_app_state", json!({})).unwrap();

    assert_eq!(state["onboarding_complete"], json!(false));
    assert_eq!(state["models_configured"], json!(false));
    assert!(state["logical_date"].is_string());
}

#[tokio::test]
async fn save_profile_then_get_profile_round_trips_through_real_ipc() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    invoke(
        &webview,
        "save_profile",
        json!({ "profile": sample_profile() }),
    )
    .unwrap();
    let got = invoke(&webview, "get_profile", json!({})).unwrap();

    assert_eq!(got, sample_profile());
}

#[tokio::test]
async fn log_weight_then_get_today_reflects_it() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    invoke(&webview, "log_weight", json!({ "kg": 62.5 })).unwrap();
    let today = invoke(&webview, "get_today", json!({})).unwrap();

    assert_eq!(today["latest_weight"]["kg"], json!(62.5));
}

#[tokio::test]
async fn send_message_without_models_configured_returns_the_not_configured_error_code() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let err = invoke(&webview, "send_message", json!({ "text": "你好" })).unwrap_err();

    assert_eq!(err["code"], json!("not_configured"));
    assert!(err["message"].as_str().unwrap().contains("模型"));
}

/// Also the multi-word-argument case (`provider_id` → JS `providerId`), which single-word
/// commands like the other tests here can't exercise.
#[tokio::test]
async fn set_api_key_then_has_api_key_round_trips_with_camel_case_args() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    invoke(
        &webview,
        "set_api_key",
        json!({ "providerId": "anthropic", "key": "sk-test" }),
    )
    .unwrap();

    assert_eq!(
        invoke(
            &webview,
            "has_api_key",
            json!({ "providerId": "anthropic" })
        )
        .unwrap(),
        json!(true)
    );
    assert_eq!(
        invoke(&webview, "has_api_key", json!({ "providerId": "openai" })).unwrap(),
        json!(false)
    );
}

#[tokio::test]
async fn save_weight_goal_then_get_weight_goal_round_trips() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let goal = json!({
        "direction": "lose",
        "target_weight_kg": 65.0,
        "rate_kg_per_week": 0.5,
    });
    invoke(&webview, "save_weight_goal", json!({ "goal": goal })).unwrap();

    assert_eq!(
        invoke(&webview, "get_weight_goal", json!({})).unwrap(),
        goal
    );
}

#[tokio::test]
async fn complete_onboarding_flips_get_app_state() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    invoke(&webview, "complete_onboarding", json!({})).unwrap();

    let state = invoke(&webview, "get_app_state", json!({})).unwrap();
    assert_eq!(state["onboarding_complete"], json!(true));
}

/// The onboarding step-2 preview: a pure computation, so this is safe to run offline (no LLM
/// call involved) unlike `test_model` below.
#[tokio::test]
async fn preview_targets_returns_nutrition_targets_for_a_valid_profile() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let goal =
        json!({ "direction": "maintain", "target_weight_kg": null, "rate_kg_per_week": 0.5 });
    let targets = invoke(
        &webview,
        "preview_targets",
        json!({ "profile": sample_profile(), "weightKg": 60.0, "goal": goal }),
    )
    .unwrap();

    assert!(targets["kcal"].as_f64().unwrap() > 0.0);
    assert!(targets["protein_g"].as_f64().unwrap() > 0.0);
}

/// `test_model` must resolve with `{ ok: false, error: Some(...) }`, never reject the promise,
/// even with no models configured at all — onboarding calls it before anything is saved.
#[tokio::test]
async fn test_model_without_models_configured_resolves_ok_false() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let result = invoke(&webview, "test_model", json!({ "role": "conversation" })).unwrap();

    assert_eq!(result["ok"], json!(false));
    assert!(result["error"].is_string());
}
