//! Tauri 2 shell — composition root for `CoreApp` and its plugins.
//!
//! Builds `CoreApp` (with `SystemClock`, `KeyringSecretStore`, the diet plugin), starts the
//! scheduler, forwards its `AppEvent`s to the webview, and wires up plan notifications, all in
//! `setup()`, per `docs/PHASE1_SPEC.md` §10.1, plus the tray (`tray.rs`) and logging
//! (`logging.rs`).

// `pub` so the IPC round-trip integration test (`tests/commands.rs`) can reach the command
// functions and `CommandError`, exactly as external code never does in production (Tauri calls
// them by name string through `invoke_handler`) — testing only.
pub mod commands;
pub mod error;
mod events;
mod logging;
mod notify;
mod paths;
mod tray;

use std::sync::Arc;

use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_plugin_diet::DietPlugin;
use tauri::Manager;

pub fn run() {
    // Resolved before anything else so logging can go to `data_dir/logs` from the first line;
    // a failure here is reported again (and made fatal) in `setup()`.
    let data_dir = paths::default_data_dir();
    logging::init(data_dir.as_deref().ok());

    tauri::Builder::default()
        // Registered first, per the plugin's own docs: a second instance should hand off to the
        // running one and exit before any other plugin does its own setup.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch reaches the same window instead of opening a new one.
            tray::focus_main_window(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            let data_dir = data_dir?;
            // `start_scheduler()` calls `tokio::spawn` directly, which needs an active Tokio
            // runtime context to find its reactor — true inside this `block_on`, not true of
            // `setup()`'s own calling thread once it returns. Both go in the same `block_on` so
            // neither runs outside that context. The stored `autostart` setting is read here too,
            // so it can be synced to the OS once we're back on `setup()`'s own (non-async) thread.
            let (core_app, autostart_enabled): (CoreApp, bool) =
                tauri::async_runtime::block_on(async {
                    let core_app = CoreAppBuilder::new(data_dir)
                        .plugin(Arc::new(DietPlugin))
                        .build()
                        .await?;
                    // The `JoinHandle` is discarded: dropping it doesn't stop the spawned task,
                    // which runs for the life of the process by design (§7.13); `CoreApp` itself
                    // stays alive via `app.manage` below.
                    core_app.start_scheduler();
                    let autostart_enabled = core_app.settings().autostart().await?;
                    Ok::<_, lifeagent_core::error::CoreError>((core_app, autostart_enabled))
                })?;

            // Reconciles the OS-level registration with the stored setting on every launch —
            // covers both a first run where autostart was enabled during onboarding/Settings but
            // never yet registered, and drift (e.g. the user removed it via the OS's own startup
            // settings UI outside the app).
            commands::sync_autostart(app.handle(), autostart_enabled);

            events::spawn(app.handle().clone(), &core_app);
            notify::spawn(app.handle().clone(), core_app.clone());
            tray::build(app.handle(), core_app.clone())?;
            app.manage(core_app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::complete_onboarding,
            commands::send_message,
            commands::get_history,
            commands::get_today,
            commands::get_plan,
            commands::generate_plan,
            commands::accept_plan,
            commands::delete_entry,
            commands::get_profile,
            commands::save_profile,
            commands::get_weight_goal,
            commands::save_weight_goal,
            commands::log_weight,
            commands::preview_targets,
            commands::get_settings,
            commands::save_settings,
            commands::get_models,
            commands::save_models,
            commands::set_api_key,
            commands::has_api_key,
            commands::test_model,
            commands::get_memory,
            commands::delete_memory,
            commands::get_usage_summary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running lifeagent-desktop");
}
