/// Every `#[tauri::command]` in `src/commands.rs`, kept here (not derived automatically —
/// `tauri_build` has no way to discover them itself) so `capabilities/default.json` can grant
/// them. `AppManifest::commands` generates an `allow-<command>`/`deny-<command>` permission pair
/// per name; add a new command here whenever one is added to `commands.rs`, or it's registered
/// with `invoke_handler` but unreachable from the webview (denied by the ACL, per §12's
/// principle of least privilege — nothing is implicitly allowed).
const COMMANDS: &[&str] = &[
    "get_app_state",
    "complete_onboarding",
    "send_message",
    "get_history",
    "get_today",
    "get_plan",
    "generate_plan",
    "accept_plan",
    "delete_entry",
    "get_profile",
    "save_profile",
    "get_weight_goal",
    "save_weight_goal",
    "log_weight",
    "preview_targets",
    "get_settings",
    "save_settings",
    "get_models",
    "save_models",
    "set_api_key",
    "has_api_key",
    "test_model",
    "get_memory",
    "delete_memory",
    "get_usage_summary",
    "list_food_sources",
    "list_foods",
    "reorder_food_sources",
    "set_food_source_enabled",
    "rename_food_source",
    "delete_food_source",
    "preview_food_import",
    "commit_food_import",
    "cancel_food_import",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
