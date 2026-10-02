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
use lifeagent_core::secrets::MemorySecretStore;
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::types::{ChatResponse, Usage};
use lifeagent_plugin_diet::DietPlugin;
use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_builder, MockRuntime};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

/// Always an in-memory `SecretStore`: the builder's default `KeyringSecretStore` is the real OS
/// credential store, so `set_api_key` here would overwrite the developer's actual key (e.g.
/// `provider:anthropic`) on every `cargo test` run.
async fn core_app() -> (CoreApp, lifeagent_core::db::TestDb) {
    build_core_app(None).await
}

/// `core_app` with every LLM role answered by `mock` — never a real provider (CLAUDE.md).
async fn core_app_with_mock(mock: Arc<MockChatModel>) -> (CoreApp, lifeagent_core::db::TestDb) {
    build_core_app(Some(mock)).await
}

async fn build_core_app(mock: Option<Arc<MockChatModel>>) -> (CoreApp, lifeagent_core::db::TestDb) {
    let dir = lifeagent_core::db::TestDb::new().await;
    let mut builder = CoreAppBuilder::new(dir.config.clone())
        .secrets(Arc::new(MemorySecretStore::new()))
        .plugin(Arc::new(DietPlugin));
    if let Some(mock) = mock {
        builder = builder.chat_model_override(mock);
    }
    (builder.build().await.unwrap(), dir)
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
            lifeagent_desktop_lib::commands::get_history,
            lifeagent_desktop_lib::commands::get_plan,
            lifeagent_desktop_lib::commands::generate_plan,
            lifeagent_desktop_lib::commands::accept_plan,
            lifeagent_desktop_lib::commands::get_settings,
            lifeagent_desktop_lib::commands::get_models,
            lifeagent_desktop_lib::commands::save_models,
            lifeagent_desktop_lib::commands::get_memory,
            lifeagent_desktop_lib::commands::delete_memory,
            lifeagent_desktop_lib::commands::get_usage_summary,
            lifeagent_desktop_lib::commands::list_food_sources,
            lifeagent_desktop_lib::commands::list_foods,
            lifeagent_desktop_lib::commands::reorder_food_sources,
            lifeagent_desktop_lib::commands::set_food_source_enabled,
            lifeagent_desktop_lib::commands::rename_food_source,
            lifeagent_desktop_lib::commands::delete_food_source,
            lifeagent_desktop_lib::commands::preview_food_import,
            lifeagent_desktop_lib::commands::commit_food_import,
            lifeagent_desktop_lib::commands::cancel_food_import,
            // `save_settings` is deliberately absent: it syncs OS autostart, which a test must
            // not touch for real (see `docs/DECISIONS.md`'s M7.5 entry).
        ])
        .manage(core_app)
        .manage(lifeagent_api::ops::PendingImportState::default())
        .build(tauri::generate_context!())
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    (app, webview)
}

// Every test runs on a multi-thread runtime: `get_ipc_response` blocks the test thread until
// the command finishes, and the command's Postgres connections are driven by the test's
// runtime. On a current-thread runtime nothing would drive them and every query would hang
// until the pool's acquire timeout.
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

/// `preview_food_import`'s raw-body IPC shape: file bytes as the body, the file name url-encoded
/// in `x-file-name` (§8.2 "Getting the file to Rust").
fn invoke_raw_file(
    webview: &WebviewWindow<MockRuntime>,
    cmd: &str,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<Value, Value> {
    let mut headers = tauri::http::HeaderMap::new();
    headers.insert(
        "x-file-name",
        tauri::http::HeaderValue::from_str(&urlencoding_encode(file_name)).unwrap(),
    );
    tauri::test::get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Raw(bytes),
            headers,
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|b| b.deserialize::<Value>().unwrap())
}

/// Minimal `encodeURIComponent`-alike for test file names (ASCII and UTF-8 bytes as `%XX`).
fn urlencoding_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
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

#[tokio::test(flavor = "multi_thread")]
async fn get_app_state_before_onboarding() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let state = invoke(&webview, "get_app_state", json!({})).unwrap();

    assert_eq!(state["onboarding_complete"], json!(false));
    assert_eq!(state["models_configured"], json!(false));
    assert!(state["logical_date"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn log_weight_then_get_today_reflects_it() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    invoke(&webview, "log_weight", json!({ "kg": 62.5 })).unwrap();
    let today = invoke(&webview, "get_today", json!({})).unwrap();

    assert_eq!(today["latest_weight"]["kg"], json!(62.5));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_without_models_configured_returns_the_not_configured_error_code() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let err = invoke(&webview, "send_message", json!({ "text": "你好" })).unwrap_err();

    assert_eq!(err["code"], json!("not_configured"));
    assert!(err["message"].as_str().unwrap().contains("模型"));
}

/// Also the multi-word-argument case (`provider_id` → JS `providerId`), which single-word
/// commands like the other tests here can't exercise.
#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
async fn complete_onboarding_flips_get_app_state() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    invoke(&webview, "complete_onboarding", json!({})).unwrap();

    let state = invoke(&webview, "get_app_state", json!({})).unwrap();
    assert_eq!(state["onboarding_complete"], json!(true));
}

/// The onboarding step-2 preview: a pure computation, so this is safe to run offline (no LLM
/// call involved) unlike `test_model` below.
#[tokio::test(flavor = "multi_thread")]
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
#[tokio::test(flavor = "multi_thread")]
async fn test_model_without_models_configured_resolves_ok_false() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let result = invoke(&webview, "test_model", json!({ "role": "conversation" })).unwrap();

    assert_eq!(result["ok"], json!(false));
    assert_eq!(result["error"]["code"], json!("not_configured"));
    assert_eq!(result["error"]["detail"]["kind"], json!("not_configured"));
}

/// The quick-entry popup hides itself (Esc, and after showing a reply) via the JS
/// `getCurrentWindow().hide()`, which needs `core:window:allow-hide` — `core:default` alone only
/// grants read-only window permissions, so without it the call is silently rejected and the
/// undecorated popup can't be dismissed. Also checks the window stays least-privilege: none of
/// the main window's data commands are reachable from it.
#[tokio::test(flavor = "multi_thread")]
async fn quick_entry_window_can_hide_itself_but_not_reach_data_commands() {
    let (core_app, _dir) = core_app().await;
    let (app, _main) = setup(core_app);
    let quick_entry = WebviewWindowBuilder::new(&app, "quick-entry", Default::default())
        .build()
        .unwrap();

    invoke(
        &quick_entry,
        "plugin:window|hide",
        json!({ "label": "quick-entry" }),
    )
    .unwrap();

    assert!(invoke(&quick_entry, "get_today", json!({})).is_err());
}

fn text_response(content: &str) -> ChatResponse {
    ChatResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Usage::default(),
        finish_reason: "stop".into(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_then_get_history_returns_both_sides_of_the_turn() {
    let mock = Arc::new(MockChatModel::new());
    mock.push_response(text_response("你好！"));
    let (core_app, _dir) = core_app_with_mock(mock).await;
    let (_app, webview) = setup(core_app);

    let turn = invoke(&webview, "send_message", json!({ "text": "在吗" })).unwrap();
    assert_eq!(turn["reply"], json!("你好！"));
    assert_eq!(turn["activities"], json!([]));

    let history = invoke(&webview, "get_history", json!({ "limit": 10 })).unwrap();
    let pairs: Vec<(Value, Value)> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["role"].clone(), m["content"].clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (json!("user"), json!("在吗")),
            (json!("assistant"), json!("你好！")),
        ]
    );
    assert!(history[0]["created_at"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_commands_report_missing_plans_and_bad_dates_with_the_right_codes() {
    let (core_app, _dir) = core_app_with_mock(Arc::new(MockChatModel::new())).await;
    let (_app, webview) = setup(core_app);

    assert_eq!(
        invoke(&webview, "get_plan", json!({ "date": "2026-09-27" })).unwrap(),
        Value::Null
    );
    let bad_date = invoke(&webview, "get_plan", json!({ "date": "27/09/2026" })).unwrap_err();
    assert_eq!(bad_date["code"], json!("invalid_input"));

    let accept = invoke(&webview, "accept_plan", json!({ "date": "2026-09-27" })).unwrap_err();
    assert_eq!(accept["code"], json!("not_found"));

    // No profile or weight yet: nothing to plan from, reported as the user's own input problem
    // (with its own zh-CN hint), before any model call is made.
    let generate = invoke(&webview, "generate_plan", json!({ "date": null })).unwrap_err();
    assert_eq!(generate["code"], json!("invalid_input"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_settings_returns_the_defaults_in_the_dto_shape() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let settings = invoke(&webview, "get_settings", json!({})).unwrap();

    assert_eq!(settings["plan_time"], json!("21:00"));
    assert_eq!(settings["day_boundary"], json!("04:00"));
    assert!(settings["notifications_enabled"].is_boolean());
    assert!(settings["autostart"].is_boolean());
}

/// Also proves `save_models` reloads the router on the running app: `models_configured` flips
/// without a restart, which onboarding's save-then-test flow depends on.
#[tokio::test(flavor = "multi_thread")]
async fn save_models_then_get_models_round_trips_and_configures_the_running_app() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);
    assert_eq!(
        invoke(&webview, "get_models", json!({})).unwrap(),
        Value::Null
    );

    let binding = |model: &str| json!({ "provider_id": "anthropic", "model": model, "temperature": null, "effort": null });
    let models = json!({
        "providers": [
            { "id": "anthropic", "kind": "anthropic", "base_url": "https://api.anthropic.com" }
        ],
        "parser": binding("claude-haiku-4-5"),
        "planner": binding("claude-opus-5"),
        "conversation": binding("claude-sonnet-5"),
    });
    invoke(
        &webview,
        "set_api_key",
        json!({ "providerId": "anthropic", "key": "sk-test" }),
    )
    .unwrap();
    invoke(&webview, "save_models", json!({ "models": models })).unwrap();

    assert_eq!(invoke(&webview, "get_models", json!({})).unwrap(), models);
    let state = invoke(&webview, "get_app_state", json!({})).unwrap();
    assert_eq!(state["models_configured"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_and_usage_commands_work_on_an_empty_store() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    assert_eq!(
        invoke(&webview, "get_memory", json!({ "limit": 100 })).unwrap(),
        json!([])
    );
    let delete = invoke(&webview, "delete_memory", json!({ "id": "no-such-note" })).unwrap_err();
    assert_eq!(delete["code"], json!("not_found"));

    assert!(invoke(&webview, "get_usage_summary", json!({}))
        .unwrap()
        .is_array());
}

// ---- Nutrition sources / batch import (`docs/NUTRITION_SOURCES.md` §8.2) --------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_food_sources_starts_with_the_two_built_in_sources() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let sources = invoke(&webview, "list_food_sources", json!({})).unwrap();
    let ids: Vec<&str> = sources
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["manual", "llm_estimate"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_then_commit_food_import_round_trips_through_raw_ipc() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let csv = "name,kcal,protein_g,carbs_g,fat_g,fiber_g\n熟米饭,116,2.6,25.9,0.3,0.3\n";
    let preview = invoke_raw_file(&webview, "preview_food_import", "米饭.csv", csv.into()).unwrap();
    assert_eq!(preview["valid_rows"], json!(1));
    assert_eq!(preview["error_count"], json!(0));
    assert_eq!(preview["suggested_name"], json!("米饭"));
    let token = preview["token"].as_str().unwrap().to_string();

    let report = invoke(
        &webview,
        "commit_food_import",
        json!({ "token": token, "name": "我的数据集" }),
    )
    .unwrap();
    assert_eq!(report["imported"], json!(1));
    assert_eq!(report["source"]["name"], json!("我的数据集"));

    let foods = invoke(&webview, "list_foods", json!({ "limit": 50 })).unwrap();
    assert_eq!(foods.as_array().unwrap().len(), 1);
    assert_eq!(foods[0]["name"], json!("熟米饭"));
}

/// `docs/DISH_NUTRITION.md` §8.2: `list_foods` gains `kind`/`brand`, and a source's dish count
/// shows up alongside its food count.
#[tokio::test(flavor = "multi_thread")]
async fn list_foods_with_kind_dish_returns_dishes_filtered_by_brand() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let csv = "name,brand,basis,basis_g,kcal,protein_g,carbs_g,fat_g\n\
               熟米饭,,100g,,116,2.6,25.9,0.3\n\
               牛めし（並盛）,松屋,serving,350,733,25,90,28\n\
               牛丼,吉野家,serving,,650,24,85,26\n";
    let preview = invoke_raw_file(&webview, "preview_food_import", "menu.csv", csv.into()).unwrap();
    assert_eq!(preview["valid_food_rows"], json!(1));
    assert_eq!(preview["valid_dish_rows"], json!(2));
    assert_eq!(preview["brands"], json!(["吉野家", "松屋"]));
    let token = preview["token"].as_str().unwrap().to_string();

    let report = invoke(&webview, "commit_food_import", json!({ "token": token })).unwrap();
    assert_eq!(report["imported"], json!(1));
    assert_eq!(report["imported_dishes"], json!(2));

    let sources = invoke(&webview, "list_food_sources", json!({})).unwrap();
    let menu_source = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == json!("menu"))
        .unwrap();
    assert_eq!(menu_source["food_count"], json!(1));
    assert_eq!(menu_source["dish_count"], json!(2));

    let all_dishes = invoke(
        &webview,
        "list_foods",
        json!({ "limit": 50, "kind": "dish" }),
    )
    .unwrap();
    assert_eq!(all_dishes.as_array().unwrap().len(), 2);

    let matsuya_dishes = invoke(
        &webview,
        "list_foods",
        json!({ "limit": 50, "kind": "dish", "brand": "松屋" }),
    )
    .unwrap();
    let matsuya = matsuya_dishes.as_array().unwrap();
    assert_eq!(matsuya.len(), 1);
    assert_eq!(matsuya[0]["name"], json!("牛めし（並盛）"));
    assert_eq!(matsuya[0]["serving_g"], json!(350.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_food_import_consumes_the_token_once() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let csv = "name,kcal,protein_g,carbs_g,fat_g\nx,100,1,1,1\n";
    let preview = invoke_raw_file(&webview, "preview_food_import", "x.csv", csv.into()).unwrap();
    let token = preview["token"].as_str().unwrap().to_string();

    invoke(&webview, "commit_food_import", json!({ "token": token })).unwrap();
    let second = invoke(&webview, "commit_food_import", json!({ "token": token })).unwrap_err();
    assert_eq!(second["code"], json!("not_found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_food_import_then_commit_reports_not_found() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let csv = "name,kcal,protein_g,carbs_g,fat_g\nx,100,1,1,1\n";
    let preview = invoke_raw_file(&webview, "preview_food_import", "x.csv", csv.into()).unwrap();
    let token = preview["token"].as_str().unwrap().to_string();

    invoke(&webview, "cancel_food_import", json!({ "token": token })).unwrap();
    let err = invoke(&webview, "commit_food_import", json!({ "token": token })).unwrap_err();
    assert_eq!(err["code"], json!("not_found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_food_import_reports_row_errors_with_a_zh_cn_message() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    // Not UTF-8.
    let bytes = vec![0xFF, 0xFE, 0x00];
    let err = invoke_raw_file(&webview, "preview_food_import", "bad.csv", bytes).unwrap_err();
    assert_eq!(err["code"], json!("invalid_input"));
    assert!(err["message"].as_str().unwrap().contains("UTF-8"));
}

#[tokio::test(flavor = "multi_thread")]
async fn source_management_commands_reorder_rename_disable_and_delete() {
    let (core_app, _dir) = core_app().await;
    let (_app, webview) = setup(core_app);

    let csv1 = "name,kcal,protein_g,carbs_g,fat_g\na,100,1,1,1\n";
    let p1 = invoke_raw_file(&webview, "preview_food_import", "a.csv", csv1.into()).unwrap();
    let r1 = invoke(
        &webview,
        "commit_food_import",
        json!({ "token": p1["token"], "name": "数据集A" }),
    )
    .unwrap();
    let id1 = r1["source"]["id"].as_str().unwrap().to_string();

    let csv2 = "name,kcal,protein_g,carbs_g,fat_g\nb,100,1,1,1\n";
    let p2 = invoke_raw_file(&webview, "preview_food_import", "b.csv", csv2.into()).unwrap();
    let r2 = invoke(
        &webview,
        "commit_food_import",
        json!({ "token": p2["token"], "name": "数据集B" }),
    )
    .unwrap();
    let id2 = r2["source"]["id"].as_str().unwrap().to_string();

    invoke(
        &webview,
        "rename_food_source",
        json!({ "id": id1, "name": "改名后" }),
    )
    .unwrap();
    invoke(
        &webview,
        "reorder_food_sources",
        json!({ "ids": [id2.clone(), id1.clone()] }),
    )
    .unwrap();
    invoke(
        &webview,
        "set_food_source_enabled",
        json!({ "id": id2, "enabled": false }),
    )
    .unwrap();

    let sources = invoke(&webview, "list_food_sources", json!({})).unwrap();
    let by_id = |id: &str| {
        sources
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == json!(id))
            .unwrap()
            .clone()
    };
    assert_eq!(by_id(&id1)["name"], json!("改名后"));
    assert_eq!(by_id(&id2)["enabled"], json!(false));
    assert!(by_id(&id1)["priority"].as_i64().unwrap() > by_id(&id2)["priority"].as_i64().unwrap());

    invoke(&webview, "delete_food_source", json!({ "id": id1 })).unwrap();
    let after_delete = invoke(&webview, "list_food_sources", json!({})).unwrap();
    assert_eq!(after_delete.as_array().unwrap().len(), 3); // manual, id2, llm_estimate
}
